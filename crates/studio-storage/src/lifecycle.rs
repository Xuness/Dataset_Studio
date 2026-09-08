use crate::*;

/// Keeps the connection and file lease owned while an HTTP request is in flight.
pub struct ProjectLease {
    _project: Arc<ProjectDb>,
}

pub(super) fn has_background(db: &Connection) -> Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','preparing','running','waiting_input')) OR EXISTS(SELECT 1 FROM query_results WHERE status IN ('queued','running'))", [], |r| r.get(0)).map_err(db_error)
}

impl SqliteStore {
    pub fn operation_lease(&self, id: &str) -> Result<ProjectLease> {
        Ok(ProjectLease {
            _project: self.handle(id)?,
        })
    }
    pub fn request_lease(&self, id: &str) -> Result<ProjectLease> {
        let p = self.handle(id)?;
        if !p.view_open.load(Ordering::Acquire) {
            return Err(Error::new(
                "PROJECT_CLOSED",
                "项目视图已关闭；请重新打开项目",
            ));
        }
        Ok(ProjectLease { _project: p })
    }
    pub fn view_is_open(&self, id: &str) -> bool {
        self.projects
            .lock()
            .ok()
            .and_then(|p| p.get(id).map(|p| p.view_open.load(Ordering::Acquire)))
            .unwrap_or(false)
    }
    pub(super) fn registered_directory(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        self.registry
            .lock()
            .map_err(lock_error)?
            .query_row("SELECT directory FROM projects WHERE id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(db_error)?
            .map(PathBuf::from)
            .ok_or_else(|| Error::new("NOT_FOUND", "最近项目不存在"))
    }
    pub(super) fn open_tracked(&self, directory: PathBuf, view: bool) -> Result<Project> {
        let result = self.acquire_project(directory.clone(), view);
        if let Err(error) = &result {
            let canonical = directory.canonicalize().unwrap_or(directory);
            let _ = self.registry.lock().map_err(lock_error)?.execute(
                "UPDATE projects SET issue=?2 WHERE directory=?1",
                params![canonical.to_string_lossy(), error.to_string()],
            );
        }
        result
    }
    pub(super) fn recent_projects(&self) -> Result<Vec<ProjectSummary>> {
        // Release the registry mutex before inspecting the handle map.
        let rows = {
            let db = self.registry.lock().map_err(lock_error)?;
            let mut stmt = db.prepare("SELECT id,directory,opened_at,summary,issue FROM projects ORDER BY opened_at DESC,id").map_err(db_error)?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?
        };
        let projects = self.projects.lock().map_err(lock_error)?;
        rows.into_iter()
            .map(|(id, directory, opened_at, summary, issue)| {
                let saved = summary.and_then(|s| serde_json::from_str::<Project>(&s).ok());
                let state = match projects.get(&id) {
                    Some(p) if p.view_open.load(Ordering::Acquire) => ProjectState::Open,
                    Some(p) if p.background.load(Ordering::Acquire) => ProjectState::Background,
                    Some(_) => ProjectState::Draining,
                    None if issue.is_some() => ProjectState::Unavailable,
                    None => ProjectState::Closed,
                };
                Ok(ProjectSummary {
                    id,
                    name: saved
                        .map(|p| p.name)
                        .unwrap_or_else(|| "尚未打开的项目".into()),
                    directory: directory.into(),
                    opened_at,
                    state,
                    issue,
                })
            })
            .collect()
    }
    pub(super) fn close_project(&self, id: &str) -> Result<ProjectClose> {
        validate_id(id)?;
        {
            let projects = self.projects.lock().map_err(lock_error)?;
            if let Some(p) = projects.get(id) {
                p.view_open.store(false, Ordering::Release);
            }
        }
        self.reap_closed()?;
        let projects = self.projects.lock().map_err(lock_error)?;
        let state = if let Some(p) = projects.get(id) {
            if p.background.load(Ordering::Acquire) {
                ProjectState::Background
            } else {
                ProjectState::Draining
            }
        } else {
            ProjectState::Closed
        };
        Ok(ProjectClose {
            project_id: id.into(),
            state,
        })
    }
    /// Only already-owned projects are scheduled. This function never opens a DB.
    pub fn owned_projects(&self) -> Result<Vec<String>> {
        let mut ids = self
            .projects
            .lock()
            .map_err(lock_error)?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }
    pub fn reap_closed(&self) -> Result<()> {
        let mut projects = self.projects.lock().map_err(lock_error)?;
        let mut release = Vec::new();
        for (id, p) in projects.iter() {
            // Other Arc owners are in-flight requests or operations. They retain the lease.
            if !p.view_open.load(Ordering::Acquire) && Arc::strong_count(p) == 1 {
                let pending =
                    p.db.lock()
                        .map_err(lock_error)
                        .and_then(|db| has_background(&db));
                match pending {
                    Ok(pending) => {
                        p.background.store(pending, Ordering::Release);
                        if !pending {
                            self.registry
                                .lock()
                                .map_err(lock_error)?
                                .execute(
                                    "UPDATE projects SET background_pending=0 WHERE id=?1",
                                    [id],
                                )
                                .map_err(db_error)?;
                            release.push(id.clone());
                        }
                    }
                    Err(error) => {
                        self.registry
                            .lock()
                            .map_err(lock_error)?
                            .execute(
                                "UPDATE projects SET issue=?2 WHERE id=?1",
                                params![id, error.to_string()],
                            )
                            .map_err(db_error)?;
                    }
                }
            }
        }
        for id in release {
            projects.remove(&id);
        }
        Ok(())
    }
    pub(super) fn mark_background(&self, id: &str) -> Result<()> {
        self.handle(id)?.background.store(true, Ordering::Release);
        // Conservative write BEFORE enqueue: a crash can cause an extra recovery probe,
        // never an unregistered pending job. Completion is reconciled during recovery/reap.
        self.registry
            .lock()
            .map_err(lock_error)?
            .execute("UPDATE projects SET background_pending=1 WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(())
    }
    /// One startup pass, isolated per project. Legacy registry entries are probed read-only.
    pub fn recover_jobs(&self) -> Result<Vec<(String, String)>> {
        let ids = {
            let registry = self.registry.lock().map_err(lock_error)?;
            let mut stmt = registry
                .prepare(
                    "SELECT id FROM projects WHERE background_pending=1 ORDER BY opened_at DESC",
                )
                .map_err(db_error)?;
            stmt.query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        let mut issues = Vec::new();
        for id in ids {
            let recover = (|| -> Result<()> {
                let directory = self.registered_directory(&id)?;
                let dbpath = directory.join("project.sqlite");
                migrations::check_supported(&dbpath)?;
                let db =
                    Connection::open_with_flags(dbpath, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                        .map_err(db_error)?;
                let jobs: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','preparing','running','waiting_input'))", [], |r| r.get(0)).map_err(db_error)?;
                let version: u32 = db
                    .query_row("PRAGMA user_version", [], |r| r.get(0))
                    .map_err(db_error)?;
                let queries: bool = version >= 3 && db.query_row("SELECT EXISTS(SELECT 1 FROM query_results WHERE status IN ('queued','running'))", [], |r| r.get(0)).map_err(db_error)?;
                drop(db);
                if jobs || queries {
                    self.open_tracked(directory, false)?;
                } else {
                    self.registry
                        .lock()
                        .map_err(lock_error)?
                        .execute(
                            "UPDATE projects SET background_pending=0 WHERE id=?1",
                            [&id],
                        )
                        .map_err(db_error)?;
                }
                Ok(())
            })();
            if let Err(error) = recover {
                let issue = error.to_string();
                self.registry
                    .lock()
                    .map_err(lock_error)?
                    .execute(
                        "UPDATE projects SET issue=?2 WHERE id=?1",
                        params![id, issue],
                    )
                    .map_err(db_error)?;
                issues.push((id, issue));
            }
        }
        self.reap_closed()?;
        Ok(issues)
    }
}

impl SqliteStore {
    fn acquire_project(&self, directory: PathBuf, view_open: bool) -> Result<Project> {
        let directory = directory.canonicalize().map_err(Error::io)?;
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(directory.join("project.json")).map_err(Error::io)?)
                .map_err(|_| Error::invalid("不是有效的 Studio 项目"))?;
        validate_id(&manifest.id)?;
        if manifest.format_version != 1 {
            return Err(Error::new("FORMAT_UNSUPPORTED", "项目格式版本不兼容"));
        }
        let mut projects = self.projects.lock().map_err(lock_error)?;
        if let Some(p) = projects.get(&manifest.id) {
            if p.project.directory != directory {
                return Err(Error::new(
                    "PROJECT_ID_CONFLICT",
                    "相同项目身份已在另一个目录打开",
                ));
            }
            if view_open {
                p.view_open.store(true, Ordering::Release);
            }
            let revision =
                p.db.lock()
                    .map_err(lock_error)?
                    .query_row(
                        "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'",
                        [],
                        |r| unsigned(r, 0),
                    )
                    .map_err(db_error)?;
            return Ok(Project {
                revision,
                ..p.project.clone()
            });
        }
        let dbpath = directory
            .join("project.sqlite")
            .canonicalize()
            .map_err(Error::io)?;
        if dbpath.parent() != Some(directory.as_path()) {
            return Err(Error::invalid("项目数据库必须位于项目目录内"));
        }
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".project.lock"))
            .map_err(Error::io)?;
        lease
            .try_lock_exclusive()
            .map_err(|_| Error::new("PROJECT_BUSY", "项目已由其他引擎打开"))?;
        // Reject future formats before opening a writable connection or changing journal mode.
        migrations::check_supported(&dbpath)?;
        let mut db =
            Connection::open_with_flags(&dbpath, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(db_error)?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(db_error)?;
        db.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(db_error)?;
        migrations::upgrade(&mut db, &directory)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-65536; PRAGMA journal_size_limit=33554432;")
            .map_err(db_error)?;
        // A newly acquired project lease means no previous engine still owns its tasks.
        // Cached handles return above, so opening an already active project does not interrupt it.
        {
            let tx = db.transaction().map_err(db_error)?;
            // Incomplete query members are never published. Rebuild is explicit after a crash.
            tx.execute("UPDATE query_results SET status='interrupted',count=NULL,error='构建被中断，请重新计算' WHERE status='running'", []).map_err(db_error)?;
            tx.execute("DELETE FROM result_references WHERE owner_kind='query_input' AND owner_id IN (SELECT id FROM query_results WHERE status NOT IN ('queued','running'))", []).map_err(db_error)?;
            crate::query_cache::recover(&tx)?;
            let interrupted = {
                let mut stmt = tx
                    .prepare("SELECT id FROM jobs WHERE status IN ('running','preparing')")
                    .map_err(db_error)?;
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_error)?
            };
            for id in interrupted {
                tx.execute(
                    "UPDATE jobs SET status='queued',error='恢复中断的执行' WHERE id=?1",
                    [&id],
                )
                .map_err(db_error)?;
                event(&tx, "job.recovered", &id)?;
            }
            tx.commit().map_err(db_error)?;
        }
        let revision: u64 = db
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='revision'",
                [],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        let project = Project {
            id: manifest.id,
            name: manifest.name,
            directory,
            created_at: manifest.created_at,
            revision,
        };
        let background = has_background(&db)?;
        self.registry.lock().map_err(lock_error)?.execute("INSERT INTO projects(id,directory,opened_at,summary,background_pending) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET directory=excluded.directory,opened_at=excluded.opened_at,summary=excluded.summary,background_pending=excluded.background_pending,issue=NULL",params![project.id,project.directory.to_string_lossy(),now(),serde_json::to_string(&project).map_err(Error::io)?,background]).map_err(db_error)?;
        projects.insert(
            project.id.clone(),
            Arc::new(ProjectDb {
                db: Mutex::new(db),
                _lease: lease,
                project: project.clone(),
                view_open: AtomicBool::new(view_open),
                background: AtomicBool::new(background),
            }),
        );
        Ok(project)
    }
}
