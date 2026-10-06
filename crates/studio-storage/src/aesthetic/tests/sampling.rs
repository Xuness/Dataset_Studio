use super::*;
use studio_application::aesthetic_analysis::estimator::mix;

fn policy(mode: &str, min: u32, max: u32) -> AestheticSamplingPolicy {
    AestheticSamplingPolicy {
        mode: mode.into(),
        min_exposures: min,
        max_exposures: max,
        rank_tolerance: 0.08,
        seed: 17,
    }
}
fn configure(
    db: &EvaluationDb,
    id: &str,
    policy: AestheticSamplingPolicy,
    calls: u32,
) -> AestheticSamplingRequest {
    if db.stage(id).unwrap().state == "running" {
        db.control(id, "pause").unwrap();
        db.settle(id, None).unwrap();
    }
    let request = AestheticSamplingRequest {
        idempotency_key: new_id(),
        policy,
        additional_calls: calls,
    };
    db.configure_sampling(id, request.clone()).unwrap();
    db.control(id, "start").unwrap();
    request
}
fn judge(db: &EvaluationDb, id: &str, mut batch: AestheticBatch, noisy: bool) {
    for m in &mut batch.members {
        m.image_sha256 = Some("a".repeat(64));
    }
    let attempt = new_id();
    assert!(
        db.begin_attempt(
            id,
            batch.sequence,
            batch.members.clone(),
            attempt.clone(),
            "b".repeat(64),
            None,
        )
        .unwrap()
    );
    let round = batch.sampling.as_ref().map_or(0, |s| s.round);
    let mut ranked = batch.members.clone();
    ranked.sort_by_key(|m| {
        let i = m.candidate.ordinal as i64;
        let score = if noisy && (32..112).contains(&i) {
            32 + (mix(i as u64 ^ (u64::from(round) * 731)) % 80) as i64
        } else {
            i
        };
        std::cmp::Reverse((score, i))
    });
    let observation = AestheticObservation {
        schema_version: 1,
        tiers: ranked.iter().map(|m| vec![m.label.clone()]).collect(),
        elite_candidates: vec![],
        unjudgeable: vec![],
    };
    let mut result = receipt(&batch);
    result.outputs[0].content = vec![LlmContent::Text {
        text: encode(&observation).unwrap(),
    }];
    db.receive(id, &attempt, result).unwrap();
    assert_eq!(db.parse_received(id).unwrap(), 1);
    assert_eq!(db.parse_received(id).unwrap(), 0);
}
fn wave(db: &EvaluationDb, id: &str, noisy: bool) -> bool {
    if !db.plan_sampling(id, &|| Ok(())).unwrap() {
        return false;
    }
    let mut batches = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    while let Some(batch) = db.claim(id).unwrap() {
        for m in &batch.members {
            assert!(
                ids.insert(m.candidate.ordinal),
                "an image repeated within a concurrent round"
            );
        }
        batches.push(batch);
    }
    assert!(!batches.is_empty());
    // Complete in reverse dispatch order, as with asynchronous providers.
    for batch in batches.into_iter().rev() {
        judge(db, id, batch, noisy);
    }
    true
}
fn finish(db: &EvaluationDb, id: &str, noisy: bool) {
    for _ in 0..40 {
        if !wave(db, id, noisy) {
            db.settle(id, None).unwrap();
            return;
        }
    }
    panic!("sampling failed to stop");
}

