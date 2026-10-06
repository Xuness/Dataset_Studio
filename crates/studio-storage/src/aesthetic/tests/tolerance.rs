use super::execution::{due, enable};
use super::*;

fn streak(db: &EvaluationDb, id: &str, ordinal: u64) -> u32 {
    db.read()
        .unwrap()
        .query_row(
            "SELECT unjudgeable_streak FROM candidates WHERE stage_id=?1 AND ordinal=?2",
            params![id, ordinal as i64],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn noncomparative_receipts_retry_durably_without_candidate_strikes_or_evidence() {
    for judged in [0, 1] {
        let (dir, mut db, id) = fixture(16);
        enable(
            &db,
            &id,
            AestheticExecutionPolicy {
                max_retries: 1,
                ..Default::default()
            },
        );
        let (batch, attempt) = sent(&db, &id);
        let ordinals: Vec<_> = batch.members[judged..]
            .iter()
            .map(|m| m.candidate.ordinal)
            .collect();
        let response = abstaining_receipt(&batch, &ordinals);
        db.receive(&id, &attempt, response.clone()).unwrap();
        if judged == 1 {
            drop(db);
            db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
        }
        assert_eq!(db.parse_received(&id).unwrap(), 0);
        db.receive(&id, &attempt, response).unwrap();
        assert_eq!(db.parse_received(&id).unwrap(), 0);
        let saved = db.batches(&id, 0, 10).unwrap().remove(0);
        assert_eq!(saved.state, "retry_wait");
        assert!(saved.observation.is_none());
        assert_eq!(
            saved.last_failure.unwrap().code,
            "EVALUATION_NO_COMPARABLE_EVIDENCE"
        );
        let stage = db.stage(&id).unwrap();
        assert_eq!(
            (
                stage.accepted,
                stage.invalid,
                stage.unknown,
                stage.input_tokens
            ),
            (0, 0, 0, 17)
        );
        for c in db.candidate_page(&id, None, false).unwrap() {
            assert_eq!(c.disposition, AestheticDisposition::Active);
            assert!(!c.blocked);
            assert_eq!((c.exposures, streak(&db, &id, c.ordinal)), (0, 0));
        }
        assert_eq!(
            db.read()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        drop(db);
        let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
        assert_eq!(db.stage(&id).unwrap().state, "paused");
        assert!(db.claim(&id).unwrap().is_none());
        due(&db, batch.sequence);
        db.control(&id, "start").unwrap();
        let (retry, second) = sent(&db, &id);
        assert_eq!(retry.sequence, batch.sequence);
        db.receive(&id, &second, receipt(&retry)).unwrap();
        assert_eq!(db.parse_received(&id).unwrap(), 1);
        let stage = db.settle(&id, None).unwrap();
        assert_eq!(stage.state, "completed");
        assert_eq!(
            (
                stage.attempts,
                stage.accepted,
                stage.progress.blocked,
                stage.unresolved
            ),
            (2, 1, 0, 0)
        );
        let attempts = db.attempts(&id, batch.sequence).unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].state, "failed");
        assert!(attempts[0].receipt.is_some());
        assert!(!attempts[0].failure.as_ref().unwrap().outcome_unknown);
    }
}

#[test]
fn noncomparative_exhaustion_obeys_pause_or_defer_without_quality_dispositions() {
    for exhausted in ["pause", "defer"] {
        let (_dir, db, id) = fixture(16);
        enable(
            &db,
            &id,
            AestheticExecutionPolicy {
                max_retries: 1,
                exhausted: exhausted.into(),
                ..Default::default()
            },
        );
        let mut sequence = 0;
        for n in 0..2 {
            let (batch, attempt) = sent(&db, &id);
            if n == 0 {
                sequence = batch.sequence;
            } else {
                assert_eq!(batch.sequence, sequence);
            }
            let ordinals: Vec<_> = batch.members.iter().map(|m| m.candidate.ordinal).collect();
            db.receive(&id, &attempt, abstaining_receipt(&batch, &ordinals))
                .unwrap();
            db.parse_received(&id).unwrap();
            if n == 0 {
                due(&db, sequence);
            }
        }
        assert_eq!(db.stage(&id).unwrap().attempts, 2);
        assert_eq!(db.stage(&id).unwrap().accepted, 0);
        assert_eq!(
            db.batches(&id, 0, 10).unwrap()[0].state,
            if exhausted == "pause" {
                "failed"
            } else {
                "deferred"
            }
        );
        assert!(
            db.candidate_page(&id, None, false)
                .unwrap()
                .iter()
                .all(|c| c.disposition == AestheticDisposition::Active && c.exposures == 0)
        );
        if exhausted == "pause" {
            assert!(db.claim(&id).unwrap().is_none());
            assert_eq!(db.settle(&id, None).unwrap().state, "needs_attention");
            db.retry_batch(&id, sequence).unwrap();
            db.control(&id, "start").unwrap();
        }
        let (batch, attempt) = sent(&db, &id);
        assert_eq!(batch.sequence == sequence, exhausted == "pause");
        db.receive(&id, &attempt, receipt(&batch)).unwrap();
        db.parse_received(&id).unwrap();
        assert_eq!(db.settle(&id, None).unwrap().state, "completed");
    }
}

#[test]
fn noncomparative_failure_halts_after_committing_the_receipt() {
    let (_dir, db, id) = fixture(16);
    enable(
        &db,
        &id,
        AestheticExecutionPolicy {
            failure_halt_threshold: Some(1),
            ..Default::default()
        },
    );
    let (batch, attempt) = sent(&db, &id);
    let ordinals: Vec<_> = batch.members.iter().map(|m| m.candidate.ordinal).collect();
    db.receive(&id, &attempt, abstaining_receipt(&batch, &ordinals))
        .unwrap();
    assert_eq!(
        db.parse_received(&id).unwrap_err().code,
        "EVALUATION_REMOTE"
    );
    assert_eq!(db.parse_received(&id).unwrap(), 0);
    assert_eq!(db.batches(&id, 0, 10).unwrap()[0].state, "failed");
    assert!(
        db.attempts(&id, batch.sequence).unwrap()[0]
            .receipt
            .is_some()
    );
    assert_eq!(db.stage(&id).unwrap().attempts, 1);
    assert!(db.claim(&id).unwrap().is_none());
}

#[test]
fn noncomparative_retries_respect_disabled_retries_and_call_or_time_limits() {
    for limit in ["disabled", "calls", "deadline"] {
        let (_dir, db, id) = fixture(16);
        enable(
            &db,
            &id,
            AestheticExecutionPolicy {
                max_retries: if limit == "disabled" { 0 } else { 2 },
                ..Default::default()
            },
        );
        let (batch, attempt) = sent(&db, &id);
        let sid = id.clone();
        let sequence = batch.sequence;
        db.writer.submit(1024, move |tx| {
            if limit=="calls" {
                tx.execute("UPDATE stages SET config=json_set(config,'$.request.max_calls',1) WHERE id=?1", [sid]).map_err(db_error)?;
            } else if limit=="deadline" {
                tx.execute("UPDATE batches SET recovery_deadline=0 WHERE sequence=?1", [sequence as i64]).map_err(db_error)?;
            }
            Ok(())
        }).unwrap();
        let ordinals: Vec<_> = batch.members.iter().map(|m| m.candidate.ordinal).collect();
        db.receive(&id, &attempt, abstaining_receipt(&batch, &ordinals))
            .unwrap();
        db.parse_received(&id).unwrap();
        assert_eq!(db.batches(&id, 0, 10).unwrap()[0].state, "failed");
        assert!(db.claim(&id).unwrap().is_none());
        assert_eq!(db.stage(&id).unwrap().attempts, 1);
        assert_eq!(db.settle(&id, None).unwrap().state, "needs_attention");
    }
}

#[test]
fn first_abstention_is_resampled_and_success_resets_its_streak() {
    let (_dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, abstaining_receipt(&batch, &[0]))
        .unwrap();
    db.parse_received(&id).unwrap();
    let candidate = db.candidate(&id, 0).unwrap();
    assert_eq!(candidate.disposition, AestheticDisposition::Active);
    assert!(!candidate.blocked);
    assert_eq!((candidate.exposures, streak(&db, &id, 0)), (0, 1));
    let (next, attempt) = sent(&db, &id);
    assert_ne!(next.sequence, batch.sequence);
    assert!(next.members.iter().any(|m| m.candidate.ordinal == 0));
    db.receive(&id, &attempt, receipt(&next)).unwrap();
    db.parse_received(&id).unwrap();
    assert_eq!(streak(&db, &id, 0), 0);
    assert_eq!(db.settle(&id, None).unwrap().state, "completed");
}

#[test]
fn unseen_candidates_get_healthy_peers_and_require_three_distinct_batches() {
    let (dir, mut db, id) = fixture(16);
    let mut sequences = std::collections::BTreeSet::new();
    for n in 1..=3 {
        let (batch, attempt) = sent(&db, &id);
        assert!(sequences.insert(batch.sequence));
        assert!(
            batch.members.len() >= 4,
            "retries need comparable healthy peers"
        );
        let response = abstaining_receipt(&batch, &[0, 1]);
        db.receive(&id, &attempt, response.clone()).unwrap();
        assert_eq!(db.parse_received(&id).unwrap(), 1);
        db.receive(&id, &attempt, response).unwrap();
        assert_eq!(db.parse_received(&id).unwrap(), 0);
        assert_eq!(streak(&db, &id, 0), n);
        assert_eq!(streak(&db, &id, 1), n);
        let c = db.candidate(&id, 0).unwrap();
        assert_eq!(c.exposures, 0);
        assert_eq!(c.blocked, n == 3);
        assert_eq!(
            c.disposition,
            if n == 3 {
                AestheticDisposition::NeedsReview
            } else {
                AestheticDisposition::Active
            }
        );
        if n == 1 {
            drop(db);
            db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
            db.control(&id, "start").unwrap();
        }
    }
    assert!(db.claim(&id).unwrap().is_none());
    assert_eq!(db.settle(&id, None).unwrap().state, "needs_attention");
    db.decide_candidate(
        &id,
        0,
        AestheticCandidateDecision {
            idempotency_key: new_id(),
            action: AestheticDispositionAction::Rejudge,
            reason: "try repaired input".into(),
        },
    )
    .unwrap();
    assert_eq!(streak(&db, &id, 0), 0);
    db.control(&id, "start").unwrap();
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, abstaining_receipt(&batch, &[0]))
        .unwrap();
    db.parse_received(&id).unwrap();
    assert_eq!(
        db.candidate(&id, 0).unwrap().disposition,
        AestheticDisposition::Rejudge
    );
    assert!(!db.candidate(&id, 0).unwrap().blocked);
    assert_eq!(streak(&db, &id, 0), 1);
}

