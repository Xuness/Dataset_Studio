use super::*;
use std::time::Instant;
#[cfg(test)]
mod tests;

// A delta starts with the changed keys. A correlated EXISTS here instead walks
// every old member in the source even when just one asset was re-evaluated.
const CLOSE_DELTA: &str = "UPDATE query_member_data AS m SET valid_until=?3
    WHERE family_id=?1 AND source_id=?2 AND valid_until IS NULL
    AND asset_id IN (SELECT asset_id FROM query_stage.affected WHERE source_id=?2)
    AND NOT EXISTS(SELECT 1 FROM query_stage.matches s
        WHERE s.source_id=m.source_id AND s.asset_id=m.asset_id AND s.post_id IS m.post_id)";
const CLOSE_FULL: &str = "UPDATE query_member_data AS m SET valid_until=?3
    WHERE family_id=?1 AND source_id=?2 AND valid_until IS NULL
    AND NOT EXISTS(SELECT 1 FROM query_stage.matches s
        WHERE s.source_id=m.source_id AND s.asset_id=m.asset_id AND s.post_id IS m.post_id)";

pub(super) fn forget_receipt(db: &Connection, rid: &str) -> Result<()> {
    db.execute(
        "DELETE FROM meta WHERE key=?1",
        [format!("query_publication/{rid}")],
    )
    .map_err(db_error)?;
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Explicit publication and resource limits"
)]
pub(super) fn publish(
    db: &mut Connection,
    pid: &str,
    rid: &str,
    stage: &QueryStage,
    mode: &str,
    cancelled: &AtomicBool,
    cache_bytes: u64,
) -> Result<()> {
    studio_application::read_cancelled(cancelled)?;
    if !stage.db.is_autocommit() {
        return Err(Error::invalid("查询暂存尚未封存"));
    }
    let result = query::read_result(db, pid, rid)?;
    if result.state != ResultState::Running {
        return Err(Error::new("CANCELLED", "结果构建已停止"));
    }
    let (family, revision, latest, baseline, current_count): (String, i64, i64, u64, u64) = db
        .query_row(
            "SELECT r.family_id,r.member_revision,f.latest_revision,f.latest_count,COALESCE(r.count,0)
         FROM query_results r JOIN query_families f ON f.id=r.family_id WHERE r.id=?1",
            [rid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, unsigned(r, 3)?, unsigned(r, 4)?)),
        )
        .map_err(db_error)?;
    let receipt = format!("query_publication/{rid}");
    let pending: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM query_results r WHERE r.family_id=?1 AND
            ((r.id<>?2 AND r.status IN ('queued','running')) OR
             EXISTS(SELECT 1 FROM meta WHERE key='query_publication/'||r.id)))",
            params![family, rid],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if pending || latest.checked_add(1) != Some(revision) || current_count != 0 {
        return Err(Error::new(
            "QUERY_PUBLICATION_CONFLICT",
            "查询族存在重复、错序或未完成的发布",
        ));
    }
    let old_cache: i64 = db
        .query_row("PRAGMA cache_size", [], |r| r.get(0))
        .map_err(db_error)?;
    // Only the project is written. Staging is sealed; there is no cross-file commit.
    db.execute(
        "ATTACH DATABASE ?1 AS query_stage",
        [stage.file.path().to_string_lossy().as_ref()],
    )
    .map_err(db_error)?;
    let outcome = (|| {
        db.execute_batch(&format!(
            "PRAGMA cache_size=-{}",
            cache_bytes.clamp(64 << 20, 4 << 30) / 1024
        ))
        .map_err(db_error)?;
        let transaction = Instant::now();
        let tx = db.transaction().map_err(db_error)?;
        let closing = Instant::now();
        let mut closed = 0u64;
        for source in &result.spec.source_ids {
            studio_application::read_cancelled(cancelled)?;
            let sql = if stage.full_sources.contains(source) {
                CLOSE_FULL
            } else {
                CLOSE_DELTA
            };
            closed += tx
                .execute(sql, params![family, source, revision])
                .map_err(db_error)? as u64;
        }
        let close_us = closing.elapsed().as_micros() as u64;
        let inserting = Instant::now();
        let mut inserted = 0u64;
        let mut after = (String::new(), String::new());
        loop {
            studio_application::read_cancelled(cancelled)?;
            let last = {
                let mut stmt = tx
                    .prepare(
                        "SELECT source_id,asset_id FROM query_stage.matches
                    WHERE (source_id,asset_id)>(?1,?2) ORDER BY source_id,asset_id LIMIT 32768",
                    )
                    .map_err(db_error)?;
                stmt.query_map(params![after.0, after.1], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(db_error)?
                .last()
                .transpose()
                .map_err(db_error)?
            };
            let Some(last) = last else { break };
            inserted += tx
                .execute(
                    "INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id)
                SELECT ?1,s.source_id,s.asset_id,?2,s.post_id FROM query_stage.matches s
                WHERE (s.source_id,s.asset_id)>(?3,?4) AND (s.source_id,s.asset_id)<=(?5,?6)
                AND NOT EXISTS(SELECT 1 FROM query_member_data m WHERE m.family_id=?1
                    AND m.source_id=s.source_id AND m.asset_id=s.asset_id AND m.valid_until IS NULL)
                ORDER BY s.source_id,s.asset_id",
                    params![family, revision, after.0, after.1, last.0, last.1],
                )
                .map_err(db_error)? as u64;
            after = last;
        }
        let insert_us = inserting.elapsed().as_micros() as u64;
        studio_application::read_cancelled(cancelled)?;
        // The initial family has baseline zero, so deduplicated successful inserts
        // also give its exact first count without a second walk over the result.
        let count = baseline
            .checked_sub(closed)
            .and_then(|n| n.checked_add(inserted))
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| Error::new("DATABASE_ERROR", "查询族成员计数不一致"))?;
        let changed = closed + inserted;
        tx.execute(
            "UPDATE query_families SET stored_members=stored_members+?2 WHERE id=?1",
            params![family, inserted as i64],
        )
        .map_err(db_error)?;
        tx.execute("UPDATE query_results SET count=?2,processed=?3,cache_mode=?4,evaluated_count=?5,changed_members=?6,post_ready=?7 WHERE id=?1",
            params![rid, count as i64, stage.processed as i64, mode, stage.evaluated as i64, changed as i64, stage.post_ready]).map_err(db_error)?;
        if changed == 0 && revision > 1 {
            tx.execute(
                "UPDATE query_results SET member_revision=?2 WHERE id=?1",
                params![rid, latest],
            )
            .map_err(db_error)?;
        }
        if changed > 0 {
            touch_sizes(&tx)?;
        }
        // Prevent replay before finish_result's final source fence. This receipt
        // is committed with the members and removed by success or rollback/recovery.
        tx.execute(
            "INSERT INTO meta(key,value) VALUES(?1,'published')",
            [&receipt],
        )
        .map_err(db_error)?;
        let committing = Instant::now();
        tx.commit().map_err(db_error)?;
        tracing::info!(target: "studio_storage::query_publish", project_id = pid, result_id = rid,
            baseline, closed, inserted, count, close_us, insert_us,
            commit_us = committing.elapsed().as_micros() as u64,
            transaction_us = transaction.elapsed().as_micros() as u64,
            sqlite_version = rusqlite::version(), "query membership committed");
        Ok(())
    })();
    let detached = db
        .execute_batch("DETACH DATABASE query_stage;")
        .map_err(db_error);
    let restored = db
        .execute_batch(&format!(
            "PRAGMA cache_size={old_cache}; PRAGMA shrink_memory;"
        ))
        .map_err(db_error);
    outcome.and(detached).and(restored)
}
