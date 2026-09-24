//! Keyset pages stay bounded while the complete frozen population is retained.
use super::*;
const PAGE: usize = 1024;

pub(in crate::aesthetic) fn diagnostics(
    db: &Connection,
    checkpoint: &AestheticSamplingStatus,
    total: u64,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<AestheticSamplingDiagnostic>> {
    if checkpoint.round == 0 {
        return Ok(vec![]);
    }
    let mut after = -1i64;
    let mut result = Vec::new();
    let mut stmt=db.prepare("SELECT ordinal,data FROM sampling_diagnostics WHERE plan_id=?1 AND round=?2 AND ordinal>?3 ORDER BY ordinal LIMIT ?4").map_err(db_error)?;
    loop {
        check()?;
        let page = stmt
            .query_map(
                params![checkpoint.plan_id, checkpoint.round, after, PAGE as u32],
                |r| Ok((crate::unsigned(r, 0)?, r.get::<_, String>(1)?)),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        if page.is_empty() {
            break;
        }
        for (ordinal, json) in page {
            if ordinal >= total || result.len() as u64 >= total {
                return Err(Error::new("EVIDENCE_INVALID", "采样诊断超出冻结候选范围"));
            }
            let row: AestheticSamplingDiagnostic = decode(json)?;
            if row.ordinal != ordinal {
                return Err(Error::new("EVIDENCE_INVALID", "采样诊断序号不一致"));
            }
            after = ordinal as i64;
            result.push(row);
        }
    }
    if result.len() as u64 != total {
        return Err(Error::new("EVIDENCE_INVALID", "采样检查点诊断不完整"));
    }
    Ok(result)
}

pub(in crate::aesthetic) fn available(
    db: &Connection,
    id: &str,
    total: u64,
    check: &dyn Fn() -> Result<()>,
) -> Result<BTreeSet<u64>> {
    let mut after = -1i64;
    let mut result = BTreeSet::new();
    let mut stmt=db.prepare("SELECT ordinal FROM candidates WHERE stage_id=?1 AND ordinal>?2 AND blocked=0 AND reserved=0 AND disposition IN ('active','rejudge') AND rating IN ('g','s','q','e') ORDER BY ordinal LIMIT ?3").map_err(db_error)?;
    loop {
        check()?;
        let page = stmt
            .query_map(params![id, after, PAGE as u32], |r| crate::unsigned(r, 0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        if page.is_empty() {
            break;
        }
        for ordinal in page {
            if ordinal >= total {
                return Err(Error::new("EVIDENCE_INVALID", "可用候选超出冻结范围"));
            }
            after = ordinal as i64;
            result.insert(ordinal);
        }
    }
    Ok(result)
}
