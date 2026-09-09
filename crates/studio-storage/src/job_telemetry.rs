use crate::*;

fn read(db: &Connection, jid: &str) -> Result<Option<JobStage>> {
    let value: Option<String> = db
        .query_row(
            "SELECT stage_json FROM job_progress WHERE job_id=?1",
            [jid],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    value
        .map(|v| serde_json::from_str(&v).map_err(Error::io))
        .transpose()
}
fn write(db: &Connection, jid: &str, stage: &JobStage) -> Result<()> {
    db.execute("INSERT INTO job_progress VALUES (?1,?2) ON CONFLICT(job_id) DO UPDATE SET stage_json=excluded.stage_json", params![jid,serde_json::to_string(stage).map_err(Error::io)?]).map_err(db_error)?;
    Ok(())
}
fn advance(stage: &mut JobStage, at: &str) {
    if let Some(t) = &mut stage.telemetry {
        if t.finished_at.is_some() {
            return;
        }
        let delta = at
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_sub(t.updated_at.parse::<u64>().unwrap_or(0));
        if let Some(phase) = t
            .phases
            .iter_mut()
            .find(|p| p.name == stage.name && p.rating == stage.rating)
        {
            phase.elapsed_ms = phase.elapsed_ms.saturating_add(delta);
        } else if t.phases.len() < 64 {
            t.phases.push(JobPhaseTiming {
                name: stage.name.clone(),
                rating: stage.rating.clone(),
                elapsed_ms: delta,
            });
        }
        t.updated_at = at.into();
    }
}
pub(super) fn record(db: &Connection, jid: &str, incoming: &JobStage) -> Result<()> {
    if incoming.name.len() > 96
        || incoming
            .rating
            .as_ref()
            .is_some_and(|v| !["g", "s", "q", "e"].contains(&v.as_str()))
    {
        return Err(Error::new("WORKER_PROTOCOL_ERROR", "任务阶段无效"));
    }
    let status: String = db
        .query_row("SELECT status FROM jobs WHERE id=?1", [jid], |r| r.get(0))
        .map_err(db_error)?;
    let terminal = ["succeeded", "failed", "cancelled"].contains(&status.as_str());
    if terminal && !(status == "succeeded" && incoming.name == "complete") {
        return Ok(());
    }
    let at = now();
    let mut previous = read(db, jid)?;
    if let Some(old) = &mut previous {
        advance(old, &at);
    }
    let mut telemetry = previous
        .and_then(|s| s.telemetry)
        .unwrap_or_else(|| JobTelemetry {
            started_at: at.clone(),
            updated_at: at.clone(),
            heartbeat_at: at.clone(),
            finished_at: None,
            phases: Vec::new(),
        });
    telemetry.updated_at = at.clone();
    telemetry.heartbeat_at = at.clone();
    if terminal && telemetry.finished_at.is_none() {
        telemetry.finished_at = Some(at);
    }
    let mut stage = incoming.clone();
    // Worker clocks/history are not authoritative: the owning engine stamps them.
    stage.telemetry = Some(telemetry);
    write(db, jid, &stage)
}
pub(super) fn finish(db: &Connection, jid: &str) -> Result<()> {
    if let Some(mut stage) = read(db, jid)? {
        let at = now();
        advance(&mut stage, &at);
        if let Some(t) = &mut stage.telemetry {
            if t.finished_at.is_none() {
                t.finished_at = Some(at.clone());
            }
            t.heartbeat_at = at;
        }
        write(db, jid, &stage)?;
    }
    Ok(())
}
pub(super) fn waiting(db: &Connection, jid: &str, processed: u64) -> Result<()> {
    let Some(stage) = read(db, jid)? else {
        return Ok(());
    };
    let Some(t) = stage.telemetry else {
        return Ok(());
    };
    if now()
        .parse::<u64>()
        .unwrap_or(0)
        .saturating_sub(t.heartbeat_at.parse::<u64>().unwrap_or(0))
        < 2000
    {
        return Ok(());
    }
    record(
        db,
        jid,
        &JobStage {
            name: "waiting_input".into(),
            completed: processed,
            ..Default::default()
        },
    )
}
impl SqliteStore {
    pub fn job_heartbeat(&self, pid: &str, jid: &str) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        if let Some(mut stage) = read(&db, jid)?
            && let Some(t) = &mut stage.telemetry
        {
            if t.finished_at.is_some() {
                return Ok(());
            }
            t.heartbeat_at = now();
            write(&db, jid, &stage)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_progress_and_terminal_observations_remain_consistent() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE jobs(id TEXT PRIMARY KEY,status TEXT); CREATE TABLE job_progress(job_id TEXT PRIMARY KEY,stage_json TEXT); INSERT INTO jobs VALUES ('job','running');").unwrap();
        let legacy: JobStage =
            serde_json::from_str(r#"{"name":"scope_basis","completed":1,"total":2}"#).unwrap();
        assert!(legacy.telemetry.is_none());
        record(&db, "job", &legacy).unwrap();
        let mut stage = read(&db, "job").unwrap().unwrap();
        let began = stage.telemetry.as_ref().unwrap().started_at.clone();
        stage.telemetry.as_mut().unwrap().updated_at = "1".into();
        write(&db, "job", &stage).unwrap();
        let mut next = JobStage {
            name: "writing".into(),
            rating: Some("g".into()),
            completed: 2,
            total: 2,
            ..Default::default()
        };
        next.telemetry = Some(JobTelemetry {
            started_at: "forged".into(),
            updated_at: "forged".into(),
            heartbeat_at: "forged".into(),
            finished_at: None,
            phases: Vec::new(),
        });
        record(&db, "job", &next).unwrap();
        let stage = read(&db, "job").unwrap().unwrap();
        assert_eq!(stage.telemetry.as_ref().unwrap().started_at, began);
        assert_eq!(stage.rating.as_deref(), Some("g"));
        assert_eq!(
            stage.telemetry.as_ref().unwrap().phases[0].name,
            "scope_basis"
        );
        db.execute("UPDATE jobs SET status='succeeded'", [])
            .unwrap();
        finish(&db, "job").unwrap();
        let completed = read(&db, "job").unwrap().unwrap();
        record(&db, "job", &legacy).unwrap();
        assert_eq!(
            serde_json::to_string(&read(&db, "job").unwrap()).unwrap(),
            serde_json::to_string(&Some(completed.clone())).unwrap()
        );
        record(
            &db,
            "job",
            &JobStage {
                name: "complete".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read(&db, "job")
                .unwrap()
                .unwrap()
                .telemetry
                .unwrap()
                .finished_at,
            completed.telemetry.unwrap().finished_at
        );
    }
}