#[test]
fn frozen_rounds_mix_145_candidates_despite_concurrent_reservations_and_restart() {
    let (dir, db, id) = fixture(145);
    configure(&db, &id, policy("balanced", 4, 6), 48);
    assert!(wave(&db, &id, false));
    assert!(db.plan_sampling(&id, &|| Ok(())).unwrap());
    let status = db.stage(&id).unwrap().sampling.unwrap();
    assert_eq!(status.round, 2);
    drop(db);
    let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    assert_eq!(db.stage(&id).unwrap().state, "paused");
    assert_eq!(db.stage(&id).unwrap().sampling.unwrap().round, 2);
    db.control(&id, "start").unwrap();
    finish(&db, &id, false);
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.state, "completed");
    let status = stage.sampling.unwrap();
    assert_eq!((status.covered, status.components), (145, 1));
    assert!(stage.attempts <= 48);
    let exposure: Vec<u32> = db
        .read()
        .unwrap()
        .prepare("SELECT exposures FROM candidates WHERE stage_id=?1")
        .unwrap()
        .query_map([&id], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(exposure.iter().all(|v| (4..=6).contains(v)));
    assert!(
        db.sampling_diagnostic(&id, 0)
            .unwrap()
            .unwrap()
            .distinct_opponents
            >= 24
    );
}

#[test]
fn adaptive_exposure_focuses_on_uncertain_middle_and_retains_stable_extremes() {
    let (_dir, db, id) = fixture(145);
    configure(&db, &id, policy("adaptive", 2, 8), 100);
    finish(&db, &id, true);
    let connection = db.read().unwrap();
    let (low,middle,high):(f64,f64,f64)=connection.query_row("SELECT AVG(CASE WHEN ordinal<32 THEN exposures END),AVG(CASE WHEN ordinal>=32 AND ordinal<112 THEN exposures END),AVG(CASE WHEN ordinal>=112 THEN exposures END) FROM candidates WHERE stage_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(
        low < middle && high < middle,
        "stable low={low}, middle={middle}, stable high={high}"
    );
    let limits:(u32,u32,u32)=connection.query_row("SELECT MIN(exposures),MAX(exposures),SUM(disposition='excluded') FROM candidates WHERE stage_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(limits.0 >= 2 && limits.1 <= 8);
    assert_eq!(limits.2, 0);
    let status = db.stage(&id).unwrap().sampling.unwrap();
    assert_eq!(status.components, 1);
    assert!(status.stable > 0 && status.stable < 145);
    assert_eq!(status.state, "limited");
    assert_eq!(db.stage(&id).unwrap().state, "needs_attention");
}

#[test]
fn refinement_revision_survives_restart_and_preserves_paid_evidence() {
    let (dir, db, id) = fixture(145);
    configure(&db, &id, policy("balanced", 4, 6), 48);
    assert!(wave(&db, &id, false));
    db.control(&id, "pause").unwrap();
    db.settle(&id, None).unwrap();
    let before = db.stage(&id).unwrap();
    let request = AestheticSamplingRequest {
        idempotency_key: new_id(),
        policy: policy("refine", 4, 8),
        additional_calls: 45,
    };
    let next = db.configure_sampling(&id, request.clone()).unwrap();
    assert_eq!(next.attempts, before.attempts);
    assert_eq!(next.accepted, before.accepted);
    assert_eq!(next.config_hash, before.config_hash);
    assert_eq!(
        next.sampling.as_ref().unwrap().version,
        "neighbor_budget_v2"
    );
    db.configure_sampling(&id, request).unwrap();
    drop(db);
    let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    db.control(&id, "start").unwrap();
    finish(&db, &id, true);
    let done = db.stage(&id).unwrap();
    assert_eq!(done.state, "needs_attention");
    assert_eq!(done.attempts, before.attempts + 45);
    assert_eq!(done.sampling.unwrap().stable, 0);
    assert!(
        db.sampling_diagnostic(&id, 0)
            .unwrap()
            .unwrap()
            .rank_sensitivity
            .is_some()
    );
    let count:u64=db.read().unwrap().query_row("SELECT COUNT(*) FROM evidence e JOIN batches b ON b.sequence=e.batch WHERE b.stage_id=?1",[&id],|r|crate::unsigned(r,0)).unwrap();
    assert_eq!(count, done.accepted);
}

#[test]
fn interrupted_staging_is_invisible_to_dispatch_and_can_resume_after_restart() {
    let (dir, db, id) = fixture(1024);
    configure(&db, &id, policy("balanced", 2, 8), 128);
    let result = db.plan_sampling(&id, &|| {
        let count: i64 = db
            .read()?
            .query_row("SELECT COUNT(*) FROM sampling_queue", [], |r| r.get(0))
            .map_err(db_error)?;
        if count > 0 {
            Err(Error::new("CANCELLED", "after staged slots"))
        } else {
            Ok(())
        }
    });
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    assert_eq!(db.stage(&id).unwrap().sampling.unwrap().round, 0);
    assert!(db.claim(&id).unwrap().is_none());
    assert_eq!(db.stage(&id).unwrap().attempts, 0);
    drop(db);
    let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    db.control(&id, "start").unwrap();
    assert!(db.plan_sampling(&id, &|| Ok(())).unwrap());
    let mut seen = std::collections::BTreeSet::new();
    while let Some(batch) = db.claim(&id).unwrap() {
        for m in batch.members {
            assert!(seen.insert(m.candidate.ordinal));
        }
    }
    assert_eq!(seen.len(), 1024);
    assert_eq!(db.stage(&id).unwrap().attempts, 0);
}

#[test]
fn replacing_a_cancelled_plan_cleans_only_unpublished_staging() {
    let (_dir, db, id) = fixture(32);
    configure(&db, &id, policy("balanced", 4, 8), 16);
    assert!(wave(&db, &id, false));
    let old = db.stage(&id).unwrap().sampling.unwrap().plan_id;
    let published: String = db
        .read()
        .unwrap()
        .query_row(
            "SELECT summary_json FROM sampling_rounds WHERE plan_id=?1 AND round=1",
            [&old],
            |r| r.get(0),
        )
        .unwrap();
    let result = db.plan_sampling(&id, &|| {
        let exists: bool = db
            .read()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sampling_queue WHERE batch IS NULL)",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if exists {
            Err(Error::new("CANCELLED", "staged"))
        } else {
            Ok(())
        }
    });
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    configure(&db, &id, policy("refine", 2, 8), 16);
    assert!(db.plan_sampling(&id, &|| Ok(())).unwrap());
    let remaining: i64 = db
        .read()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sampling_rounds WHERE plan_id=?1",
            [&old],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 1);
    let retained: String = db
        .read()
        .unwrap()
        .query_row(
            "SELECT summary_json FROM sampling_rounds WHERE plan_id=?1 AND round=1",
            [old],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, published);
    let final_stage = db.stage(&id).unwrap();
    assert_eq!(final_stage.attempts, 2);
    assert_eq!(final_stage.accepted, 2);
    assert!(db.claim(&id).unwrap().is_some());
}

