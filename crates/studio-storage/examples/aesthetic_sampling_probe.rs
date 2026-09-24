//! Offline dry-run, restricted to an explicitly copied test ledger. No model client.
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};
use studio_application::{aesthetic::AestheticRepository, aesthetic_analysis::estimator};
use studio_domain::{Error, Result, aesthetic::*, aesthetic_analysis::*};
use studio_storage::aesthetic::EvaluationDb;

fn digest(path: &std::path::Path) -> Result<String> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(Error::io)?;
    let mut hash = Sha256::new();
    for query in [
        "SELECT observation FROM evidence ORDER BY sequence",
        "SELECT sha256 FROM raw_receipts ORDER BY attempt_id",
    ] {
        let mut stmt = db.prepare(query).map_err(Error::io)?;
        for value in stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(Error::io)?
        {
            hash.update(value.map_err(Error::io)?.as_bytes());
            hash.update([0]);
        }
    }
    Ok(hex::encode(hash.finalize()))
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err(Error::invalid(
            "usage: aesthetic_sampling_probe COPIED_LEDGER STAGE_ID",
        ));
    }
    let path = PathBuf::from(&args[1]).canonicalize().map_err(Error::io)?;
    let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/test-runs")
        .canonicalize()
        .map_err(Error::io)?;
    if !path.starts_with(allowed) || path.file_name().is_none_or(|n| n != "evaluation.sqlite") {
        return Err(Error::invalid(
            "只允许操作 .local/test-runs 内显式复制的 evaluation.sqlite",
        ));
    }
    let id = &args[2];
    let db = EvaluationDb::open(&path)?;
    let before = db.stage(id)?;
    let preserved = digest(&path)?;
    let read = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(Error::io)?;
    let watermark = read
        .query_row("SELECT COALESCE(MAX(sequence),0) FROM evidence", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(Error::io)?;
    let watermark = u64::try_from(watermark).map_err(Error::io)?;
    drop(read);
    let input = AestheticAnalysisInput {
        stage_id: id.clone(),
        stage_config_hash: before.config_hash.clone(),
        evidence_watermark: watermark,
        observations: before.accepted,
        candidates: before.total,
        review_watermark: 0,
    };
    let fit = AestheticFit {
        stage_id: id.clone(),
        estimator: AestheticEstimator {
            kind: "davidson_v1".into(),
            iterations: 128,
            regularization: 0.1,
            tie_strength: 1.0,
        },
        stability_seed: None,
    };
    let mut components = BTreeMap::new();
    let summary = estimator::replay(
        &db,
        &input,
        &fit,
        &|| Ok(()),
        &mut |_, _, _| Ok(()),
        &mut |rows| {
            for row in rows {
                components.insert(row.ordinal, row.component);
            }
            Ok(())
        },
    )?;
    db.configure_sampling(
        id,
        AestheticSamplingRequest {
            idempotency_key: studio_domain::new_id(),
            additional_calls: 4,
            policy: AestheticSamplingPolicy {
                mode: "balanced".into(),
                min_exposures: before.config.request.exposures,
                max_exposures: (before.config.request.exposures + 4).min(32),
                rank_tolerance: 0.1,
                seed: 17,
            },
        },
    )?;
    db.control(id, "start")?;
    db.plan_sampling(id, &|| Ok(()))?;
    let mut planned = Vec::new();
    while let Some(batch) = db.claim(id)? {
        let mut membership = BTreeMap::new();
        for m in &batch.members {
            *membership
                .entry(format!("{:?}", components[&m.candidate.ordinal]))
                .or_insert(0) += 1;
        }
        planned.push(serde_json::json!({"members":batch.members.len(),"prior_components":membership,"sampling":batch.sampling}));
    }
    db.control(id, "pause")?;
    db.settle(id, None)?;
    let after = db.stage(id)?;
    if before.attempts != after.attempts
        || before.accepted != after.accepted
        || preserved != digest(&path)?
        || before.config_hash != after.config_hash
    {
        return Err(Error::new("PROBE_FAILED", "离线规划改变了付费证据"));
    }
    println!(
        "{}",
        serde_json::json!({"commercial_calls":0,"accepted":before.accepted,"candidates":before.total,"before":summary,"planned":planned,"paid_evidence_unchanged":true,"config_hash_unchanged":true})
    );
    Ok(())
}
