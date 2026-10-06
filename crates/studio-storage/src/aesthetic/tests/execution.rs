use super::*;

pub(super) fn enable(db: &EvaluationDb, id: &str, policy: AestheticExecutionPolicy) {
    db.control(id, "pause").unwrap();
    db.settle(id, None).unwrap();
    db.configure_execution(
        id,
        AestheticExecutionUpdate {
            idempotency_key: new_id(),
            expected_revision: 0,
            policy,
        },
        1,
        1,
    )
    .unwrap();
    db.control(id, "start").unwrap();
}
fn unknown() -> LlmFailure {
    let mut failure = LlmFailure::new("LLM_TIMEOUT", "test timeout");
    failure.outcome_unknown = true;
    failure
}
pub(super) fn due(db: &EvaluationDb, sequence: u64) {
    db.writer
        .submit(1024, move |tx| {
            tx.execute(
                "UPDATE batches SET retry_at=0 WHERE sequence=?1",
                [sequence as i64],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn transport_locks_are_visible_without_becoming_candidate_quality_decisions() {
    let (_dir, db, id) = fixture(32);
    let (batch, attempt) = sent(&db, &id);
    db.fail_attempt(&id, &attempt, unknown()).unwrap();
    assert_eq!(
        db.schedule_recovery(&id, batch.sequence, unknown())
            .unwrap(),
        "unresolved"
    );
    let blocked = db.blocked_candidates(&id, None).unwrap();
    assert_eq!(blocked.len(), 16);
    assert!(
        blocked
            .iter()
            .all(|c| c.disposition == AestheticDisposition::Active
                && c.blocked
                && c.blocking_batch == Some(batch.sequence))
    );
    assert_eq!(db.stage(&id).unwrap().progress.blocked, 16);
    assert_eq!(db.stage(&id).unwrap().progress.failed, 1);
    assert_eq!(
        db.filtered_batches(&id, 0, 20, Some("issues"), None)
            .unwrap()
            .len(),
        1
    );
    let next = db.claim(&id).unwrap().unwrap();
    assert_ne!(next.sequence, batch.sequence);
}

#[test]
fn automatic_retry_is_durable_and_accepts_one_observation_only() {
    let (dir, db, id) = fixture(16);
    enable(
        &db,
        &id,
        AestheticExecutionPolicy {
            retry_unknown: true,
            max_retries: 1,
            ..Default::default()
        },
    );
    let (batch, attempt) = sent(&db, &id);
    let deadline = db.batch_deadline(&id, batch.sequence).unwrap();
    db.fail_attempt(&id, &attempt, unknown()).unwrap();
    assert_eq!(
        db.schedule_recovery(&id, batch.sequence, unknown())
            .unwrap(),
        "retry_wait"
    );
    assert_eq!(db.stage(&id).unwrap().progress.retry_waiting, 1);
    drop(db);
    let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    assert_eq!(db.stage(&id).unwrap().state, "paused");
    assert_eq!(db.stage(&id).unwrap().attempts, 1);
    assert!(db.claim(&id).unwrap().is_none());
    due(&db, batch.sequence);
    db.control(&id, "start").unwrap();
    let (retry, second) = sent(&db, &id);
    assert_eq!(retry.sequence, batch.sequence);
    assert_eq!(db.batch_deadline(&id, batch.sequence).unwrap(), deadline);
    db.receive(&id, &second, receipt(&retry)).unwrap();
    db.parse_received(&id).unwrap();
    let stage = db.settle(&id, None).unwrap();
    assert_eq!(stage.state, "completed");
    assert_eq!(
        (
            stage.attempts,
            stage.accepted,
            stage.progress.covered,
            stage.progress.blocked
        ),
        (2, 1, 16, 0)
    );
    let attempts = db.attempts(&id, batch.sequence).unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].state, "outcome_unknown");
    assert_eq!(attempts[1].state, "accepted");
    assert!(attempts.iter().all(|a| a.execution_settings.is_some()));
}

#[test]
fn recovery_exhaustion_can_close_a_batch_without_inventing_exposure() {
    let (_dir, db, id) = fixture(16);
    enable(
        &db,
        &id,
        AestheticExecutionPolicy {
            retry_unknown: true,
            max_retries: 1,
            exhausted: "defer".into(),
            ..Default::default()
        },
    );
    let (batch, first) = sent(&db, &id);
    db.fail_attempt(&id, &first, unknown()).unwrap();
    db.schedule_recovery(&id, batch.sequence, unknown())
        .unwrap();
    due(&db, batch.sequence);
    let (_, second) = sent(&db, &id);
    db.fail_attempt(&id, &second, unknown()).unwrap();
    assert_eq!(
        db.schedule_recovery(&id, batch.sequence, unknown())
            .unwrap(),
        "deferred"
    );
    let stage = db.stage(&id).unwrap();
    assert_eq!(
        (
            stage.accepted,
            stage.progress.covered,
            stage.progress.blocked,
            stage.progress.deferred
        ),
        (0, 0, 0, 1)
    );
    assert!(
        db.candidate_page(&id, None, false)
            .unwrap()
            .iter()
            .all(|c| c.exposures == 0 && !c.blocked)
    );
    assert_eq!(
        db.receive(&id, &second, receipt(&batch)).unwrap_err().code,
        "EVALUATION_BATCH_CLOSED"
    );
    let (new, third) = sent(&db, &id);
    assert_ne!(new.sequence, batch.sequence);
    db.receive(&id, &third, receipt(&new)).unwrap();
    db.parse_received(&id).unwrap();
    assert_eq!(db.settle(&id, None).unwrap().state, "completed");
    assert_eq!(db.stage(&id).unwrap().accepted, 1);
}

#[test]
fn an_expired_retry_window_never_dispatches_another_attempt() {
    let (_dir, db, id) = fixture(16);
    enable(
        &db,
        &id,
        AestheticExecutionPolicy {
            retry_unknown: true,
            ..Default::default()
        },
    );
    let (batch, attempt) = sent(&db, &id);
    db.fail_attempt(&id, &attempt, unknown()).unwrap();
    db.schedule_recovery(&id, batch.sequence, unknown())
        .unwrap();
    let seq = batch.sequence;
    db.writer
        .submit(1024, move |tx| {
            tx.execute(
                "UPDATE batches SET recovery_deadline=0,retry_at=0 WHERE sequence=?1",
                [seq as i64],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
    assert!(db.claim(&id).unwrap().is_none());
    assert_eq!(db.stage(&id).unwrap().attempts, 1);
    assert_eq!(db.batches(&id, 0, 10).unwrap()[0].state, "outcome_unknown");
    db.settle(&id, None).unwrap();
    db.retry_batch(&id, batch.sequence).unwrap();
    db.control(&id, "start").unwrap();
    let _ = sent(&db, &id);
    assert!(db.batch_deadline(&id, batch.sequence).unwrap().unwrap() > 0);
}

#[test]
fn bulk_recovery_resumes_in_bounded_pages_without_repeating_completed_items() {
    let (_dir, db, id) = fixture(640);
    for _ in 0..40 {
        let (_, attempt) = sent(&db, &id);
        db.fail_attempt(&id, &attempt, unknown()).unwrap();
    }
    db.settle(&id, None).unwrap();
    let request = AestheticBatchAction {
        idempotency_key: new_id(),
        action: "retry".into(),
        batches: vec![],
        acknowledge_possible_charge: true,
        reason: String::new(),
    };
    let first = db.batch_action(&id, request.clone()).unwrap();
    assert!(!first.completed);
    assert_eq!(first.succeeded, 32);
    let second = db.batch_action(&id, request.clone()).unwrap();
    assert!(second.completed);
    assert_eq!(
        (second.processed, second.succeeded, second.failed),
        (40, 40, 0)
    );
    let repeated = db.batch_action(&id, request).unwrap();
    assert_eq!(repeated.processed, 40);
    let stage = db.stage(&id).unwrap();
    assert_eq!(
        (stage.attempts, stage.unknown, stage.progress.queued),
        (40, 0, 40)
    );
}

#[test]
fn metadata_and_operational_revisions_preserve_the_frozen_standard() {
    let (_dir, db, id) = fixture(16);
    db.control(&id, "pause").unwrap();
    db.settle(&id, None).unwrap();
    let original = db.stage(&id).unwrap();
    let request = AestheticExecutionUpdate {
        idempotency_key: new_id(),
        expected_revision: 0,
        policy: Default::default(),
    };
    db.configure_execution(&id, request.clone(), 2, 3).unwrap();
    db.configure_execution(&id, request.clone(), 2, 3).unwrap();
    assert_eq!(
        db.stage(&id).unwrap().execution_settings.unwrap().revision,
        1
    );
    let mut changed = request;
    changed.policy.concurrency = 4;
    assert_eq!(
        db.configure_execution(&id, changed, 2, 3).unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    db.stage_metadata(
        &id,
        AestheticStageMetadata {
            name: "整理后的名称".into(),
            archived: true,
        },
    )
    .unwrap();
    assert!(db.control(&id, "start").is_err());
    assert!(
        db.recent_stages(None, 25, false, "", None)
            .unwrap()
            .is_empty()
    );
    let stage = db
        .recent_stages(None, 25, true, "整理", None)
        .unwrap()
        .remove(0);
    assert_eq!(stage.config_hash, original.config_hash);
    assert_eq!(
        encode(&stage.config).unwrap(),
        encode(&original.config).unwrap()
    );
}

#[test]
fn v7_upgrade_preserves_unknown_history_and_adds_stable_local_numbers() {
    let (dir, db, id) = fixture(16);
    let config = db.stage(&id).unwrap().config;
    drop(db);
    let path = dir.path().join("legacy-v7.sqlite");
    let old = Connection::open(&path).unwrap();
    for sql in [
        include_str!("../schema.sql"),
        include_str!("../schema_v2.sql"),
        include_str!("../schema_v3.sql"),
        include_str!("../schema_v4.sql"),
        include_str!("../schema_v5.sql"),
        include_str!("../schema_v6.sql"),
        include_str!("../schema_v7.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    let json = encode(&config).unwrap();
    old.execute("INSERT INTO stages(id,name,state,created_at,config,config_hash,request_json,total,frozen,eligible,attempts,unknown) VALUES(?1,'legacy','paused','1000',?2,?3,?4,16,16,16,1,1)",params![id,json,hash(&json),encode(&config.request).unwrap()]).unwrap();
    let members: Vec<AestheticMember> = (0..16)
        .map(|ordinal| AestheticMember {
            label: format!("img{:02}", ordinal + 1),
            candidate: AestheticCandidate {
                ordinal,
                key: AssetKey {
                    source_id: "fixture".into(),
                    asset_id: format!("{ordinal:064x}"),
                },
                rating: "g".into(),
                year: None,
                basis: "test".into(),
                content_version: "frozen".into(),
                bytes: 10,
                exposures: 0,
                protected: false,
                disposition: Default::default(),
                disposition_reason: None,
                blocked: false,
                blocking_batch: None,
            },
            image_sha256: None,
        })
        .collect();
    for m in &members {
        old.execute("INSERT INTO candidates(stage_id,ordinal,source_id,asset_id,rating,basis,content_version,bytes,sort_key,blocked) VALUES(?1,?2,'fixture',?3,'g','test','frozen',10,?3,1)",params![id,m.candidate.ordinal as i64,m.candidate.key.asset_id]).unwrap();
    }
    let attempt = new_id();
    old.execute("INSERT INTO batches(sequence,stage_id,rating,state,members,attempt_id) VALUES(50,?1,'g','outcome_unknown',?2,?3)",params![id,encode(&members).unwrap(),attempt]).unwrap();
    old.execute("INSERT INTO attempts(id,batch,state,created_at,failure) VALUES(?1,50,'outcome_unknown','1001',?2)",params![attempt,encode(&unknown()).unwrap()]).unwrap();
    drop(old);
    let migrated = EvaluationDb::open(&path).unwrap();
    let batch = migrated.batches(&id, 0, 10).unwrap().remove(0);
    assert_eq!(
        (batch.sequence, batch.stage_sequence, batch.attempt_count),
        (50, 1, 1)
    );
    assert_eq!(migrated.blocked_candidates(&id, None).unwrap().len(), 16);
    assert_eq!(migrated.stage(&id).unwrap().unknown, 1);
    assert!(migrated.stage(&id).unwrap().execution_settings.is_none());
    assert_eq!(migrated.attempts(&id, 50).unwrap()[0].id, attempt);
}

#[test]
fn image_input_changes_preserve_each_attempt_and_the_frozen_stage() {
    let (dir, db, id) = fixture(16);
    let frozen = encode(&db.stage(&id).unwrap().config).unwrap();
    enable(
        &db,
        &id,
        AestheticExecutionPolicy {
            image_max_edge: Some(1536),
            ..Default::default()
        },
    );
    let mut batch = db.claim(&id).unwrap().unwrap();
    for member in &mut batch.members {
        member.image_sha256 = Some("a".repeat(64));
    }
    let images = |edge, digest: &str| AestheticImageInputs {
        request_bytes: 10000,
        images: batch
            .members
            .iter()
            .map(|member| AestheticImageInput {
                label: member.label.clone(),
                image: studio_domain::ImageInputInfo {
                    source_sha256: "a".repeat(64),
                    sha256: digest.repeat(64),
                    source_width: Some(2048),
                    source_height: Some(1536),
                    width: Some(edge),
                    height: Some(edge * 3 / 4),
                    source_bytes: 1000,
                    bytes: 500,
                    content_type: "image/jpeg".into(),
                    max_edge: Some(edge),
                    transform_version: "test-encoder".into(),
                },
            })
            .collect(),
    };
    let first = new_id();
    assert_eq!(
        db.begin_attempt(
            &id,
            batch.sequence,
            batch.members.clone(),
            first.clone(),
            "f".repeat(64),
            Some(images(1024, "b"))
        )
        .unwrap_err()
        .code,
        "INVALID_INPUT"
    );
    assert!(
        db.begin_attempt(
            &id,
            batch.sequence,
            batch.members.clone(),
            first.clone(),
            "f".repeat(64),
            Some(images(1536, "b"))
        )
        .unwrap()
    );
    db.fail_attempt(&id, &first, unknown()).unwrap();
    db.schedule_recovery(&id, batch.sequence, unknown())
        .unwrap();
    db.control(&id, "pause").unwrap();
    db.settle(&id, None).unwrap();
    let old_attempt = encode(&db.attempts(&id, batch.sequence).unwrap()[0]).unwrap();
    db.configure_execution(
        &id,
        AestheticExecutionUpdate {
            idempotency_key: new_id(),
            expected_revision: 1,
            policy: AestheticExecutionPolicy {
                image_max_edge: Some(1024),
                ..Default::default()
            },
        },
        1,
        1,
    )
    .unwrap();
    db.retry_batch(&id, batch.sequence).unwrap();
    db.control(&id, "start").unwrap();
    let retry = db.claim(&id).unwrap().unwrap();
    assert_eq!(retry.sequence, batch.sequence);
    let second = new_id();
    assert!(
        db.begin_attempt(
            &id,
            batch.sequence,
            retry.members.clone(),
            second.clone(),
            "e".repeat(64),
            Some(images(1024, "c"))
        )
        .unwrap()
    );
    db.receive(&id, &second, receipt(&retry)).unwrap();
    db.parse_received(&id).unwrap();
    db.settle(&id, None).unwrap();
    assert_eq!(encode(&db.stage(&id).unwrap().config).unwrap(), frozen);
    drop(db);
    let reopened = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    let attempts = reopened.attempts(&id, batch.sequence).unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        encode(attempts.iter().find(|a| a.id == first).unwrap()).unwrap(),
        old_attempt
    );
    let new = attempts.iter().find(|a| a.id == second).unwrap();
    assert_eq!(
        new.execution_settings
            .as_ref()
            .unwrap()
            .policy
            .image_max_edge,
        Some(1024)
    );
    assert_eq!(
        new.image_inputs.as_ref().unwrap().images[0].image.width,
        Some(1024)
    );
    assert_eq!(reopened.stage(&id).unwrap().accepted, 1);
}

#[test]
fn image_input_v11_migration_preserves_missing_history_and_backs_up_once() {
    let (dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&id).unwrap();
    let frozen = encode(&db.stage(&id).unwrap().config).unwrap();
    drop(db);
    let path = dir.path().join("evaluation.sqlite");
    let old = Connection::open(&path).unwrap();
    old.execute_batch("ALTER TABLE attempts DROP COLUMN image_inputs; PRAGMA user_version=11;")
        .unwrap();
    drop(old);
    let migrated = EvaluationDb::open(&path).unwrap();
    assert_eq!(migrated.stage(&id).unwrap().accepted, 1);
    assert_eq!(
        encode(&migrated.stage(&id).unwrap().config).unwrap(),
        frozen
    );
    assert!(
        migrated.attempts(&id, batch.sequence).unwrap()[0]
            .image_inputs
            .is_none()
    );
    assert_eq!(
        migrated
            .read()
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        12
    );
    drop(migrated);
    let backup_paths: Vec<_> = std::fs::read_dir(dir.path().join(".backups"))
        .unwrap()
        .map(|v| v.unwrap().path())
        .collect();
    assert_eq!(backup_paths.len(), 1);
    let backup = Connection::open(&backup_paths[0]).unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        11
    );
    drop(backup);
    drop(EvaluationDb::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(dir.path().join(".backups"))
            .unwrap()
            .count(),
        1
    );
}