#[test]
fn large_round_pages_cross_the_old_cutoff_and_the_writer_payload_limit() {
    let count = 200_000;
    let (_dir, db, id) = fixture(1);
    // Seed real rows in bulk; this test measures publication, not freeze throughput.
    let stage_id = id.clone();
    db.writer.submit(4096,move|connection|{
        connection.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?2) INSERT INTO candidates(stage_id,ordinal,source_id,asset_id,rating,year,basis,content_version,bytes,sort_key) SELECT ?1,x,'fixture',printf('%064x',x),'g',2020,'test','v1',10,printf('%064x',x) FROM n",params![stage_id,count as i64-1]).map_err(db_error)?;
        connection.execute("UPDATE stages SET total=?2,frozen=?2,eligible=?2,unresolved=?2 WHERE id=?1",params![stage_id,count as i64]).map_err(db_error)?;
        Ok(())
    }).unwrap();
    configure(&db, &id, policy("refine", 2, 8), 1);
    let stage = db.stage(&id).unwrap();
    let status = stage.sampling.clone().unwrap();
    let input = studio_domain::aesthetic_analysis::AestheticAnalysisInput {
        stage_id: id.clone(),
        stage_config_hash: stage.config_hash.clone(),
        candidates: count,
        observations: 0,
        evidence_watermark: 0,
        review_watermark: 0,
    };
    let mut next = status.clone();
    next.round = 1;
    next.state = "dispatching".into();
    next.eligible = count;
    next.unresolved = count;
    let diagnostics: Vec<_> = (0..count)
        .map(|ordinal| AestheticSamplingDiagnostic {
            ordinal,
            exposures: 0,
            distinct_opponents: 0,
            component: None,
            component_size: 0,
            percentile: None,
            rank_delta: None,
            rank_sensitivity: Some(1.0),
            stable_rounds: 0,
            reason: "coverage".into(),
        })
        .collect();
    assert!(encode(&diagnostics).unwrap().len() > 32 << 20);
    let batches = vec![
        (count - 16..count)
            .map(|ordinal| AestheticSamplingMemberReason {
                ordinal,
                reason: "coverage".into(),
            })
            .collect(),
    ];
    assert!(
        super::super::sampling::publish::round(
            &db,
            &stage,
            &status,
            &input,
            studio_application::aesthetic::sampling::Round {
                status: next,
                diagnostics,
                batches
            },
            &|| Ok(())
        )
        .unwrap()
    );
    let ready = db.stage(&id).unwrap().sampling.unwrap();
    {
        let connection = db.read().unwrap();
        let available =
            super::super::sampling::read::available(&connection, &id, count, &|| Ok(())).unwrap();
        assert_eq!(available.len(), count as usize);
        assert_eq!(available.last(), Some(&(count - 1)));
        let rows =
            super::super::sampling::read::diagnostics(&connection, &ready, count, &|| Ok(()))
                .unwrap();
        assert_eq!(rows.len(), count as usize);
        assert_eq!(rows.last().unwrap().ordinal, count - 1);
    }
    assert!(db.writer.stats.lock().unwrap().peak < 32 << 20);
    let batch = db.claim(&id).unwrap().unwrap();
    assert!(
        batch
            .members
            .iter()
            .all(|m| m.candidate.ordinal >= count - 16)
    );
    assert_eq!(db.stage(&id).unwrap().attempts, 0);
}

