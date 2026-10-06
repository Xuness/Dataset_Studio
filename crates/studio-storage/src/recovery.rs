//! Quiescent project packages. Both database writers are held at the same boundary.
use crate::*;
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    version: u32,
    project_id: String,
    created_at: String,
    project_schema: u32,
    evaluation_schema: u32,
    evidence_watermark: u64,
    review_watermark: u64,
    files: Vec<Entry>,
}
fn digest(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path).map_err(Error::io)?;
    let mut bytes = 0;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(Error::io)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        hash.update(&buffer[..n]);
    }
    Ok((bytes, hex::encode(hash.finalize())))
}
fn relative(root: &Path, value: &str) -> Result<PathBuf> {
    if value.contains(':')
        || value.is_empty()
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::invalid("恢复包包含无效路径"));
    }
    Ok(root.join(value))
}
fn regular(root: &Path, path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(Error::io)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::invalid("恢复包不接受重解析点"));
        }
    }
    if metadata.file_type().is_symlink()
        || !path.canonicalize().map_err(Error::io)?.starts_with(root)
    {
        return Err(Error::invalid("恢复包路径超出项目"));
    }
    Ok(())
}
fn copy_tree(root: &Path, dir: &Path, destination: &Path, files: &mut Vec<Entry>) -> Result<()> {
    regular(root, dir)?;
    for entry in fs::read_dir(dir).map_err(Error::io)? {
        let path = entry.map_err(Error::io)?.path();
        regular(root, &path)?;
        if path.is_dir() {
            copy_tree(root, &path, destination, files)?;
        } else if path.is_file() {
            copy_file(root, &path, destination, files)?;
        }
    }
    Ok(())
}
fn copy_file(root: &Path, path: &Path, destination: &Path, files: &mut Vec<Entry>) -> Result<()> {
    if files.len() >= 100000 {
        return Err(Error::invalid("恢复包超过十万个文件"));
    }
    let name = path
        .strip_prefix(root)
        .map_err(Error::io)?
        .to_string_lossy()
        .replace('\\', "/");
    let target = relative(destination, &name)?;
    fs::create_dir_all(target.parent().ok_or_else(|| Error::invalid("路径无效"))?)
        .map_err(Error::io)?;
    fs::copy(path, &target).map_err(Error::io)?;
    OpenOptions::new()
        .write(true)
        .open(&target)
        .map_err(Error::io)?
        .sync_all()
        .map_err(Error::io)?;
    let (bytes, sha256) = digest(&target)?;
    files.push(Entry {
        path: name,
        bytes,
        sha256,
    });
    Ok(())
}
fn backup(source: &Connection, path: &Path) -> Result<()> {
    let mut target = Connection::open(path).map_err(db_error)?;
    rusqlite::backup::Backup::new(source, &mut target)
        .map_err(db_error)?
        .run_to_completion(128, std::time::Duration::from_millis(10), None)
        .map_err(db_error)?;
    target
        .execute_batch("PRAGMA journal_mode=DELETE")
        .map_err(db_error)?;
    check_db(&target)?;
    drop(target);
    OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(Error::io)?
        .sync_all()
        .map_err(Error::io)
}
fn check_db(db: &Connection) -> Result<()> {
    let ok: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(db_error)?;
    if ok != "ok"
        || db
            .prepare("PRAGMA foreign_key_check")
            .map_err(db_error)?
            .exists([])
            .map_err(db_error)?
    {
        return Err(Error::new("BACKUP_INVALID", "恢复数据库完整性校验失败"));
    }
    Ok(())
}
pub(crate) fn build(
    root: PathBuf,
    destination: PathBuf,
    evaluation: &Connection,
    pid: String,
    registry_path: PathBuf,
) -> Result<()> {
    let project = Connection::open_with_flags(
        root.join("project.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(db_error)?;
    if crate::lifecycle::has_background(&project)? || evaluation.query_row("SELECT EXISTS(SELECT 1 FROM stages WHERE state IN ('preparing','running','pausing','cancelling')) OR EXISTS(SELECT 1 FROM analysis_jobs WHERE state IN ('queued','running','cancelling'))",[],|r|r.get::<_,bool>(0)).map_err(db_error)? {
        return Err(Error::new("PROJECT_BUSY","请暂停评审并等待任务和在途结果完成后创建恢复包"));
    }
    fs::create_dir(&destination).map_err(Error::io)?;
    backup(&project, &destination.join("project.sqlite"))?;
    backup(evaluation, &destination.join("evaluation.sqlite"))?;
    if root.join("members.sqlite").exists() {
        let members = Connection::open_with_flags(
            root.join("members.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(db_error)?;
        backup(&members, &destination.join("members.sqlite"))?;
    }
    let mut files = Vec::new();
    for name in ["project.sqlite", "evaluation.sqlite", "members.sqlite"] {
        if !destination.join(name).exists() {
            continue;
        }
        let (bytes, sha256) = digest(&destination.join(name))?;
        files.push(Entry {
            path: name.into(),
            bytes,
            sha256,
        });
    }
    regular(&root, &root.join("project.json"))?;
    copy_file(&root, &root.join("project.json"), &destination, &mut files)?;
    for name in ["artifacts", ".staging"] {
        let dir = root.join(name);
        if dir.exists() {
            copy_tree(&root, &dir, &destination, &mut files)?;
        }
    }
    let registry =
        Connection::open_with_flags(registry_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(db_error)?;
    let mut dependencies = Vec::new();
    for (table, column) in [
        ("llm_providers", "provider_id"),
        ("llm_models", "model_id"),
        ("llm_system_prompts", "system_prompt_id"),
    ] {
        let mut stmt = evaluation
            .prepare(&format!(
                "SELECT DISTINCT json_extract(config,'$.model.{column}') FROM stages LIMIT 4097"
            ))
            .map_err(db_error)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, Option<String>>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        if ids.len() > 4096 {
            return Err(Error::invalid("恢复包的应用配置引用超过上限"));
        }
        for id in ids.into_iter().flatten() {
            let value: Option<String> = registry
                .query_row(
                    &format!("SELECT json FROM {table} WHERE id=?1"),
                    [&id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            let value = value
                .map(|v| serde_json::from_str::<serde_json::Value>(&v).map_err(Error::io))
                .transpose()?
                .map(|mut v| {
                    if let Some(o) = v.as_object_mut() {
                        o.remove("credential_ref");
                    }
                    v
                });
            dependencies
                .push(serde_json::json!({"table":table,"id":id,"current_configuration":value}));
        }
    }
    let mut stmt = project
        .prepare("SELECT id FROM sources")
        .map_err(db_error)?;
    for sid in stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
    {
        let sid = sid.map_err(db_error)?;
        let value: Option<String> = registry
            .query_row(
                "SELECT json FROM source_locations WHERE id=?1",
                [&sid],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        dependencies.push(serde_json::json!({"source_id":sid,"location":value.map(|v|serde_json::from_str::<serde_json::Value>(&v)).transpose().map_err(Error::io)?}));
    }
    atomic_json(&destination.join("dependencies.json"), &dependencies)?;
    let (bytes, sha256) = digest(&destination.join("dependencies.json"))?;
    files.push(Entry {
        path: "dependencies.json".into(),
        bytes,
        sha256,
    });
    let package = Package {
        version: 1,
        project_id: pid,
        created_at: now(),
        project_schema: project
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db_error)?,
        evaluation_schema: evaluation
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db_error)?,
        evidence_watermark: evaluation
            .query_row("SELECT coalesce(max(sequence),0) FROM evidence", [], |r| {
                unsigned(r, 0)
            })
            .map_err(db_error)?,
        review_watermark: evaluation
            .query_row("SELECT coalesce(max(sequence),0) FROM reviews", [], |r| {
                unsigned(r, 0)
            })
            .map_err(db_error)?,
        files,
    };
    atomic_json(&destination.join("recovery.json"), &package)?;
    verify(&destination)?;
    Ok(())
}
fn verify(directory: &Path) -> Result<Package> {
    let directory = directory.canonicalize().map_err(Error::io)?;
    regular(&directory, &directory.join("recovery.json"))?;
    let file = File::open(directory.join("recovery.json")).map_err(Error::io)?;
    if file.metadata().map_err(Error::io)?.len() > 32 << 20 {
        return Err(Error::invalid("恢复清单过大"));
    }
    let package: Package = serde_json::from_reader(file).map_err(Error::io)?;
    let mut seen = std::collections::BTreeSet::new();
    if package.version != 1
        || package.files.len() > 100000
        || package.project_schema > crate::migrations::VERSION
        || package.evaluation_schema > crate::aesthetic::EVALUATION_SCHEMA_VERSION
    {
        return Err(Error::invalid("恢复包版本或文件数不受支持"));
    }
    for entry in &package.files {
        let path = relative(&directory, &entry.path)?;
        regular(&directory, &path)?;
        if !seen.insert(entry.path.clone()) || digest(&path)? != (entry.bytes, entry.sha256.clone())
        {
            return Err(Error::new("BACKUP_INVALID", "恢复文件摘要不一致"));
        }
    }
    for name in ["project.json", "project.sqlite", "evaluation.sqlite"] {
        if !seen.contains(name) {
            return Err(Error::new("BACKUP_INVALID", "恢复包缺少必需文件"));
        }
    }
    let p = Connection::open_with_flags(
        directory.join("project.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(db_error)?;
    let e = Connection::open_with_flags(
        directory.join("evaluation.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(db_error)?;
    check_db(&p)?;
    check_db(&e)?;
    if package.project_schema >= 13 {
        if !seen.contains("members.sqlite") {
            return Err(Error::new("BACKUP_INVALID", "恢复包缺少固定成员数据库"));
        }
        let members = Connection::open_with_flags(
            directory.join("members.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(db_error)?;
        check_db(&members)?;
        let owner: String = members
            .query_row("SELECT project_id FROM owner", [], |r| r.get(0))
            .map_err(db_error)?;
        if owner != package.project_id {
            return Err(Error::new("BACKUP_INVALID", "固定成员数据库身份不一致"));
        }
        let mut results = p
            .prepare(
                "SELECT id,count FROM query_results WHERE storage_kind='sealed' AND status='ready'",
            )
            .map_err(db_error)?;
        for row in results
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_error)?
        {
            let (id, count) = row.map_err(db_error)?;
            let valid:bool=members.query_row("SELECT EXISTS(SELECT 1 FROM datasets WHERE id=?1 AND state='sealed' AND count=?2)",params![id,count],|r|r.get(0)).map_err(db_error)?;
            if !valid {
                return Err(Error::new("BACKUP_INVALID", "固定结果与封存成员记录不一致"));
            }
        }
    }
    let project_manifest: Manifest =
        serde_json::from_reader(File::open(directory.join("project.json")).map_err(Error::io)?)
            .map_err(Error::io)?;
    let project_id = project_manifest.id;
    if p.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
        .map_err(db_error)?
        != package.project_schema
        || e.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(db_error)?
            != package.evaluation_schema
        || e.query_row("SELECT coalesce(max(sequence),0) FROM evidence", [], |r| {
            unsigned(r, 0)
        })
        .map_err(db_error)?
            != package.evidence_watermark
        || e.query_row("SELECT coalesce(max(sequence),0) FROM reviews", [], |r| {
            unsigned(r, 0)
        })
        .map_err(db_error)?
            != package.review_watermark
    {
        return Err(Error::new("BACKUP_INVALID", "恢复包版本或证据水位不一致"));
    }

    if project_id != package.project_id {
        return Err(Error::new("BACKUP_INVALID", "项目身份不一致"));
    }
    let mut refs = p
        .prepare("SELECT r.id FROM evaluation_stage_refs r LEFT JOIN evaluation_creation_intents i ON i.stage_id=r.id WHERE i.state IS NULL OR i.state IN ('materialized','cancelled')")
        .map_err(db_error)?;
    for id in refs
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
    {
        let id = id.map_err(db_error)?;
        if !e
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM stages WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(db_error)?
        {
            return Err(Error::new("BACKUP_INVALID", "两库评审引用缺失"));
        }
    }
    let mut refs = p
        .prepare("SELECT id FROM evaluation_analysis_refs")
        .map_err(db_error)?;
    for id in refs
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
    {
        if !e
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM analysis_jobs WHERE id=?1)",
                [id.map_err(db_error)?],
                |r| r.get::<_, bool>(0),
            )
            .map_err(db_error)?
        {
            return Err(Error::new("BACKUP_INVALID", "两库分析引用缺失"));
        }
    }
    let mut intents = p
        .prepare(
            "SELECT stage_id,config FROM evaluation_creation_intents WHERE state='materialized'",
        )
        .map_err(db_error)?;
    for row in intents
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(db_error)?
    {
        let (id, config) = row.map_err(db_error)?;
        let actual: String = e
            .query_row("SELECT config FROM stages WHERE id=?1", [id], |r| r.get(0))
            .map_err(db_error)?;
        if serde_json::from_str::<serde_json::Value>(&actual).map_err(Error::io)?
            != serde_json::from_str::<serde_json::Value>(&config).map_err(Error::io)?
        {
            return Err(Error::new("BACKUP_INVALID", "两库冻结配置不一致"));
        }
    }
    let entries = package
        .files
        .iter()
        .map(|f| (f.path.replace('\\', "/"), f))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut artifacts = p
        .prepare("SELECT files_json FROM artifacts WHERE status='ready'")
        .map_err(db_error)?;
    for row in artifacts
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
    {
        let files: Vec<studio_domain::ArtifactFile> =
            serde_json::from_str(&row.map_err(db_error)?).map_err(Error::io)?;
        for file in files {
            let entry = entries
                .get(&file.path.replace('\\', "/"))
                .ok_or_else(|| Error::new("BACKUP_INVALID", "恢复包缺少成果文件"))?;
            if file.bytes.is_some_and(|v| v != entry.bytes)
                || file.sha256.as_ref().is_some_and(|v| v != &entry.sha256)
            {
                return Err(Error::new("BACKUP_INVALID", "成果摘要与恢复文件不一致"));
            }
        }
    }
    Ok(package)
}
impl SqliteStore {
    pub fn recovery_package(&self, pid: &str) -> Result<String> {
        let p = self.handle(pid)?;
        let evaluation = self.evaluation(pid)?;
        let _guard = p.db.lock().map_err(lock_error)?;
        let root = p.project.directory.clone();
        let parent = root.join(".backups");
        fs::create_dir_all(&parent).map_err(Error::io)?;
        regular(&root, &parent)?;
        let name = format!(".backups/project-{}-{}", now(), new_id());
        evaluation.project_package(
            root.clone(),
            root.join(&name),
            pid.to_owned(),
            self.root.join("registry.sqlite"),
        )?;
        Ok(name)
    }
    /// Restores to an unused directory. Registration/opening is an explicit separate operation.
    pub fn restore_package(&self, package: &Path, destination: &Path) -> Result<()> {
        let package = package.canonicalize().map_err(Error::io)?;
        let manifest = verify(&package)?;
        if self
            .projects
            .lock()
            .map_err(lock_error)?
            .contains_key(&manifest.project_id)
        {
            return Err(Error::new("PROJECT_BUSY", "请先关闭同身份项目再恢复"));
        }
        if destination.exists() {
            return Err(Error::invalid("恢复目标必须为新目录"));
        }
        fs::create_dir(destination).map_err(Error::io)?;
        let destination = destination.canonicalize().map_err(Error::io)?;
        for entry in manifest.files.iter().filter(|e| e.path != "project.json") {
            let from = relative(&package, &entry.path)?;
            let target = relative(&destination, &entry.path)?;
            fs::create_dir_all(target.parent().ok_or_else(|| Error::invalid("路径无效"))?)
                .map_err(Error::io)?;
            fs::copy(from, &target).map_err(Error::io)?;
            if digest(&target)? != (entry.bytes, entry.sha256.clone()) {
                return Err(Error::new("BACKUP_INVALID", "恢复复制校验失败"));
            }
            OpenOptions::new()
                .write(true)
                .open(target)
                .map_err(Error::io)?
                .sync_all()
                .map_err(Error::io)?;
        }
        fs::create_dir_all(destination.join("artifacts")).map_err(Error::io)?;
        fs::create_dir_all(destination.join(".staging")).map_err(Error::io)?;
        // Publish an openable project only after every payload is verified.
        let temporary = destination.join(".project.restore");
        fs::copy(package.join("project.json"), &temporary).map_err(Error::io)?;
        let expected = manifest
            .files
            .iter()
            .find(|e| e.path == "project.json")
            .ok_or_else(|| Error::new("BACKUP_INVALID", "缺少项目清单"))?;
        if digest(&temporary)? != (expected.bytes, expected.sha256.clone()) {
            return Err(Error::new("BACKUP_INVALID", "项目清单摘要不一致"));
        }
        OpenOptions::new()
            .write(true)
            .open(&temporary)
            .map_err(Error::io)?
            .sync_all()
            .map_err(Error::io)?;
        fs::rename(temporary, destination.join("project.json")).map_err(Error::io)?;
        Ok(())
    }
}
