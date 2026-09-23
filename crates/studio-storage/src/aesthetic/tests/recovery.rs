use super::*;

#[test]
fn disposition_queue_seeks_sparse_candidates_and_v3_migration_preserves_evidence() {
    let (dir, db, id) = fixture(256);
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&id).unwrap();
    let sid = id.clone();
    db.writer.submit(0, move |tx| {
        tx.execute("UPDATE candidates SET disposition='needs_review',disposition_reason='fixture' WHERE stage_id=?1 AND ordinal%2=0", [&sid]).map_err(db_error)?;
        Ok(())
    }).unwrap();
    let first = db
        .filtered_candidates(&id, None, false, Some(AestheticDisposition::NeedsReview))
        .unwrap();
    assert_eq!(first.len(), 64);
    assert!(first.iter().all(|v| v.ordinal % 2 == 0));
    let second = db
        .filtered_candidates(
            &id,
            Some(first[63].ordinal),
            false,
            Some(AestheticDisposition::NeedsReview),
        )
        .unwrap();
    assert_eq!(second.len(), 64);
    assert_eq!(second[0].ordinal, 128);
    assert!(
        db.filtered_candidates(
            &id,
            Some(254),
            false,
            Some(AestheticDisposition::NeedsReview)
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        db.filtered_candidates(
            "other-stage",
            None,
            false,
            Some(AestheticDisposition::NeedsReview)
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        db.filtered_candidates(&id, Some(u64::MAX), false, None)
            .is_err()
    );
    assert_eq!(db.candidate(&id, 200).unwrap().ordinal, 200);
    assert!(db.candidate(&id, 256).is_err());
    assert!(db.candidate(&id, u64::MAX).is_err());
    let plan: String = db.read().unwrap().query_row(
        "EXPLAIN QUERY PLAN SELECT ordinal FROM candidates WHERE stage_id=?1 AND disposition='needs_review' AND ordinal>100 ORDER BY ordinal LIMIT 64",
        [&id], |r| r.get(3)).unwrap();
    assert!(plan.contains("candidate_disposition_page"), "{plan}");
    drop(db);
    let legacy = dir.path().join("v3-upgrade");
    std::fs::create_dir(&legacy).unwrap();
    let path = legacy.join("evaluation.sqlite");
    {
        let source = Connection::open(dir.path().join("evaluation.sqlite")).unwrap();
        let mut target = Connection::open(&path).unwrap();
        rusqlite::backup::Backup::new(&source, &mut target)
            .unwrap()
            .run_to_completion(128, std::time::Duration::from_millis(1), None)
            .unwrap();
        target
            .execute_batch("DROP INDEX candidate_disposition_page; PRAGMA user_version=3;")
            .unwrap();
    }
    let migrated = EvaluationDb::open(&path).unwrap();
    assert_eq!(migrated.stage(&id).unwrap().accepted, 1);
    assert_eq!(
        migrated.candidate(&id, 200).unwrap().disposition,
        AestheticDisposition::NeedsReview
    );
    assert_eq!(
        migrated
            .read()
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        4
    );
    let backup = std::fs::read_dir(legacy.join(".backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let old = Connection::open(backup).unwrap();
    assert_eq!(
        old.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        old.query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
}

#[test]
fn corrupt_ledger_is_classified_without_overwriting_the_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("r1-corrupt-")
        .tempdir_in(root)
        .unwrap();
    let path = dir.path().join("evaluation.sqlite");
    let bytes = b"This is not an SQLite database. Preserve it for recovery.";
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        EvaluationDb::open(&path),
        Err(Error {
            code: "EVALUATION_CORRUPT",
            ..
        })
    ));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn cancelled_materialized_intent_cannot_recreate_a_missing_paid_ledger() {
    let (dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&id).unwrap();
    db.control(&id, "cancel").unwrap();
    let cancelled = db.settle(&id, Some("cancelled by user".into())).unwrap();
    assert_eq!(
        db.settle(&id, Some("retry finalization".into()))
            .unwrap()
            .state,
        "cancelled"
    );
    let mut project = Connection::open(dir.path().join("project.sqlite")).unwrap();
    crate::migrations::initialize(&mut project).unwrap();
    project
        .execute(
            "INSERT INTO evaluation_stage_refs VALUES (?1,NULL,'cancelled',?2,16)",
            params![id, encode(&cancelled.config.request).unwrap()],
        )
        .unwrap();
    crate::aesthetic::project::project_stage(&project, &cancelled).unwrap();
    drop(db);
    std::fs::rename(
        dir.path().join("evaluation.sqlite"),
        dir.path().join("saved-paid.sqlite"),
    )
    .unwrap();
    assert!(matches!(
        crate::aesthetic::recover(&dir.path().canonicalize().unwrap(), &project),
        Err(Error {
            code: "EVALUATION_MISSING",
            ..
        })
    ));
    assert!(!dir.path().join("evaluation.sqlite").exists());
    let saved = Connection::open(dir.path().join("saved-paid.sqlite")).unwrap();
    assert_eq!(
        saved
            .query_row("SELECT count(*) FROM evidence", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
}

fn abstain_one(db: &EvaluationDb, id: &str) -> (AestheticBatch, u64) {
    let (batch, attempt) = sent(db, id);
    let mut response = receipt(&batch);
    let unjudgeable = &batch.members[0];
    response.outputs[0].content = vec![LlmContent::Text {
        text: encode(&AestheticObservation {
            schema_version: 1,
            tiers: vec![batch.members[1..].iter().map(|v| v.label.clone()).collect()],
            elite_candidates: vec![batch.members[1].label.clone()],
            unjudgeable: vec![AestheticUnjudgeable {
                id: unjudgeable.label.clone(),
                reason: "image unreadable".into(),
            }],
        })
        .unwrap(),
    }];
    db.receive(id, &attempt, response).unwrap();
    db.parse_received(id).unwrap();
    db.settle(id, None).unwrap();
    let ordinal = unjudgeable.candidate.ordinal;
    (batch, ordinal)
}
#[test]
fn exclusion_completes_without_inventing_exposure_and_is_idempotent() {
    let (_dir, db, id) = fixture(16);
    let (batch, ordinal) = abstain_one(&db, &id);
    let observation = batch_observation(&db, &id, batch.sequence);
    let before = db.stage(&id).unwrap();
    assert_eq!(
        (before.comparable, before.unresolved, before.excluded),
        (15, 1, 0)
    );
    let decision = AestheticCandidateDecision {
        idempotency_key: new_id(),
        action: AestheticDispositionAction::Exclude,
        reason: "explicitly excluded".into(),
    };
    db.decide_candidate(&id, ordinal, decision.clone()).unwrap();
    db.decide_candidate(&id, ordinal, decision.clone()).unwrap();
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.state, "completed_with_exclusions");
    assert_eq!(
        (stage.comparable, stage.excluded, stage.unresolved),
        (15, 1, 0)
    );
    assert_eq!(
        db.candidate_page(&id, None, false)
            .unwrap()
            .iter()
            .find(|v| v.ordinal == ordinal)
            .unwrap()
            .exposures,
        0
    );
    assert_eq!(db.batches(&id, 0, 20).unwrap()[0].observation, observation);
    let mut conflicting = decision;
    conflicting.reason = "changed".into();
    assert_eq!(
        db.decide_candidate(&id, ordinal, conflicting)
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert!(db.retry_batch(&id, batch.sequence).is_err());
}
fn batch_observation(db: &EvaluationDb, id: &str, sequence: u64) -> Option<AestheticObservation> {
    read_batch(&db.read().unwrap(), id, sequence)
        .unwrap()
        .observation
}
#[test]
fn rejudge_is_a_new_observation_with_same_rating_anchors_and_preserved_history() {
    let (_dir, db, id) = fixture(16);
    let (old, ordinal) = abstain_one(&db, &id);
    let observation = batch_observation(&db, &id, old.sequence);
    db.decide_candidate(
        &id,
        ordinal,
        AestheticCandidateDecision {
            idempotency_key: new_id(),
            action: AestheticDispositionAction::Rejudge,
            reason: "evaluate again".into(),
        },
    )
    .unwrap();
    db.control(&id, "start").unwrap();
    let (batch, attempt) = sent(&db, &id);
    assert_ne!(batch.sequence, old.sequence);
    assert!(batch.members.len() >= 2);
    assert!(batch.members.iter().any(|v| v.candidate.ordinal == ordinal));
    assert!(
        batch
            .members
            .iter()
            .all(|v| v.candidate.rating == old.rating)
    );
    db.receive(&id, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&id).unwrap();
    let stage = db.settle(&id, None).unwrap();
    assert_eq!(stage.state, "completed");
    assert_eq!(
        (
            stage.attempts,
            stage.accepted,
            stage.comparable,
            stage.unresolved
        ),
        (2, 2, 16, 0)
    );
    assert_eq!(batch_observation(&db, &id, old.sequence), observation);
}
#[test]
fn singleton_never_purchases_a_meaningless_relative_comparison() {
    let (_dir, db, id) = fixture(1);
    assert!(db.claim(&id).unwrap().is_none());
    let stage = db.settle(&id, None).unwrap();
    assert_eq!(stage.attempts, 0);
    assert_eq!(stage.unresolved, 1);
    assert_eq!(
        db.candidate_page(&id, None, false).unwrap()[0]
            .disposition_reason
            .as_deref(),
        Some("no_comparison_peer")
    );
}
#[test]
fn sqlite_busy_rolls_back_and_recovers_without_reapplying_a_receipt() {
    let (dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    let lock = Connection::open(dir.path().join("evaluation.sqlite")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let response = receipt(&batch);
    assert_eq!(
        db.receive(&id, &attempt, response.clone())
            .unwrap_err()
            .code,
        "EVALUATION_BUSY"
    );
    lock.execute_batch("ROLLBACK").unwrap();
    db.receive(&id, &attempt, response.clone()).unwrap();
    db.receive(&id, &attempt, response).unwrap();
    db.health_check().unwrap();
    assert_eq!(db.parse_received(&id).unwrap(), 1);
    assert_eq!(db.parse_received(&id).unwrap(), 0);
    assert_eq!(db.stage(&id).unwrap().input_tokens, 17);
}
#[test]
fn v1_and_v2_ledgers_migrate_paid_history_and_abstentions_with_durable_backups() {
    for version in [1, 2] {
        let (dir, db, id) = fixture(16);
        let (_, ordinal) = abstain_one(&db, &id);
        let legacy = dir.path().join(format!("legacy-{version}"));
        std::fs::create_dir(&legacy).unwrap();
        let path = legacy.join("evaluation.sqlite");
        legacy_copy(&dir.path().join("evaluation.sqlite"), &path, version);
        let migrated = EvaluationDb::open(&path).unwrap();
        let stage = migrated.stage(&id).unwrap();
        assert_eq!(
            (
                stage.attempts,
                stage.accepted,
                stage.comparable,
                stage.unresolved
            ),
            (1, 1, 15, 1)
        );
        assert_eq!(
            migrated
                .candidate_page(&id, None, false)
                .unwrap()
                .iter()
                .find(|v| v.ordinal == ordinal)
                .unwrap()
                .disposition,
            AestheticDisposition::NeedsReview
        );
        assert!(
            migrated.attempts(&id, 1).unwrap()[0]
                .semantic_request_hash
                .is_none()
        );
        assert_eq!(migrated.parse_received(&id).unwrap(), 0);
        assert_eq!(
            migrated
                .read()
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            4
        );
        let backup = std::fs::read_dir(legacy.join(".backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let saved = Connection::open(backup).unwrap();
        assert_eq!(
            saved
                .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            saved
                .query_row("SELECT count(*) FROM evidence", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
    }
}