fn exact_batch(db: &EvaluationDb, id: &str, ordinals: Vec<u64>) -> AestheticBatch {
    let members: Vec<_> = ordinals
        .iter()
        .enumerate()
        .map(|(i, n)| AestheticMember {
            label: format!("img{:02}", i + 1),
            candidate: db.candidate(id, *n).unwrap(),
            image_sha256: None,
        })
        .collect();
    let id = id.to_owned();
    db.writer
        .submit(32768, move |db| {
            for m in &members {
                db.execute(
                    "UPDATE candidates SET reserved=1 WHERE stage_id=?1 AND ordinal=?2",
                    params![id, m.candidate.ordinal as i64],
                )
                .map_err(db_error)?;
            }
            db.execute(
                "INSERT INTO batches(stage_id,rating,state,members) VALUES(?1,'g','preparing',?2)",
                params![id, encode(&members)?],
            )
            .map_err(db_error)?;
            read_batch(db, &id, db.last_insert_rowid() as u64)
        })
        .unwrap()
}

#[test]
fn supplemental_plan_bridges_129_plus_16_without_replacing_paid_evidence() {
    let (_dir, db, id) = fixture(145);
    for _ in 0..4 {
        for start in (0..129).step_by(15) {
            judge(
                &db,
                &id,
                exact_batch(&db, &id, (start..(start + 16).min(129)).collect()),
                false,
            );
        }
        judge(&db, &id, exact_batch(&db, &id, (129..145).collect()), false);
    }
    db.settle(&id, None).unwrap();
    let before = db.stage(&id).unwrap();
    let old = db
        .analysis_create(super::analysis::fit_request(&id))
        .unwrap();
    super::analysis::complete(&db, &old.id);
    let old_json = encode(&db.analysis_job(&old.id).unwrap()).unwrap();
    let request = configure(&db, &id, policy("balanced", 4, 10), 4);
    db.configure_sampling(&id, request.clone()).unwrap(); // exact retry cannot reset/start a plan
    let mut conflict = request;
    conflict.additional_calls += 1;
    assert_eq!(
        db.configure_sampling(&id, conflict).unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    finish(&db, &id, false);
    let after = db.stage(&id).unwrap();
    assert_eq!(after.sampling.as_ref().unwrap().components, 1);
    assert_eq!(after.state, "completed");
    assert!(after.attempts - before.attempts <= 2);
    assert_eq!(after.config_hash, before.config_hash);
    assert_eq!(
        after.accepted - before.accepted,
        after.attempts - before.attempts
    );
    assert_eq!(
        encode(&db.analysis_job(&old.id).unwrap()).unwrap(),
        old_json
    );
}

#[test]
fn budget_exhaustion_and_cancel_do_not_masquerade_as_sampling_completion() {
    let (_dir, db, id) = fixture(16);
    configure(&db, &id, policy("adaptive", 1, 8), 1);
    let err = db
        .plan_sampling(&id, &|| Err(Error::new("CANCELLED", "test")))
        .unwrap_err();
    assert_eq!(err.code, "CANCELLED");
    assert_eq!(db.stage(&id).unwrap().attempts, 0);
    assert!(db.batches(&id, 0, 64).unwrap().is_empty());
    finish(&db, &id, false);
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.attempts, 1);
    assert_eq!(stage.unresolved, 0); // legacy exposure requirement is satisfied
    assert_eq!(stage.state, "needs_attention"); // adaptive requirement is not
    assert_eq!(
        stage.sampling.unwrap().reason.as_deref(),
        Some("call_budget")
    );
}

