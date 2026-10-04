//! Local CPU jobs have their own admission and never touch the LLM service.
use crate::api::AppState;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use studio_application::aesthetic_analysis::{comparison::Comparison, estimator, matches_filter};
use studio_domain::{Error, Result, aesthetic_analysis::*};
use tokio::sync::Semaphore;

pub struct Runner {
    active: Mutex<HashMap<(String, String), Arc<AtomicBool>>>,
    compute: Arc<Semaphore>,
    stopped: AtomicBool,
}
impl Default for Runner {
    fn default() -> Self {
        Self {
            active: Default::default(),
            compute: Arc::new(Semaphore::new(1)),
            stopped: AtomicBool::new(false),
        }
    }
}
impl Runner {
    pub fn contains(&self, pid: &str, id: &str) -> bool {
        self.active
            .lock()
            .is_ok_and(|v| v.contains_key(&(pid.into(), id.into())))
    }
    pub fn busy(&self) -> bool {
        self.active.lock().is_ok_and(|v| !v.is_empty())
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(active) = self.active.lock() {
            for flag in active.values() {
                flag.store(true, Ordering::Release);
            }
        }
    }
    pub fn cancel(&self, pid: &str, id: &str) {
        if let Ok(active) = self.active.lock()
            && let Some(flag) = active.get(&(pid.into(), id.into()))
        {
            flag.store(true, Ordering::Release);
        }
    }
    pub fn launch(self: &Arc<Self>, state: AppState, pid: String, id: String) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::new("EVALUATION_BUSY", "离线执行器正在关闭"));
        }
        let lease = state.store.operation_lease(&pid)?;
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::io("离线执行器锁不可用"))?;
            if self.stopped.load(Ordering::Acquire) {
                return Err(Error::new("EVALUATION_BUSY", "离线执行器正在关闭"));
            }
            if active.contains_key(&(pid.clone(), id.clone())) {
                return Err(Error::new("REVISION_CONFLICT", "离线任务已在执行"));
            }
            if active.len() >= 16 {
                return Err(Error::new("EVALUATION_BUSY", "离线任务等待队列已满"));
            }
            active.insert((pid.clone(), id.clone()), flag.clone());
        }
        let runner = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let permit = loop {
                if flag.load(Ordering::Acquire) {
                    break None;
                }
                tokio::select! {
                    p=runner.compute.clone().acquire_owned()=>{break p.ok();},
                    _=tokio::time::sleep(Duration::from_millis(100))=>{}
                }
            };
            let p = pid.clone();
            let key = id.clone();
            let s = state.clone();
            let cancel = flag.clone();
            let result = tokio::task::spawn_blocking(move || {
                let db = s.store.evaluation(&p)?;
                if cancel.load(Ordering::Acquire) {
                    return Err(Error::new("CANCELLED", "离线任务已停止"));
                }
                let item = db.analysis_start(&key)?;
                s.store.sync_analysis(&p, &key)?;
                run(&s, &p, &db, item, &cancel)
            })
            .await
            .unwrap_or_else(|e| Err(Error::io(format!("离线计算异常: {e}"))));
            drop(permit);
            let p = pid.clone();
            let key = id.clone();
            let s = state.clone();
            let stopped = runner.stopped.load(Ordering::Acquire);
            let finish = tokio::task::spawn_blocking(move || -> Result<()> {
                if let Err(error) = result {
                    let status = if stopped {
                        "interrupted"
                    } else if error.code == "CANCELLED" {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    s.store
                        .evaluation(&p)?
                        .analysis_fail(&key, status, &error.to_string())?;
                }
                s.store.sync_analysis(&p, &key)
            })
            .await;
            if !matches!(finish, Ok(Ok(()))) {
                tracing::error!("offline analysis finalization needs recovery");
            }
            if let Ok(mut active) = runner.active.lock() {
                active.remove(&(pid, id));
            }
        });
        Ok(())
    }
}
fn check(flag: &AtomicBool) -> Result<()> {
    if flag.load(Ordering::Acquire) {
        Err(Error::new("CANCELLED", "离线任务已停止"))
    } else {
        Ok(())
    }
}
fn fit_summary(job: &AestheticAnalysisJob) -> Result<&AestheticFitSummary> {
    match &job.result {
        Some(AestheticAnalysisSummary::Fit(s)) => Ok(s),
        _ => Err(Error::new("RESULT_NOT_READY", "需要排名快照")),
    }
}
fn run(
    state: &AppState,
    pid: &str,
    db: &studio_storage::aesthetic::EvaluationDb,
    item: AestheticAnalysisJob,
    cancel: &AtomicBool,
) -> Result<()> {
    while db.analysis_reset_page(&item.id)? {
        check(cancel)?;
    }
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut progress = |phase: &str, done: u64, total: u64| -> Result<()> {
        check(cancel)?;
        if last.elapsed() >= Duration::from_millis(500) {
            db.analysis_progress(&item.id, phase, done, total)?;
            last = Instant::now();
        }
        Ok(())
    };
    let result = match &item.request.spec {
        AestheticAnalysisSpec::Fit { config, .. } => {
            let summary = estimator::replay(
                db,
                &item.input,
                config,
                &|| check(cancel),
                &mut progress,
                &mut |rows| db.append_ranking_rows(&item.id, rows),
            )?;
            check(cancel)?;
            AestheticAnalysisSummary::Fit(summary)
        }
        AestheticAnalysisSpec::Compare { left, right } => {
            let a = db.ranking_snapshot(left)?;
            let b = db.ranking_snapshot(right)?;
            let mut comparison = Comparison::new(fit_summary(&a)?, fit_summary(&b)?);
            let mut after = 0;
            loop {
                check(cancel)?;
                let page = db.comparison_input_page(left, right, after)?;
                if page.is_empty() {
                    break;
                }
                for (l, r) in &page {
                    comparison.push(l, r.as_ref());
                    after = l.position;
                }
                progress("comparing", after, item.input.candidates)?;
            }
            let groups = comparison.summary();
            after = 0;
            loop {
                check(cancel)?;
                let page = db.comparison_input_page(left, right, after)?;
                if page.is_empty() {
                    break;
                }
                let mut rows = Vec::with_capacity(page.len());
                for (l, r) in page {
                    let group = groups.iter().find(|g| g.rating == l.rating);
                    let comparable = group.is_some_and(|g| g.comparable);
                    let valid = r
                        .as_ref()
                        .filter(|r| r.rating == l.rating && r.content_version == l.content_version);
                    let right_percentile = valid.and_then(|r| r.percentile);
                    after = l.position;
                    rows.push(AestheticComparisonRow {
                        position: l.position,
                        key: l.key,
                        rating: l.rating,
                        comparable,
                        left_percentile: l.percentile,
                        right_percentile,
                        percentile_delta: if comparable {
                            l.percentile
                                .zip(right_percentile)
                                .map(|(a, b)| (a - b).abs())
                        } else {
                            None
                        },
                        left_protected: l.protected,
                        right_protected: valid.map(|r| r.protected),
                        reason: group.and_then(|g| g.reason.clone()).or_else(|| {
                            valid.is_none().then(|| "另一快照缺少匹配的图片版本".into())
                        }),
                    });
                }
                db.append_comparison_rows(&item.id, rows)?;
                progress("publishing", after, item.input.candidates)?;
            }
            AestheticAnalysisSummary::Compare { groups }
        }
        AestheticAnalysisSpec::Preview {
            snapshot_id,
            filter,
            ..
        } => {
            let mut after = 0;
            let mut count = 0;
            let mut ranked_count = 0;
            let mut protected_added = 0;
            let mut boundary_tie_count = 0;
            let mut ranked = filter.clone();
            ranked.include_protected = false;
            loop {
                check(cancel)?;
                let page = db.ranking_page(snapshot_id, after, None, 256)?;
                if page.is_empty() {
                    break;
                }
                let protected =
                    db.effective_protection(snapshot_id, &page, item.input.review_watermark)?;
                for (row, protected) in page.iter().zip(protected) {
                    after = row.position;
                    if !matches_filter(row, filter, protected) {
                        continue;
                    }
                    count += 1;
                    if matches_filter(row, &ranked, protected) {
                        ranked_count += 1;
                        let lo = if filter.component.is_some() {
                            row.rank_min
                        } else {
                            row.rating_rank_min
                        };
                        let hi = if filter.component.is_some() {
                            row.rank_max
                        } else {
                            row.rating_rank_max
                        };
                        let cutoff = filter.rank_to.or_else(|| {
                            filter.top_percent.map(|v| {
                                ((row.component_size as f64 * v / 100.0).ceil() as u64).max(1)
                            })
                        });
                        if let (Some(lo), Some(hi), Some(cutoff)) = (lo, hi, cutoff)
                            && lo <= cutoff
                            && hi > cutoff
                        {
                            boundary_tie_count += 1;
                        }
                    } else {
                        protected_added += 1;
                    }
                }
                progress("counting_selection", after, item.input.candidates)?;
            }
            AestheticAnalysisSummary::Preview {
                count,
                ranked_count,
                protected_added,
                boundary_tie_count,
            }
        }
        AestheticAnalysisSpec::Derive {
            snapshot_id,
            filter,
            ..
        } => {
            let (_, mut after, _, published) = state.store.begin_evaluation_workset(pid, &item)?;
            if !published {
                loop {
                    check(cancel)?;
                    let page = db.ranking_page(snapshot_id, after, None, 256)?;
                    if page.is_empty() {
                        break;
                    }
                    let protected =
                        db.effective_protection(snapshot_id, &page, item.input.review_watermark)?;
                    let next = page.last().expect("page").position;
                    let keys = page
                        .into_iter()
                        .zip(protected)
                        .filter(|(row, p)| matches_filter(row, filter, *p))
                        .map(|(row, _)| row.key)
                        .collect();
                    state
                        .store
                        .append_evaluation_workset(pid, &item.id, after, next, keys)?;
                    after = next;
                    progress("selecting", after, item.input.candidates)?;
                }
                check(cancel)?;
            }
            let collection = state.store.publish_evaluation_workset(pid, &item)?;
            // Publication in project.sqlite is the commit point. If cancellation raced
            // with it, completion/recovery must report the already-visible workset.
            AestheticAnalysisSummary::Derive {
                collection_id: collection.id,
                count: collection.count,
            }
        }
    };
    db.analysis_finish(&item.id, result)
}