#[test]
fn historical_exposure_survives_repeated_abstentions_without_fabricating_more() {
    let (_dir, db, id) = fixture(16);
    let sid = id.clone();
    db.writer
        .submit(1024, move |tx| {
            tx.execute(
                "UPDATE stages SET config=json_set(config,'$.request.exposures',2) WHERE id=?1",
                [sid],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&id).unwrap();
    for _ in 0..4 {
        let (batch, attempt) = sent(&db, &id);
        db.receive(&id, &attempt, abstaining_receipt(&batch, &[0]))
            .unwrap();
        db.parse_received(&id).unwrap();
        let c = db.candidate(&id, 0).unwrap();
        assert_eq!(c.exposures, 1);
        assert_eq!(c.disposition, AestheticDisposition::Active);
        assert!(!c.blocked);
    }
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&id).unwrap();
    assert_eq!(streak(&db, &id, 0), 0);
    assert_eq!(db.candidate(&id, 0).unwrap().exposures, 2);
    assert_eq!(db.settle(&id, None).unwrap().state, "completed");
}

#[test]
fn v9_upgrade_preserves_history_and_dispositions_without_replaying_old_abstentions() {
    let (dir, db, id) = fixture(16);
    let (batch, attempt) = sent(&db, &id);
    db.receive(&id, &attempt, abstaining_receipt(&batch, &[0]))
        .unwrap();
    db.parse_received(&id).unwrap();
    let path = dir.path().join("evaluation.sqlite");
    let observation = db.batches(&id, 0, 10).unwrap()[0].observation.clone();
    drop(db);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("ALTER TABLE candidates DROP COLUMN unjudgeable_streak; UPDATE candidates SET blocked=1,disposition='needs_review' WHERE ordinal=0; PRAGMA user_version=9;").unwrap();
    drop(old);
    let db = EvaluationDb::open(&path).unwrap();
    assert_eq!(
        db.read()
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        10
    );
    assert_eq!(db.batches(&id, 0, 10).unwrap()[0].observation, observation);
    assert_eq!(db.stage(&id).unwrap().accepted, 1);
    assert_eq!(
        db.candidate(&id, 0).unwrap().disposition,
        AestheticDisposition::NeedsReview
    );
    assert_eq!(streak(&db, &id, 0), 0);
    assert_eq!(db.parse_received(&id).unwrap(), 0);
    let backups: Vec<_> = std::fs::read_dir(dir.path().join(".backups"))
        .unwrap()
        .map(|v| v.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(&backups[0]).unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        9
    );
}