#[test]
fn sampling_budget_can_be_extended_without_automatically_retrying_unknown_outcomes() {
    let (_dir, db, id) = fixture(16);
    let (_batch, _attempt) = sent(&db, &id);
    db.control(&id, "pause").unwrap();
    db.settle(&id, None).unwrap();
    let request = AestheticSamplingRequest {
        idempotency_key: new_id(),
        policy: policy("adaptive", 2, 8),
        additional_calls: 4,
    };
    db.configure_sampling(&id, request).unwrap();
    db.control(&id, "start").unwrap();
    assert!(db.claim(&id).unwrap().is_none());
    assert!(!db.plan_sampling(&id, &|| Ok(())).unwrap());
    assert_eq!(db.stage(&id).unwrap().attempts, 1);
    assert_eq!(db.stage(&id).unwrap().unknown, 1);
    assert_eq!(
        db.stage(&id).unwrap().sampling.unwrap().reason.as_deref(),
        Some("pending_outcomes")
    );
    assert_eq!(db.settle(&id, None).unwrap().state, "needs_attention");
    assert_eq!(
        studio_application::aesthetic::sampling::validate(&policy("adaptive", 2, 8), 10_000_001)
            .unwrap_err()
            .code,
        "EVALUATION_SAMPLING_CAPACITY"
    );
}

