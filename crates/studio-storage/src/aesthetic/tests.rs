use super::*;
use studio_domain::{AssetKey, llm::*, new_id};
mod analysis;
mod execution;
mod recovery;
mod sampling;
mod transport;
mod usage;

fn fixture(count: u64) -> (tempfile::TempDir, EvaluationDb, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("aesthetic-ledger-")
        .tempdir_in(root)
        .unwrap();
    let db = EvaluationDb::open(&directory.path().join("evaluation.sqlite")).unwrap();
    let id = new_id();
    let request = AestheticCreate {
        idempotency_key: id.clone(),
        name: "测试".into(),
        collection_id: new_id(),
        model_id: new_id(),
        system_prompt_id: new_id(),
        overrides: Default::default(),
        exposures: 1,
        max_calls: 1000,
        concurrency: 4,
        expected_input_version: None,
        max_request_mib: None,
        sampling: None,
        execution_policy: None,
        budget_mode: None,
    };
    let model = LlmInvocationSnapshot {
        schema_version: 1,
        invocation_id: id.clone(),
        provider_id: new_id(),
        provider_revision: 1,
        provider_kind: LlmProviderKind::OpenaiCompatible,
        base_url: "http://127.0.0.1:1".into(),
        model_id: request.model_id.clone(),
        model_revision: 1,
        remote_model_id: "fixture".into(),
        protocol: LlmProtocol::OpenaiChat,
        preset_id: None,
        preset_revision: None,
        system_prompt_id: Some(request.system_prompt_id.clone()),
        system_prompt_revision: Some(1),
        parameters: Default::default(),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: vec![LlmContent::Text {
                text: studio_application::aesthetic::OUTPUT_INSTRUCTIONS.into(),
            }],
        }],
        tools: vec![],
        warnings: vec![],
    };
    db.create(
        AestheticConfig {
            version: 1,
            request,
            model,
            sources: vec![],
            image_policy: "stored_original_v1".into(),
            grouping_policy: "test".into(),
            observation_policy: "meaningful_indifference_v1".into(),
            max_image_bytes: 2 << 20,
            max_request_bytes: 12 << 20,
            execution: None,
        },
        count,
    )
    .unwrap();
    for start in (0..count).step_by(128) {
        db.append_candidates(
            &id,
            (start..(start + 128).min(count))
                .map(|ordinal| AestheticCandidate {
                    ordinal,
                    key: AssetKey {
                        source_id: "fixture".into(),
                        asset_id: format!("{ordinal:064x}"),
                    },
                    rating: "g".into(),
                    year: Some(2020),
                    basis: "test".into(),
                    content_version: format!("sha256:{ordinal:064x}"),
                    bytes: 10,
                    exposures: 0,
                    protected: false,
                    disposition: Default::default(),
                    disposition_reason: None,
                    blocked: false,
                    blocking_batch: None,
                })
                .collect(),
        )
        .unwrap();
    }
    db.finish_freeze(&id).unwrap();
    db.control(&id, "start").unwrap();
    (directory, db, id)
}
fn sent(db: &EvaluationDb, id: &str) -> (AestheticBatch, String) {
    let mut batch = db.claim(id).unwrap().unwrap();
    for member in &mut batch.members {
        member.image_sha256 = Some("a".repeat(64));
    }
    let attempt = new_id();
    assert!(
        db.begin_attempt(
            id,
            batch.sequence,
            batch.members.clone(),
            attempt.clone(),
            "f".repeat(64)
        )
        .unwrap()
    );
    (batch, attempt)
}
fn receipt(batch: &AestheticBatch) -> AestheticReceipt {
    let observation = AestheticObservation {
        schema_version: 1,
        tiers: vec![batch.members.iter().map(|m| m.label.clone()).collect()],
        elite_candidates: vec![batch.members[0].label.clone()],
        unjudgeable: vec![],
    };
    AestheticReceipt {
        provider_request_id: Some("request-1".into()),
        response_id: None,
        model: Some("fixture".into()),
        outputs: vec![LlmOutput {
            index: 0,
            content: vec![LlmContent::Text {
                text: encode(&observation).unwrap(),
            }],
            finish_reason: Some("stop".into()),
        }],
        usage: LlmUsage {
            input_tokens: Some(17),
            output_tokens: Some(9),
            ..Default::default()
        },
    }
}
#[test]
fn committed_response_replays_once_and_elite_does_not_add_exposure() {
    let (dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let response = receipt(&batch);
    db.receive(&id, &attempt, response.clone()).unwrap();
    db.receive(&id, &attempt, response).unwrap();
    assert_eq!(db.stage(&id).unwrap().accepted, 0);
    let backup = dir.path().join("backup.sqlite");
    db.backup(&backup).unwrap();
    let saved = Connection::open(backup).unwrap();
    assert_eq!(
        saved
            .query_row(
                "SELECT count(*) FROM attempts WHERE receipt IS NOT NULL",
                [],
                |r| crate::unsigned(r, 0)
            )
            .unwrap(),
        1
    );
    drop(db);
    let reopened = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    assert_eq!(reopened.stage(&id).unwrap().state, "paused");
    assert_eq!(reopened.parse_received(&id).unwrap(), 1);
    assert_eq!(reopened.parse_received(&id).unwrap(), 0);
    let stage = reopened.stage(&id).unwrap();
    assert_eq!(stage.accepted, 1);
    assert_eq!(stage.protected, 1);
    assert_eq!(stage.input_tokens, 17);
    let rows = reopened.candidate_page(&id, None, false).unwrap();
    assert!(rows.iter().all(|r| r.exposures == 1));
    assert!(reopened.retry_batch(&id, batch.sequence).is_err());
}
#[test]
fn interrupted_send_is_unknown_and_explicit_retry_keeps_one_accepted_observation() {
    let (dir, db, id) = fixture(16);
    let (batch, _) = sent(&db, &id);
    drop(db);
    let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    assert_eq!(db.stage(&id).unwrap().unknown, 1);
    db.control(&id, "start").unwrap();
    assert!(db.claim(&id).unwrap().is_none());
    db.settle(&id, None).unwrap();
    db.retry_batch(&id, batch.sequence).unwrap();
    db.control(&id, "start").unwrap();
    let (retry, attempt) = sent(&db, &id);
    assert_eq!(retry.sequence, batch.sequence);
    db.receive(&id, &attempt, receipt(&retry)).unwrap();
    db.parse_received(&id).unwrap();
    assert_eq!(db.attempts(&id, batch.sequence).unwrap().len(), 2);
    assert_eq!(db.stage(&id).unwrap().accepted, 1);
    assert_eq!(db.stage(&id).unwrap().unknown, 0);
}
#[test]
fn invalid_response_and_abstention_are_never_low_quality_or_valid_exposure() {
    let (_dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let mut response = receipt(&batch);
    response.outputs[0].content=vec![LlmContent::Text{text:"{\"schema_version\":1,\"tiers\":[[\"img01\"]],\"elite_candidates\":[],\"unjudgeable\":[]}".into()}];
    db.receive(&id, &attempt, response).unwrap();
    assert_eq!(db.parse_received(&id).unwrap(), 0);
    assert_eq!(db.stage(&id).unwrap().invalid, 1);
    assert!(
        db.candidate_page(&id, None, false)
            .unwrap()
            .iter()
            .all(|v| v.exposures == 0)
    );
    db.settle(&id, None).unwrap();
    db.retry_batch(&id, batch.sequence).unwrap();
    db.control(&id, "start").unwrap();
    let (batch, attempt) = sent(&db, &id);
    let mut response = receipt(&batch);
    response.outputs[0].content = vec![LlmContent::Text {
        text: encode(&AestheticObservation {
            schema_version: 1,
            tiers: vec![],
            elite_candidates: vec![],
            unjudgeable: batch
                .members
                .iter()
                .map(|m| AestheticUnjudgeable {
                    id: m.label.clone(),
                    reason: "不可读".into(),
                })
                .collect(),
        })
        .unwrap(),
    }];
    db.receive(&id, &attempt, response).unwrap();
    db.parse_received(&id).unwrap();
    assert!(
        db.candidate_page(&id, None, false)
            .unwrap()
            .iter()
            .all(|v| v.exposures == 0)
    );
}
#[test]
fn paused_preparation_is_resumable_and_call_budget_is_atomic() {
    let (_dir, db, id) = fixture(32);
    let batch = db.claim(&id).unwrap().unwrap();
    db.control(&id, "pause").unwrap();
    db.settle(&id, None).unwrap();
    db.control(&id, "start").unwrap();
    assert_eq!(db.claim(&id).unwrap().unwrap().sequence, batch.sequence);
    let sid = id.clone();
    db.writer
        .submit(1024, move |db| {
            db.execute("UPDATE stages SET attempts=1000 WHERE id=?1", [sid])
                .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
    let mut members = batch.members;
    for m in &mut members {
        m.image_sha256 = Some("b".repeat(64));
    }
    assert!(
        !db.begin_attempt(&id, batch.sequence, members, new_id(), "f".repeat(64))
            .unwrap()
    );
}
#[test]
fn simultaneous_paid_returns_have_bounded_queue_and_durable_counts() {
    let (_dir, db, id) = fixture(1600);
    let db = std::sync::Arc::new(db);
    let pending = (0..100).map(|_| sent(&db, &id)).collect::<Vec<_>>();
    std::thread::scope(|scope| {
        for (batch, attempt) in pending {
            let db = db.clone();
            let id = id.clone();
            scope.spawn(move || {
                loop {
                    match db.receive(&id, &attempt, receipt(&batch)) {
                        Ok(()) => break,
                        Err(e) if e.code == "EVALUATION_BUSY" => std::thread::yield_now(),
                        Err(e) => panic!("{e}"),
                    }
                }
            });
        }
    });
    assert_eq!(db.parse_received(&id).unwrap(), 100);
    assert_eq!(db.stage(&id).unwrap().accepted, 100);
    assert!(db.metrics().unwrap().1 <= 64 << 20);
    let db = db.read().unwrap();
    assert_eq!(
        db.query_row("SELECT sum(exposures) FROM candidates", [], |r| {
            crate::unsigned(r, 0)
        })
        .unwrap(),
        1600
    );
    assert_eq!(
        db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

fn legacy_copy(source: &Path, path: &Path, version: u32) {
    let old = Connection::open(path).unwrap();
    old.execute_batch(include_str!("schema.sql")).unwrap();
    if version == 2 {
        old.execute_batch(include_str!("schema_v2.sql")).unwrap();
    }
    old.execute("ATTACH DATABASE ?1 AS current", [source.to_str().unwrap()])
        .unwrap();
    for table in ["stages", "candidates", "batches", "attempts", "evidence"] {
        let names = old
            .prepare(&format!("PRAGMA main.table_info({table})"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join(",");
        old.execute_batch(&format!(
            "INSERT INTO main.{table}({names}) SELECT {names} FROM current.{table}"
        ))
        .unwrap();
    }
    drop(old);
}
