use super::*;

#[test]
fn cache_usage_is_idempotent_and_reparse_replaces_unknown_metrics_once() {
    let (_dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let mut bad = receipt(&batch);
    bad.outputs[0].content = vec![LlmContent::Text {
        text: "invalid".into(),
    }];
    db.receive(&id, &attempt, bad.clone()).unwrap();
    db.receive(&id, &attempt, bad).unwrap();
    let before = db.stage(&id).unwrap().usage_summary;
    assert_eq!(before.recorded_requests, 1);
    assert_eq!(before.cache_observed_requests, 0);
    assert_eq!(before.cost_observed_requests, 0);
    db.parse_received(&id).unwrap();
    db.settle(&id, Some("invalid".into())).unwrap();
    let mut good = receipt(&batch);
    good.usage.cached_input_tokens = Some(12);
    good.usage.cache_write_tokens = Some(0);
    good.usage.cost_usd = Some(0.000123);
    db.apply_reparsed(&id, &attempt, good.clone()).unwrap();
    db.apply_reparsed(&id, &attempt, good).unwrap();
    let s = db.stage(&id).unwrap().usage_summary;
    assert_eq!(s.recorded_requests, 1);
    assert_eq!(s.cache_observed_requests, 1);
    assert_eq!(s.cache_hit_requests, 1);
    assert_eq!(s.cached_input_tokens, 12);
    assert_eq!(s.cache_observed_input_tokens, 17);
    assert_eq!(s.cache_write_observed_requests, 1);
    assert_eq!(s.cache_write_tokens, 0);
    assert_eq!(s.cost_observed_requests, 1);
    assert!((s.cost_usd - 0.000123).abs() < 1e-12);
}

#[test]
fn cache_usage_v8_upgrade_backfills_existing_receipts_and_preserves_unknown_cost() {
    let (dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let mut result = receipt(&batch);
    result.usage.cached_input_tokens = Some(12);
    db.receive(&id, &attempt, result).unwrap();
    let path = dir.path().join("evaluation.sqlite");
    drop(db);
    let legacy = Connection::open(&path).unwrap();
    legacy
        .execute_batch("ALTER TABLE analysis_jobs DROP COLUMN deleted; ALTER TABLE analysis_jobs DROP COLUMN name; ALTER TABLE candidates DROP COLUMN unjudgeable_streak; ALTER TABLE stages DROP COLUMN usage_summary; ALTER TABLE attempts DROP COLUMN image_inputs; PRAGMA user_version=8;")
        .unwrap();
    drop(legacy);
    let reopened = EvaluationDb::open(&path).unwrap();
    let s = reopened.stage(&id).unwrap().usage_summary;
    assert_eq!(s.recorded_requests, 1);
    assert_eq!(s.cache_observed_requests, 1);
    assert_eq!(s.cache_hit_requests, 1);
    assert_eq!(s.cached_input_tokens, 12);
    assert_eq!(s.cost_observed_requests, 0);
    assert_eq!(s.cache_write_observed_requests, 0);
    assert!(
        dir.path()
            .join(".backups")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
    drop(reopened);
    let reopened = EvaluationDb::open(&path).unwrap();
    assert_eq!(
        reopened
            .stage(&id)
            .unwrap()
            .usage_summary
            .cached_input_tokens,
        12
    );
}
