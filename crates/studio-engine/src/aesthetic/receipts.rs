use super::*;

pub fn reparse_batch(state: &AppState, pid: &str, id: &str, sequence: u64) -> Result<()> {
    let db = state.store.evaluation(pid)?;
    let stage = db.stage(id)?;
    if matches!(
        stage.state.as_str(),
        "running" | "preparing" | "pausing" | "cancelling"
    ) {
        return Err(Error::new("REVISION_CONFLICT", "请先等待阶段暂停"));
    }
    let batch = db
        .batches(id, sequence.saturating_sub(1), 1)?
        .into_iter()
        .find(|b| b.sequence == sequence)
        .ok_or_else(|| Error::new("NOT_FOUND", "批次不存在"))?;
    if batch.state == "accepted" {
        return Ok(());
    }
    let attempt = batch
        .attempt_id
        .ok_or_else(|| Error::invalid("批次尚未发送"))?;
    let receipt = db
        .raw_receipt(id, &attempt)?
        .ok_or_else(|| Error::new("NOT_FOUND", "此历史调用没有原始 HTTP 回执"))?;
    let mut snapshot = stage.config.model.clone();
    snapshot.invocation_id = attempt.clone();
    match state.llm.reparse(&snapshot, &receipt) {
        Ok(response) => {
            db.apply_reparsed(id, &attempt, response.into())?;
            db.parse_received(id)?;
            let stage = db.stage(id)?;
            state.store.sync_evaluation(pid, &stage)?;
            Ok(())
        }
        Err(error) => {
            db.record_parse(
                id,
                &attempt,
                Some(format!("{}: {}", error.code, error.message)),
            )?;
            Err(Error::new(
                "EVALUATION_REPARSE_FAILED",
                format!("{}；未发送任何网络请求", error.message),
            ))
        }
    }
}