#[test]
fn pause_during_planning_cannot_publish_a_partial_or_stale_round() {
    let (_dir, db, id) = fixture(145);
    configure(&db, &id, policy("adaptive", 2, 8), 48);
    let ticks = std::cell::Cell::new(0);
    let result = db.plan_sampling(&id, &|| {
        ticks.set(ticks.get() + 1);
        if ticks.get() == 8 {
            db.control(&id, "pause")?;
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    assert!(ticks.get() >= 8);
    assert!(db.batches(&id, 0, 64).unwrap().is_empty());
    assert_eq!(db.stage(&id).unwrap().sampling.unwrap().round, 0);
    assert_eq!(db.stage(&id).unwrap().attempts, 0);
    assert_eq!(db.settle(&id, None).unwrap().state, "paused");
}

#[test]
fn supplemental_budget_reuses_compatible_stability_instead_of_retesting_every_image() {
    for count in [2, 16] {
        let (_dir, db, id) = fixture(count);
        configure(&db, &id, policy("balanced", 3, 8), 10);
        finish(&db, &id, false);
        let before = db.stage(&id).unwrap();
        let stable = before.sampling.as_ref().unwrap().stable;
        assert!(stable > 0);
        configure(&db, &id, policy("adaptive", 3, 8), 10);
        let planned = db.plan_sampling(&id, &|| Ok(())).unwrap();
        let after = db.stage(&id).unwrap();
        assert_eq!(after.attempts, before.attempts);
        assert_eq!(after.sampling.as_ref().unwrap().stable, stable);
        assert_eq!(
            after.sampling.unwrap().previous_plan_id,
            before.sampling.map(|p| p.plan_id)
        );
        if stable == count {
            assert!(!planned);
            assert_eq!(db.settle(&id, None).unwrap().state, "completed");
        } else {
            assert!(planned);
            let batch = db.claim(&id).unwrap().unwrap();
            for member in batch.sampling.unwrap().members {
                if db
                    .sampling_diagnostic(&id, member.ordinal)
                    .unwrap()
                    .unwrap()
                    .stable_rounds
                    >= 2
                {
                    assert!(matches!(member.reason.as_str(), "anchor" | "exploration"));
                }
            }
        }
    }
}

#[test]
fn transient_abstention_keeps_adaptive_coverage_running() {
    let (_dir, db, id) = fixture(16);
    configure(&db, &id, policy("adaptive", 2, 8), 100);
    assert!(db.plan_sampling(&id, &|| Ok(())).unwrap());
    let (batch, attempt) = sent(&db, &id);
    let observation = AestheticObservation {
        schema_version: 1,
        tiers: vec![batch.members[1..].iter().map(|m| m.label.clone()).collect()],
        elite_candidates: vec![],
        unjudgeable: vec![AestheticUnjudgeable {
            id: batch.members[0].label.clone(),
            reason: "fixture abstention".into(),
        }],
    };
    let mut response = receipt(&batch);
    response.outputs[0].content = vec![LlmContent::Text {
        text: encode(&observation).unwrap(),
    }];
    db.receive(&id, &attempt, response).unwrap();
    db.parse_received(&id).unwrap();
    finish(&db, &id, false);
    let stage = db.stage(&id).unwrap();
    assert!(stage.attempts >= 3);
    assert_eq!(stage.state, "completed");
    let status = stage.sampling.unwrap();
    assert_eq!(status.covered, 16);
}

#[test]
fn confirmed_abstention_still_stops_incomplete_adaptive_ranking() {
    let (_dir, db, id) = fixture(16);
    configure(&db, &id, policy("adaptive", 2, 8), 100);
    for _ in 0..3 {
        assert!(db.plan_sampling(&id, &|| Ok(())).unwrap());
        let (batch, attempt) = sent(&db, &id);
        db.receive(&id, &attempt, abstaining_receipt(&batch, &[0]))
            .unwrap();
        db.parse_received(&id).unwrap();
    }
    finish(&db, &id, false);
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.state, "needs_attention");
    assert_eq!(
        db.candidate(&id, 0).unwrap().disposition,
        AestheticDisposition::NeedsReview
    );
    let status = stage.sampling.unwrap();
    assert_eq!(status.covered, 15);
    assert_eq!(status.reason.as_deref(), Some("ranking_scope_incomplete"));
}

#[test]
fn isolated_rating_candidates_have_an_explicit_disposition_and_all_excluded_exit() {
    let (_dir, db, id) = fixture(2);
    let sid = id.clone();
    db.writer
        .submit(1024, move |db| {
            db.execute(
                "UPDATE candidates SET rating='s' WHERE stage_id=?1 AND ordinal=1",
                [sid],
            )
            .map_err(db_error)?;
            Ok(())
        })
        .unwrap();
    configure(&db, &id, policy("adaptive", 1, 4), 10);
    finish(&db, &id, false);
    assert_eq!(db.stage(&id).unwrap().state, "needs_attention");
    for ordinal in 0..2 {
        let candidate = db.candidate(&id, ordinal).unwrap();
        assert_eq!(candidate.disposition, AestheticDisposition::NeedsReview);
        assert_eq!(
            candidate.disposition_reason.as_deref(),
            Some("no_comparison_peer")
        );
        db.decide_candidate(
            &id,
            ordinal,
            AestheticCandidateDecision {
                idempotency_key: new_id(),
                action: AestheticDispositionAction::Exclude,
                reason: "singleton Rating explicitly excluded".into(),
            },
        )
        .unwrap();
    }
    db.control(&id, "start").unwrap();
    finish(&db, &id, false);
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.state, "completed_with_exclusions");
    assert_eq!(stage.excluded, 2);
    assert_eq!(stage.attempts, 0);
    assert_eq!(
        stage.sampling.unwrap().reason.as_deref(),
        Some("no_remaining_candidates")
    );
}

#[test]
fn exclusion_can_finalize_a_sampling_stage_with_no_remaining_call_budget() {
    let (_dir, db, id) = fixture(16);
    configure(&db, &id, policy("balanced", 1, 3), 3);
    let ordinal = 0;
    for _ in 0..3 {
        assert!(db.plan_sampling(&id, &|| Ok(())).unwrap());
        let (batch, attempt) = sent(&db, &id);
        db.receive(&id, &attempt, abstaining_receipt(&batch, &[ordinal]))
            .unwrap();
        db.parse_received(&id).unwrap();
    }
    finish(&db, &id, false);
    assert_eq!(db.stage(&id).unwrap().state, "needs_attention");
    db.decide_candidate(
        &id,
        ordinal,
        AestheticCandidateDecision {
            idempotency_key: new_id(),
            action: AestheticDispositionAction::Exclude,
            reason: "explicit exclusion after exhausted budget".into(),
        },
    )
    .unwrap();
    db.control(&id, "start").unwrap();
    assert!(db.claim(&id).unwrap().is_none());
    finish(&db, &id, false);
    let stage = db.stage(&id).unwrap();
    assert_eq!(stage.state, "completed_with_exclusions");
    assert_eq!(stage.attempts, 3);
    assert_eq!(stage.excluded, 1);
}

#[test]
fn a_single_judged_image_requires_batch_retry_without_accepting_evidence() {
    let (_dir, db, id) = fixture(2);
    configure(&db, &id, policy("balanced", 1, 1), 3);
    db.plan_sampling(&id, &|| Ok(())).unwrap();
    let (batch, attempt) = sent(&db, &id);
    let observation = AestheticObservation {
        schema_version: 1,
        tiers: vec![vec![batch.members[0].label.clone()]],
        elite_candidates: vec![],
        unjudgeable: vec![AestheticUnjudgeable {
            id: batch.members[1].label.clone(),
            reason: "fixture abstention".into(),
        }],
    };
    let mut response = receipt(&batch);
    response.outputs[0].content = vec![LlmContent::Text {
        text: encode(&observation).unwrap(),
    }];
    db.receive(&id, &attempt, response).unwrap();
    db.parse_received(&id).unwrap();
    finish(&db, &id, false);
    assert_eq!(db.stage(&id).unwrap().accepted, 0);
    for ordinal in 0..2 {
        assert_eq!(db.candidate(&id, ordinal).unwrap().exposures, 0);
        assert_eq!(
            db.candidate(&id, ordinal).unwrap().disposition,
            AestheticDisposition::Active
        );
    }
    db.retry_batch(&id, batch.sequence).unwrap();
    db.control(&id, "start").unwrap();
    let (retry, attempt) = sent(&db, &id);
    assert_eq!(retry.sequence, batch.sequence);
    db.receive(&id, &attempt, receipt(&retry)).unwrap();
    db.parse_received(&id).unwrap();
    finish(&db, &id, false);
    assert_eq!(db.stage(&id).unwrap().state, "completed");
    assert_eq!(db.stage(&id).unwrap().attempts, 2);
    assert_eq!(db.candidate(&id, 0).unwrap().exposures, 1);
    assert_eq!(db.candidate(&id, 1).unwrap().exposures, 1);
}
