use super::*;
use studio_application::aesthetic_analysis::estimator::replay;
use studio_domain::aesthetic_analysis::*;
pub(super) fn fit_request(stage: &str) -> AestheticAnalysisCreate {
    AestheticAnalysisCreate {
        idempotency_key: new_id(),
        name: "重放".into(),
        spec: AestheticAnalysisSpec::Fit {
            config: AestheticFit {
                stage_id: stage.into(),
                estimator: AestheticEstimator {
                    kind: "davidson_v1".into(),
                    iterations: 32,
                    regularization: 0.1,
                    tie_strength: 1.0,
                },
                stability_seed: Some(7),
            },
            experiment_id: None,
            variant: None,
        },
    }
}
pub(super) fn complete(db: &EvaluationDb, id: &str) -> AestheticAnalysisJob {
    let job = db.analysis_start(id).unwrap();
    while db.analysis_reset_page(id).unwrap() {}
    let AestheticAnalysisSpec::Fit { config, .. } = &job.request.spec else {
        panic!("fit")
    };
    let summary = replay(
        db,
        &job.input,
        config,
        &|| Ok(()),
        &mut |_, _, _| Ok(()),
        &mut |rows| db.append_ranking_rows(id, rows),
    )
    .unwrap();
    db.analysis_finish(id, AestheticAnalysisSummary::Fit(summary))
        .unwrap();
    db.analysis_job(id).unwrap()
}
#[test]
fn frozen_watermark_excludes_later_paid_results_and_rebuild_is_idempotent() {
    let (_dir, db, stage) = fixture(32);
    let (a, aid) = sent(&db, &stage);
    db.receive(&stage, &aid, receipt(&a)).unwrap();
    db.parse_received(&stage).unwrap();
    let request = fit_request(&stage);
    let job = db.analysis_create(request.clone()).unwrap();
    assert_eq!(job.input.observations, 1);
    let (b, bid) = sent(&db, &stage);
    db.receive(&stage, &bid, receipt(&b)).unwrap();
    db.parse_received(&stage).unwrap();
    assert_eq!(
        db.analysis_create(request.clone())
            .unwrap()
            .input
            .evidence_watermark,
        job.input.evidence_watermark
    );
    let mut conflict = request;
    conflict.name = "另一个请求".into();
    assert_eq!(
        db.analysis_create(conflict).unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    complete(&db, &job.id);
    let rows = db.ranking_page(&job.id, 0, None, 256).unwrap();
    assert_eq!(rows.len(), 32);
    let read = db.read().unwrap();
    let plan = {
        let mut statement = read
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                crate::aesthetic::analysis::ranking_page_sql(true)
            ))
            .unwrap();
        statement
            .query_map(params![job.id, 0, "g", 64], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        plan.iter().any(|line| line.contains("ranking_rating")),
        "{plan:?}"
    );
    drop(read);
    assert_eq!(rows.iter().map(|r| u64::from(r.exposures)).sum::<u64>(), 16);
    assert_eq!(rows.iter().filter(|r| r.protected).count(), 1);
    assert!(rows.iter().all(|r| r.rating_rank_min.is_none()));
    assert_eq!(
        db.append_ranking_rows(&job.id, vec![rows[0].clone()])
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let next = db.analysis_create(fit_request(&stage)).unwrap();
    complete(&db, &next.id);
    assert_eq!(
        db.ranking_page(&next.id, 0, None, 256)
            .unwrap()
            .iter()
            .map(|r| u64::from(r.exposures))
            .sum::<u64>(),
        32
    );
    assert_eq!(db.stage(&stage).unwrap().attempts, 2);
}
#[test]
fn renamed_and_removed_jobs_keep_frozen_requests_and_dependents() {
    let (_dir, db, stage) = fixture(32);
    let (a, aid) = sent(&db, &stage);
    db.receive(&stage, &aid, receipt(&a)).unwrap();
    db.parse_received(&stage).unwrap();
    let request = fit_request(&stage);
    let running = db.analysis_create(request.clone()).unwrap();
    assert_eq!(
        db.analysis_remove(&running.id).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let left = complete(&db, &running.id);
    let right = complete(&db, &db.analysis_create(fit_request(&stage)).unwrap().id);
    assert_eq!(
        db.analysis_rename(&left.id, "  基准  ")
            .unwrap()
            .request
            .name,
        "基准"
    );
    // Idempotent retries still match the frozen request, but report the new name.
    assert_eq!(db.analysis_create(request).unwrap().request.name, "基准");
    assert_eq!(
        db.analysis_rename(&left.id, " ").unwrap_err().code,
        "INVALID_INPUT"
    );
    let compare = db
        .analysis_create(AestheticAnalysisCreate {
            idempotency_key: new_id(),
            name: "对照".into(),
            spec: AestheticAnalysisSpec::Compare {
                left: left.id.clone(),
                right: right.id.clone(),
            },
        })
        .unwrap();
    db.analysis_remove(&right.id).unwrap();
    let listed: Vec<_> = db
        .analysis_jobs("", None, 50)
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect();
    assert!(listed.contains(&left.id) && listed.contains(&compare.id));
    assert!(!listed.contains(&right.id));
    assert_eq!(
        db.latest_stage_snapshot(&stage).unwrap().unwrap().id,
        left.id
    );
    // Existing references stay readable; new jobs cannot start from a removed snapshot.
    assert_eq!(db.ranking_snapshot(&right.id).unwrap().id, right.id);
    assert_eq!(db.analysis_job(&compare.id).unwrap().state, "queued");
    assert_eq!(
        db.analysis_create(AestheticAnalysisCreate {
            idempotency_key: new_id(),
            name: "对照".into(),
            spec: AestheticAnalysisSpec::Compare {
                left: left.id.clone(),
                right: right.id.clone(),
            },
        })
        .unwrap_err()
        .code,
        "NOT_FOUND"
    );
    assert_eq!(
        db.analysis_rename(&right.id, "改名").unwrap_err().code,
        "NOT_FOUND"
    );
}
#[test]
fn partial_projection_recovers_without_publishing_or_network_retries() {
    let (dir, db, stage) = fixture(16);
    let (batch, attempt) = sent(&db, &stage);
    db.receive(&stage, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&stage).unwrap();
    let job = db.analysis_create(fit_request(&stage)).unwrap();
    db.analysis_start(&job.id).unwrap();
    assert_eq!(
        db.ranking_page(&job.id, 0, None, 64).unwrap_err().code,
        "RESULT_NOT_READY"
    );
    drop(db);
    let db = EvaluationDb::open(&dir.path().join("evaluation.sqlite")).unwrap();
    let restored = db.analysis_job(&job.id).unwrap();
    assert_eq!(restored.state, "interrupted");
    assert_eq!(
        restored.input.evidence_watermark,
        job.input.evidence_watermark
    );
    db.analysis_control(&job.id, "resume").unwrap();
    complete(&db, &job.id);
    assert_eq!(db.stage(&stage).unwrap().attempts, 1);
    assert_eq!(db.ranking_page(&job.id, 0, None, 64).unwrap().len(), 16);
}
#[test]
fn experiments_and_review_watermarks_are_frozen_and_review_has_no_score_effect() {
    let (_dir, db, stage) = fixture(16);
    let (batch, attempt) = sent(&db, &stage);
    db.receive(&stage, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&stage).unwrap();
    let request = fit_request(&stage);
    let AestheticAnalysisSpec::Fit { config, .. } = request.spec.clone() else {
        panic!("fit")
    };
    let experiment = db
        .experiment_create(AestheticExperimentCreate {
            idempotency_key: new_id(),
            name: "对照".into(),
            description: "fixture".into(),
            variants: vec![AestheticExperimentVariant {
                label: "A".into(),
                fit: config,
            }],
        })
        .unwrap();
    let job = db.analysis_create(request).unwrap();
    complete(&db, &job.id);
    let rows = db.ranking_page(&job.id, 0, None, 64).unwrap();
    let row = rows.iter().find(|r| !r.protected).unwrap();
    let derive = db
        .analysis_create(AestheticAnalysisCreate {
            idempotency_key: new_id(),
            name: "候选".into(),
            spec: AestheticAnalysisSpec::Derive {
                snapshot_id: job.id.clone(),
                filter: Default::default(),
                review_watermark: None,
            },
        })
        .unwrap();
    let review = AestheticReviewCreate {
        idempotency_key: new_id(),
        snapshot_id: job.id.clone(),
        ordinal: row.ordinal,
        decision: "protect".into(),
        reviewer: "test human".into(),
        reason: "校准参考".into(),
    };
    let saved = db.review_create(review.clone()).unwrap();
    assert_eq!(db.review_create(review).unwrap().sequence, saved.sequence);
    assert!(
        !db.effective_protection(
            &job.id,
            std::slice::from_ref(row),
            derive.input.review_watermark
        )
        .unwrap()[0]
    );
    assert!(
        db.effective_protection(&job.id, std::slice::from_ref(row), saved.sequence)
            .unwrap()[0]
    );
    let unchanged = db.ranking_candidate(&job.id, row.ordinal).unwrap();
    assert_eq!(row.score, unchanged.score);
    assert!(!unchanged.protected);
    assert_eq!(
        experiment.inputs[0].evidence_watermark,
        job.input.evidence_watermark
    );
}
#[test]
fn candidate_review_history_is_bounded_latest_first_and_isolated() {
    let (_dir, db, stage) = fixture(16);
    let (batch, attempt) = sent(&db, &stage);
    db.receive(&stage, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&stage).unwrap();
    let job = db.analysis_create(fit_request(&stage)).unwrap();
    complete(&db, &job.id);
    let mut sequences = Vec::new();
    for n in 0..70 {
        let review = db
            .review_create(AestheticReviewCreate {
                idempotency_key: new_id(),
                snapshot_id: job.id.clone(),
                ordinal: 0,
                decision: if n % 2 == 0 { "protect" } else { "release" }.into(),
                reviewer: "history test".into(),
                reason: format!("revision {n}"),
            })
            .unwrap();
        sequences.push(review.sequence);
    }
    db.review_create(AestheticReviewCreate {
        idempotency_key: new_id(),
        snapshot_id: job.id.clone(),
        ordinal: 1,
        decision: "defer".into(),
        reviewer: "another image".into(),
        reason: "separate history".into(),
    })
    .unwrap();
    let first = db.candidate_reviews(&job.id, 0, 0).unwrap();
    assert_eq!(first.len(), 65);
    assert_eq!(first[0].sequence, sequences[69]);
    assert_eq!(first[0].request.decision, "release");
    let second = db
        .candidate_reviews(&job.id, 0, first[63].sequence)
        .unwrap();
    assert_eq!(second.len(), 6);
    assert_eq!(second.last().unwrap().sequence, sequences[0]);
    assert!(db.candidate_reviews(&job.id, 2, 0).unwrap().is_empty());
    assert!(db.candidate_reviews(&job.id, 1000, 0).is_err());
    assert!(db.candidate_reviews(&job.id, u64::MAX, 0).is_err());
    assert!(db.candidate_reviews(&job.id, 0, u64::MAX).is_err());
    let other = db.analysis_create(fit_request(&stage)).unwrap();
    complete(&db, &other.id);
    assert!(db.candidate_reviews(&other.id, 0, 0).unwrap().is_empty());
}
#[test]
fn ledger_v1_upgrade_backs_up_paid_evidence_before_creating_projections() {
    let (dir, db, stage) = fixture(16);
    let (batch, attempt) = sent(&db, &stage);
    db.receive(&stage, &attempt, receipt(&batch)).unwrap();
    db.parse_received(&stage).unwrap();
    drop(db);
    let legacy = dir.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    let path = legacy.join("evaluation.sqlite");
    legacy_copy(&dir.path().join("evaluation.sqlite"), &path, 1);
    let db = EvaluationDb::open(&path).unwrap();
    assert_eq!(db.stage(&stage).unwrap().accepted, 1);
    let backups = std::fs::read_dir(legacy.join(".backups"))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(backups[0].path()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(backup);
    drop(db);
    let future = Connection::open(&path).unwrap();
    future.execute_batch("PRAGMA user_version=999;").unwrap();
    drop(future);
    assert!(matches!(
        EvaluationDb::open(&path),
        Err(Error {
            code: "FORMAT_UNSUPPORTED",
            ..
        })
    ));
}
