use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::ProjectRepository;
use studio_domain::*;
mod artifacts;
mod browse_scopes;
mod derived_fields;
mod drafts;
pub use derived_fields::{artifact_field_id, native_spec};
mod job_scopes;
mod job_telemetry;
mod lifecycle;
mod migrations;
mod query;
mod query_cache;
pub use query_cache::{
    QueryCacheEntry, QueryCachePolicy, QueryCacheRequest, QueryCacheStats, QueryStage,
};
mod ranking;
pub mod ranking_tables;
mod registry;
mod scopes;
mod selection;
mod source_locations;
pub use lifecycle::ProjectLease;

fn unsigned(row: &rusqlite::Row, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn db_error(error: rusqlite::Error) -> Error {
    Error::new("DATABASE_ERROR", error.to_string())
}
fn lock_error<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::new("INTERNAL_ERROR", "项目状态锁不可用")
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::invalid("无效的文件位置"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(Error::io)?;
    serde_json::to_writer_pretty(&mut file, value).map_err(Error::io)?;
    file.write_all(b"\n").map_err(Error::io)?;
    file.as_file().sync_all().map_err(Error::io)?;
    file.persist(path).map_err(Error::io)?;
    Ok(())
}
pub fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}
fn connection(path: &Path) -> Result<Connection> {
    let db = Connection::open(path).map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(db_error)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA cache_size=-65536; PRAGMA journal_size_limit=33554432;")
        .map_err(db_error)?;
    Ok(db)
}
fn event(db: &Connection, kind: &str, resource: &str) -> Result<()> {
    db.execute(
        "INSERT INTO events(kind,resource_id) VALUES (?1,?2)",
        params![kind, resource],
    )
    .map_err(db_error)?;
    db.execute(
        "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='revision'",
        [],
    )
    .map_err(db_error)?;
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    format_version: u32,
    id: String,
    name: String,
    created_at: String,
}
struct ProjectDb {
    db: Mutex<Connection>,
    _lease: File,
    project: Project,
    view_open: AtomicBool,
    background: AtomicBool,
}
pub struct SqliteStore {
    root: PathBuf,
    registry: Mutex<Connection>,
    projects: Mutex<HashMap<String, Arc<ProjectDb>>>,
    query_sizes: Mutex<HashMap<String, (u64, u64)>>,
}
impl SqliteStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root).map_err(Error::io)?;
        let root = root.canonicalize().map_err(Error::io)?;
        registry::check(&root.join("registry.sqlite"))?;
        let mut db = connection(&root.join("registry.sqlite"))?;
        registry::initialize(&mut db, &root)?;
        Ok(Self {
            root,
            registry: Mutex::new(db),
            projects: Mutex::new(HashMap::new()),
            query_sizes: Mutex::new(HashMap::new()),
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn handle(&self, id: &str) -> Result<Arc<ProjectDb>> {
        validate_id(id)?;
        if let Some(p) = self.projects.lock().map_err(lock_error)?.get(id) {
            return Ok(p.clone());
        }
        Err(Error::new("PROJECT_CLOSED", "请先显式打开项目"))
    }
    pub fn directory(&self, id: &str) -> Result<PathBuf> {
        Ok(self.handle(id)?.project.directory.clone())
    }
    pub fn source(&self, project_id: &str, source_id: &str) -> Result<Source> {
        self.sources(project_id)?
            .into_iter()
            .find(|s| s.id == source_id)
            .ok_or_else(|| Error::new("NOT_FOUND", "该数据源未加入当前项目"))
    }
    pub fn contains(&self, project_id: &str, keys: &[AssetKey]) -> Result<Vec<bool>> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        keys.iter()
            .map(|key| selection::contains(&db, key))
            .collect()
    }
    fn keys(
        &self,
        project_id: &str,
        owner: Option<(&str, &str)>,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<Vec<AssetKey>> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        let (source, asset) = after
            .map(|a| (a.source_id.as_str(), a.asset_id.as_str()))
            .unwrap_or(("", ""));
        let (sql, id) = match owner {
            Some(("collection", id)) => (
                "SELECT source_id,asset_id FROM collection_members WHERE collection_id=?1 AND (source_id,asset_id)>(?2,?3) ORDER BY source_id,asset_id LIMIT ?4",
                id,
            ),
            Some(("job", id)) => (
                "SELECT source_id,asset_id FROM job_inputs WHERE job_id=?1 AND (source_id,asset_id)>(?2,?3) ORDER BY source_id,asset_id LIMIT ?4",
                id,
            ),
            _ => return selection::keys(&db, after, limit),
        };
        let mut stmt = db.prepare(sql).map_err(db_error)?;
        stmt.query_map(
            params![id, source, asset, limit.clamp(1, 1000) as u32],
            |r| {
                Ok(AssetKey {
                    source_id: r.get(0)?,
                    asset_id: r.get(1)?,
                })
            },
        )
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    pub fn job_inputs(
        &self,
        project_id: &str,
        id: &str,
        after: Option<&AssetKey>,
    ) -> Result<Vec<AssetKey>> {
        self.keys(project_id, Some(("job", id)), after, 256)
    }
    pub fn submit_job(
        &self,
        project_id: &str,
        key: &str,
        expected_selection: u64,
        delay_ms: u64,
    ) -> Result<Job> {
        validate_id(key)?;
        let p = self.handle(project_id)?;
        self.mark_background(project_id)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let request = format!("manifest-v1:{expected_selection}:{delay_ms}");
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT id,request_hash FROM jobs WHERE idempotency_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if let Some((id, old)) = previous {
            if old != request {
                return Err(Error::new("IDEMPOTENCY_CONFLICT", "幂等键已被不同请求使用"));
            }
            return read_job(&tx, project_id, &id);
        }
        let revision: u64 = tx
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_revision'",
                [],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        if revision != expected_selection {
            return Err(Error::new("REVISION_CONFLICT", "选择已变化，请刷新后提交"));
        }
        let total: u64 = tx
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_count'",
                [],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        if total == 0 {
            return Err(Error::invalid("请先选择任务输入"));
        }
        let id = new_id();
        tx.execute("INSERT INTO jobs(id,operator,status,total,created_at,idempotency_key,request_hash,delay_ms) VALUES (?1,'core.manifest','queued',?2,?3,?4,?5,?6)",params![id,total as i64,now(),key,request,delay_ms.min(1000) as i64]).map_err(db_error)?;
        tx.execute(
            &format!(
                "INSERT INTO job_inputs SELECT ?1,source_id,asset_id FROM ({})",
                selection::MEMBERS
            ),
            [&id],
        )
        .map_err(db_error)?;
        let scope = ScopeRef {
            project_id: project_id.into(),
            target: ScopeTarget::Selection {
                revision: expected_selection,
            },
        };
        let resolved = scopes::resolve(&tx, project_id, &scope)?;
        scopes::references(&tx, "job", &id, &resolved.results)?;
        tx.execute(
            "INSERT INTO job_scopes VALUES (?1,?2,?3,NULL)",
            params![
                id,
                serde_json::to_string(&scope).map_err(Error::io)?,
                resolved.provenance.to_string()
            ],
        )
        .map_err(db_error)?;
        event(&tx, "job.created", &id)?;
        let job = read_job(&tx, project_id, &id)?;
        tx.commit().map_err(db_error)?;
        Ok(job)
    }
    pub fn jobs(&self, project_id: &str) -> Result<Vec<Job>> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        let mut stmt = db
            .prepare("SELECT id FROM jobs ORDER BY created_at DESC LIMIT 200")
            .map_err(db_error)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| read_job(&db, project_id, id)).collect()
    }
    pub fn job(&self, project_id: &str, id: &str) -> Result<Job> {
        validate_id(id)?;
        let p = self.handle(project_id)?;
        read_job(&*p.db.lock().map_err(lock_error)?, project_id, id)
    }
    pub fn job_delay(&self, project_id: &str, id: &str) -> Result<u64> {
        let p = self.handle(project_id)?;
        p.db.lock()
            .map_err(lock_error)?
            .query_row("SELECT delay_ms FROM jobs WHERE id=?1", [id], |r| {
                unsigned(r, 0)
            })
            .map_err(db_error)
    }
    pub fn update_job(
        &self,
        project_id: &str,
        id: &str,
        status: &str,
        completed: u64,
        error: Option<&str>,
        artifact: Option<&str>,
    ) -> Result<Job> {
        let p = self.handle(project_id)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let old = read_job(&tx, project_id, id)?;
        if ["succeeded", "cancelled", "failed"].contains(&old.status.as_str()) {
            return Ok(old);
        }
        tx.execute("UPDATE jobs SET status=?2,completed=?3,error=?4,artifact=?5,attempt=attempt+CASE WHEN ?2='running' AND status!='running' THEN 1 ELSE 0 END WHERE id=?1",params![id,status,completed as i64,error,artifact]).map_err(db_error)?;
        if ["succeeded", "failed", "cancelled"].contains(&status) {
            job_telemetry::finish(&tx, id)?;
        }
        event(&tx, "job.changed", id)?;
        let job = read_job(&tx, project_id, id)?;
        tx.commit().map_err(db_error)?;
        Ok(job)
    }
    pub fn scheduled_jobs(&self, project_id: &str, recovery: bool) -> Result<Vec<Job>> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        let sql = if recovery {
            "SELECT id FROM jobs WHERE status IN ('running','preparing') ORDER BY created_at"
        } else {
            "SELECT id FROM jobs WHERE status='queued' ORDER BY created_at LIMIT 1"
        };
        let mut stmt = db.prepare(sql).map_err(db_error)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| read_job(&db, project_id, id)).collect()
    }
    pub fn latest_event(&self, project_id: &str) -> Result<u64> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        db.query_row("SELECT COALESCE(MAX(sequence),0) FROM events", [], |r| {
            unsigned(r, 0)
        })
        .map_err(db_error)
    }
}
fn read_job(db: &Connection, project_id: &str, id: &str) -> Result<Job> {
    validate_id(id)?;
    let (mut job,scope)=db.query_row(
        "SELECT j.id,j.operator,j.status,j.total,j.completed,j.attempt,j.created_at,j.error,j.artifact,s.scope_json FROM jobs j LEFT JOIN job_scopes s ON s.job_id=j.id WHERE j.id=?1",
        [id],
        |row| {
            let total=unsigned(row,3)?;
            Ok((Job {
                id:row.get(0)?,project_id:project_id.to_owned(),operator:row.get(1)?,
                status:row.get(2)?,total,completed:unsigned(row,4)?,attempt:row.get(5)?,
                created_at:row.get(6)?,error:row.get(7)?,artifact:row.get(8)?,input_scope:None,
                input_members_frozen:total>0,
                stage:None,
            },row.get::<_,Option<String>>(9)?))
        },
    ).optional().map_err(db_error)?.ok_or_else(||Error::new("NOT_FOUND","任务不存在"))?;
    job.input_scope = scope
        .map(|s| serde_json::from_str(&s).map_err(Error::io))
        .transpose()?;
    job.stage = db
        .query_row(
            "SELECT stage_json FROM job_progress WHERE job_id=?1",
            [id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
        .map(|v| serde_json::from_str(&v).map_err(Error::io))
        .transpose()?;
    Ok(job)
}
impl ProjectRepository for SqliteStore {
    fn create(&self, name: &str, parent: Option<PathBuf>) -> Result<Project> {
        let name = validate_name(name)?;
        let parent = parent.unwrap_or_else(|| self.root.join("projects"));
        fs::create_dir_all(&parent).map_err(Error::io)?;
        let id = new_id();
        let directory = parent.join(&id);
        fs::create_dir(&directory).map_err(Error::io)?;
        for child in ["artifacts", ".staging"] {
            fs::create_dir(directory.join(child)).map_err(Error::io)?;
        }
        let manifest = Manifest {
            format_version: 1,
            id,
            name,
            created_at: now(),
        };
        let mut db = connection(&directory.join("project.sqlite"))?;
        migrations::initialize(&mut db)?;
        drop(db);
        atomic_json(&directory.join("project.json"), &manifest)?;
        self.open(directory)
    }
    fn open(&self, directory: PathBuf) -> Result<Project> {
        self.open_tracked(directory, true)
    }
    fn list(&self) -> Result<Vec<ProjectSummary>> {
        self.recent_projects()
    }
    fn open_recent(&self, id: &str) -> Result<Project> {
        self.open_tracked(self.registered_directory(id)?, true)
    }
    fn close(&self, id: &str) -> Result<ProjectClose> {
        self.close_project(id)
    }
    fn project(&self, id: &str) -> Result<Project> {
        let p = self.handle(id)?;
        let revision =
            p.db.lock()
                .map_err(lock_error)?
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'",
                    [],
                    |r| unsigned(r, 0),
                )
                .map_err(db_error)?;
        Ok(Project {
            revision,
            ..p.project.clone()
        })
    }
    fn attach(&self, project_id: &str, source: Source) -> Result<()> {
        let p = self.handle(project_id)?;
        self.attach_location(&source)?;
        let reference = Source {
            index_root: None,
            media_root: None,
            ..source
        };
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        tx.execute(
            "INSERT INTO sources VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            params![
                reference.id,
                serde_json::to_string(&reference).map_err(Error::io)?
            ],
        )
        .map_err(db_error)?;
        event(&tx, "source.attached", &reference.id)?;
        tx.commit().map_err(db_error)
    }
    fn sources(&self, project_id: &str) -> Result<Vec<Source>> {
        let p = self.handle(project_id)?;
        let rows = {
            let db = p.db.lock().map_err(lock_error)?;
            let mut stmt = db
                .prepare("SELECT json FROM sources ORDER BY id")
                .map_err(db_error)?;
            stmt.query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        let registry = self.registry.lock().map_err(lock_error)?;
        rows.into_iter()
            .map(|json| {
                let reference: Source = serde_json::from_str(&json).map_err(Error::io)?;
                let location: Option<String> = registry
                    .query_row(
                        "SELECT json FROM source_locations WHERE id=?1",
                        [&reference.id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(db_error)?;
                if let Some(json) = location {
                    let source: Source = serde_json::from_str(&json).map_err(Error::io)?;
                    Ok(Source {
                        name: reference.name,
                        ..source
                    })
                } else {
                    Ok(Source {
                        index_root: None,
                        media_root: None,
                        ..reference
                    })
                }
            })
            .collect()
    }
    fn selection(&self, project_id: &str) -> Result<Selection> {
        let p = self.handle(project_id)?;
        selection::read(&*p.db.lock().map_err(lock_error)?)
    }
    fn selection_keys(
        &self,
        project_id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<Vec<AssetKey>> {
        self.keys(project_id, None, after, limit)
    }
    fn change_selection(
        &self,
        project_id: &str,
        expected_revision: u64,
        add: &[AssetKey],
        remove: &[AssetKey],
        clear: bool,
    ) -> Result<Selection> {
        selection::change(self, project_id, expected_revision, add, remove, clear)
    }
    fn collections(&self, project_id: &str) -> Result<Vec<Collection>> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        let mut stmt = db
            .prepare("SELECT id,name,count FROM collections ORDER BY rowid")
            .map_err(db_error)?;
        stmt.query_map([], |r| {
            Ok(Collection {
                id: r.get(0)?,
                name: r.get(1)?,
                count: unsigned(r, 2)?,
            })
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
    fn save_collection(&self, project_id: &str, name: &str) -> Result<Collection> {
        use studio_application::ScopeRepository;
        let current = self.selection(project_id)?;
        self.save_scope_collection(
            project_id,
            name,
            &ScopeRef {
                project_id: project_id.into(),
                target: ScopeTarget::Selection {
                    revision: current.revision,
                },
            },
        )
    }
    fn collection_keys(
        &self,
        project_id: &str,
        id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<Vec<AssetKey>> {
        self.keys(project_id, Some(("collection", id)), after, limit)
    }
    fn events(&self, project_id: &str, after: u64) -> Result<Vec<ProjectEvent>> {
        let p = self.handle(project_id)?;
        let db = p.db.lock().map_err(lock_error)?;
        let mut stmt=db.prepare("SELECT sequence,kind,resource_id FROM events WHERE sequence>?1 ORDER BY sequence LIMIT 64").map_err(db_error)?;
        stmt.query_map([after.min(i64::MAX as u64) as i64], |r| {
            Ok(ProjectEvent {
                sequence: unsigned(r, 0)?,
                project_id: project_id.to_owned(),
                kind: r.get(1)?,
                resource_id: r.get(2)?,
            })
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }
}
