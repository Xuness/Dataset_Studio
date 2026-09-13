//! Engine-owned population execution, publication, and immutable table access.
use crate::worker;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
use studio_application::{ArtifactRepository, ReadResources, read_cancelled};
use studio_domain::*;
use studio_operators::ranking::{self as formula, ArtistWork, Sample};
use studio_sources::RankingReader;
use studio_storage::{
    SqliteStore, atomic_json,
    ranking_tables::{RankingInputTable, RankingResultTable},
};

const STAGE_LIMIT: u64 = 64 << 30;
fn parameters(run: &OperatorRun) -> Result<RankingParameters> {
    serde_json::from_value::<RankingParameters>(run.parameters.clone())
        .map_err(Error::io)?
        .normalize()
}
fn remove_partial(directory: &Path, name: &str) -> Result<()> {
    let path = directory.join(name);
    if path.exists() {
        if path.canonicalize().map_err(Error::io)?.parent() != Some(directory) {
            return Err(Error::invalid("任务材料不能指向其他目录"));
        }
        fs::remove_file(path).map_err(Error::io)?;
    }
    Ok(())
}
fn check_stage(path: &Path) -> Result<()> {
    if fs::metadata(path).map_err(Error::io)?.len() > STAGE_LIMIT {
        return Err(Error::new(
            "RANKING_DISK_LIMIT",
            "排名材料超过 64 GiB 单表预算，请缩小范围",
        ));
    }
    Ok(())
}
pub fn prepare(
    store: &SqliteStore,
    job: &Job,
    staging: &Path,
    resources: &dyn ReadResources,
    cancelled: Arc<AtomicBool>,
) -> Result<(PathBuf, WorkerPlan)> {
    let frozen = store.job_run(&job.project_id, &job.id)?;
    let p = parameters(&frozen.run)?;
    crate::tool_inputs::validate_versions(store, &job.project_id, &frozen)?;
    store.update_job(&job.project_id, &job.id, "preparing", 0, None, None)?;
    store.job_stage(
        &job.project_id,
        &job.id,
        &JobStage {
            name: "scope_basis".into(),
            total: job.total,
            ..Default::default()
        },
    )?;
    let memory = resources
        .metrics()
        .into_iter()
        .find(|m| m.budget.class == ReadClass::NativeQuery)
        .map(|m| m.budget.bytes)
        .unwrap_or(4 << 30)
        .min(4 << 30);
    for name in ["input.sqlite", "input.sqlite-journal"] {
        remove_partial(staging, name)?;
    }
    let input_path = staging.join("input.sqlite");
    let mut table = if p.v2.is_some() {
        RankingInputTable::create_v2(&input_path)?
    } else {
        RankingInputTable::create(&input_path)?
    };
    let bases = store.ranking_job_bases(&job.project_id, &job.id)?;
    table.set_meta("job_run", &frozen)?;
    table.set_meta("bases", &bases)?;
    table.set_meta("memory_bytes", &memory)?;
    table.set_meta("created_at", &job.created_at)?;
    let (mut after, mut count) = (None, 0u64);
    let mut last_progress = Instant::now() - Duration::from_secs(1);
    loop {
        read_cancelled(&cancelled)?;
        let keys = store.job_inputs(&job.project_id, &job.id, after.as_ref())?;
        if keys.is_empty() {
            break;
        }
        let branches = store.ranking_member_bases(&job.project_id, &job.id, &keys, &bases)?;
        let rows = keys
            .iter()
            .cloned()
            .zip(branches)
            .map(|(key, b)| {
                let ordinal = count;
                count += 1;
                (ordinal, key, b)
            })
            .collect::<Vec<_>>();
        table.append_members(&rows)?;
        after = keys.last().cloned();
        if last_progress.elapsed() >= Duration::from_millis(700) {
            store.job_stage(
                &job.project_id,
                &job.id,
                &JobStage {
                    name: "scope_basis".into(),
                    completed: count,
                    total: job.total,
                    ..Default::default()
                },
            )?;
            last_progress = Instant::now();
            check_stage(&input_path)?;
        }
    }
    if count != job.total {
        return Err(Error::new("INPUT_CHANGED", "排名任务的固定成员数量不一致"));
    }
    table.flush()?;
    let members = RankingInputTable::open(&input_path)?;
    let reader = RankingReader::configured(staging.join("native"), memory);
    let mut completed = 0u64;
    for expected in &frozen.source_versions {
        read_cancelled(&cancelled)?;
        let _permit = resources.acquire(
            ReadRequest {
                class: ReadClass::NativeQuery,
                priority: ReadPriority::Background,
                bytes: memory,
            },
            &cancelled,
        )?;
        let source = store.source(&job.project_id, &expected.source_id)?;
        store.job_stage(
            &job.project_id,
            &job.id,
            &JobStage {
                name: "metadata_snapshot".into(),
                completed,
                total: job.total,
                ..Default::default()
            },
        )?;
        reader.project(
            &source,
            expected,
            &bases,
            &p,
            cancelled.clone(),
            &mut |append| {
                let mut cursor = None;
                loop {
                    read_cancelled(&cancelled)?;
                    let rows = members.members(&source.id, cursor)?;
                    if rows.is_empty() {
                        break;
                    }
                    for (ordinal, key, basis) in &rows {
                        for index in basis {
                            append(*ordinal, &key.asset_id, *index)?;
                        }
                    }
                    cursor = rows.last().map(|v| v.0);
                }
                Ok(())
            },
            &mut |rows| {
                read_cancelled(&cancelled)?;
                table.append(rows)?;
                completed += rows.len() as u64;
                if last_progress.elapsed() >= Duration::from_millis(700) {
                    store.job_stage(
                        &job.project_id,
                        &job.id,
                        &JobStage {
                            name: "metadata_snapshot".into(),
                            completed,
                            total: job.total,
                            ..Default::default()
                        },
                    )?;
                    store.update_job(
                        &job.project_id,
                        &job.id,
                        "preparing",
                        completed,
                        None,
                        None,
                    )?;
                    last_progress = Instant::now();
                    check_stage(&input_path)?;
                }
                Ok(())
            },
        )?;
    }
    store.job_stage(
        &job.project_id,
        &job.id,
        &JobStage {
            name: "snapshot_index".into(),
            ..Default::default()
        },
    )?;
    table.verify_members(job.total)?;
    table.finalize(&p.ratings)?;
    crate::tool_inputs::validate_versions(store, &job.project_id, &frozen)?;
    read_cancelled(&cancelled)?;
    table.set_meta("complete", &true)?;
    drop(members);
    drop(table);
    fs::OpenOptions::new()
        .write(true)
        .open(&input_path)
        .map_err(Error::io)?
        .sync_all()
        .map_err(Error::io)?;
    let mut plan = WorkerPlan {
        version: 2,
        job_id: job.id.clone(),
        input_path: "input.sqlite".into(),
        input_sha256: worker::hash_file_progress(&input_path, &mut |completed, total| {
            read_cancelled(&cancelled)?;
            store.job_stage(
                &job.project_id,
                &job.id,
                &JobStage {
                    name: "input_checksum".into(),
                    completed,
                    total,
                    ..Default::default()
                },
            )
        })?,
        output_path: "output.sqlite".into(),
        checkpoint_path: "checkpoint.json".into(),
        total: job.total,
        delay_ms: store.job_delay(&job.project_id, &job.id)?,
        run: frozen.run,
    };
    let plan_path = staging.join("plan.json");
    atomic_json(&plan_path, &plan)?;
    // These paths and the hash were constructed here; the worker will independently
    // validate the persisted plan and input before reading the snapshot.
    plan.input_path = input_path;
    plan.output_path = staging.join("output.sqlite");
    plan.checkpoint_path = staging.join("checkpoint.json");
    Ok((plan_path, plan))
}

#[derive(Default, Serialize, Deserialize)]
struct Checkpoint {
    job_id: String,
    input_sha256: String,
    initialized: bool,
    ineligible: u64,
    eligible: BTreeMap<String, u64>,
    finished: Vec<RankingRatingSummary>,
    working: Option<String>,
}
fn emit(
    plan: &WorkerPlan,
    completed: u64,
    name: &str,
    stage_completed: u64,
    stage_total: u64,
    rating: Option<&str>,
) -> Result<()> {
    worker::report_progress(
        completed,
        plan.total,
        Some(JobStage {
            name: name.into(),
            completed: stage_completed,
            total: stage_total,
            rating: rating.map(str::to_owned),
            ..Default::default()
        }),
    )
}
fn ineligible(input: &RankingInput, p: &RankingParameters) -> RankingScores {
    RankingScores {
        ordinal: input.ordinal,
        rating: input.rating.clone(),
        eligibility: if input.duplicate_of.is_some() {
            RankingEligibility::Duplicate
        } else {
            formula::eligibility(input, p)
        },
        missing_flags: formula::flag_names(formula::flags(input)),
        selected_route: RankingRoute::Ineligible,
        time_reason: "not_eligible".into(),
        duplicate_of: input.duplicate_of,
        ..Default::default()
    }
}
pub fn run(plan: &WorkerPlan) -> Result<()> {
    let input = RankingInputTable::open(&plan.input_path)?;
    if !input.meta::<bool>("complete")? || input.count()? != plan.total {
        return Err(Error::new("INPUT_INVALID", "排名输入尚未完成"));
    }
    let run: JobRun = input.meta("job_run")?;
    if run.run != plan.run {
        return Err(Error::new("INPUT_CHANGED", "排名参数与固定材料不一致"));
    }
    let p = parameters(&plan.run)?;
    let memory: u64 = input.meta("memory_bytes")?;
    let mut state: Checkpoint = if plan.checkpoint_path.exists() {
        if fs::metadata(&plan.checkpoint_path)
            .map_err(Error::io)?
            .len()
            > 65_536
        {
            return Err(Error::new("CHECKPOINT_INVALID", "排名检查点超过预算"));
        }
        serde_json::from_slice(&fs::read(&plan.checkpoint_path).map_err(Error::io)?)
            .map_err(Error::io)?
    } else {
        Checkpoint {
            job_id: plan.job_id.clone(),
            input_sha256: plan.input_sha256.clone(),
            ..Default::default()
        }
    };
    if state.job_id != plan.job_id || state.input_sha256 != plan.input_sha256 {
        return Err(Error::new("CHECKPOINT_INVALID", "排名检查点不属于本次输入"));
    }
    let mut output = if plan.output_path.exists() {
        RankingResultTable::resume(&plan.output_path)?
    } else {
        if p.v2.is_some() {
            RankingResultTable::create_v2(&plan.output_path)?
        } else {
            RankingResultTable::create(&plan.output_path)?
        }
    };
    if input.is_v2()? != p.v2.is_some() || output.is_v2()? != p.v2.is_some() {
        return Err(Error::new(
            "RANKING_FORMAT_UNSUPPORTED",
            "排名任务与暂存材料版本不一致",
        ));
    }
    if !state.initialized {
        emit(plan, 0, "eligibility", 0, plan.total, None)?;
        output.reset()?;
        state.finished.clear();
        state.eligible.clear();
        state.ineligible = 0;
        state.working = None;
        let mut after = None;
        let mut visited = 0u64;
        let mut last = Instant::now() - Duration::from_secs(1);
        loop {
            let page = input.page(after, None)?;
            if page.is_empty() {
                break;
            }
            let mut rows = Vec::new();
            for row in &page {
                let classified = ineligible(row, &p);
                if classified.eligibility == RankingEligibility::Eligible {
                    *state
                        .eligible
                        .entry(row.rating.clone().expect("eligible rating"))
                        .or_default() += 1;
                } else {
                    state.ineligible += 1;
                    rows.push(classified);
                }
            }
            output.append(&rows)?;
            visited += page.len() as u64;
            after = page.last().map(|r| r.ordinal);
            if last.elapsed() >= Duration::from_millis(500) {
                emit(
                    plan,
                    state.ineligible,
                    "eligibility",
                    visited,
                    plan.total,
                    None,
                )?;
                last = Instant::now();
                check_stage(&plan.output_path)?;
            }
        }
        output.flush()?;
        state.initialized = true;
        atomic_json(&plan.checkpoint_path, &state)?;
    }
    for rating in ["g", "s", "q", "e"] {
        let count = *state.eligible.get(rating).unwrap_or(&0);
        if count == 0 || state.finished.iter().any(|s| s.rating == rating) {
            continue;
        }
        let estimated = count
            .saturating_mul(if p.v2.is_some() {
                if p.artist_enabled { 896 } else { 704 }
            } else if p.artist_enabled {
                768
            } else {
                512
            })
            .saturating_add(128 << 20);
        if estimated > memory {
            return Err(Error::new(
                "RANKING_MEMORY_LIMIT",
                format!(
                    "{rating} 分级有 {count} 个候选，估计需要约 {} MiB 工作内存，当前预算为 {} MiB；请调整性能设置或缩小范围",
                    estimated >> 20,
                    memory >> 20
                ),
            ));
        }
        if state.working.as_deref() == Some(rating) {
            output.delete_eligible_rating(rating)?;
        }
        state.working = Some(rating.into());
        atomic_json(&plan.checkpoint_path, &state)?;
        let mut samples = Vec::with_capacity(count as usize);
        let mut works = Vec::new();
        let mut type_hints = Vec::new();
        let mut names = BTreeMap::<String, u32>::new();
        let mut after = None;
        let base = state.ineligible + state.finished.iter().map(|s| s.eligible).sum::<u64>();
        let mut last = Instant::now() - Duration::from_secs(1);
        emit(plan, base, "loading_rating", 0, count, Some(rating))?;
        loop {
            let page = input.page(after, Some(rating))?;
            if page.is_empty() {
                break;
            }
            for row in &page {
                if row.duplicate_of.is_some()
                    || formula::eligibility(row, &p) != RankingEligibility::Eligible
                {
                    continue;
                }
                let item = samples.len();
                samples.push(Sample::new(row, &p)?);
                if p.v2.is_some() {
                    type_hints.push(studio_operators::ranking_v2::type_hints(
                        row.tags.as_deref(),
                    ));
                }
                if p.artist_enabled {
                    for name in &row.artists {
                        let next = names.len() as u32;
                        let artist = *names.entry(name.clone()).or_insert(next);
                        works.push(ArtistWork { item, artist });
                    }
                }
            }
            after = page.last().map(|r| r.ordinal);
            if last.elapsed() >= Duration::from_millis(500) {
                emit(
                    plan,
                    base,
                    "loading_rating",
                    samples.len() as u64,
                    count,
                    Some(rating),
                )?;
                last = Instant::now();
            }
        }
        if samples.len() as u64 != count {
            return Err(Error::new("INPUT_INVALID", "排名候选数量不一致"));
        }
        // Interned artists use lexical order so multi-artist averaging has a stable evaluation order.
        let mut remap = vec![0u32; names.len()];
        for (new, old) in names.values().enumerate() {
            remap[*old as usize] = new as u32;
        }
        for work in &mut works {
            work.artist = remap[work.artist as usize];
        }
        drop(names);
        drop(remap);
        let mut current_phase = String::new();
        let mut report_progress = |phase: &str, completed, total| {
            if current_phase != phase || last.elapsed() >= Duration::from_millis(500) {
                emit(plan, base, phase, completed, total, Some(rating))?;
                last = Instant::now();
                current_phase = phase.into();
            }
            Ok(())
        };
        let (summary, v2_scores) = if p.v2.is_some() {
            let (summary, values) = studio_operators::ranking_v2::compute(
                rating,
                &mut samples,
                &type_hints,
                &p,
                &mut works,
                &mut report_progress,
            )?;
            (summary, Some(values))
        } else {
            (
                formula::compute_rating(
                    rating,
                    &mut samples,
                    &p,
                    &mut works,
                    &mut report_progress,
                )?,
                None,
            )
        };
        drop(type_hints);
        drop(works);
        let mut written = 0u64;
        emit(plan, base, "writing", 0, count, Some(rating))?;
        for (batch_index, batch) in samples.chunks(512).enumerate() {
            output.append(
                &batch
                    .iter()
                    .enumerate()
                    .map(|(index, s)| {
                        let mut row = s.scores(rating);
                        row.v2 = v2_scores
                            .as_ref()
                            .map(|values| values[batch_index * 512 + index]);
                        row
                    })
                    .collect::<Vec<_>>(),
            )?;
            written += batch.len() as u64;
            if plan.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(plan.delay_ms.min(1000)));
            }
            if last.elapsed() >= Duration::from_millis(500) {
                emit(
                    plan,
                    base + written,
                    "writing",
                    written,
                    count,
                    Some(rating),
                )?;
                last = Instant::now();
                check_stage(&plan.output_path)?;
            }
        }
        output.flush()?;
        state.finished.push(summary);
        state.working = None;
        atomic_json(&plan.checkpoint_path, &state)?;
    }
    if output.count()? != plan.total {
        return Err(Error::new("RANKING_INVALID", "排名结果行数不完整"));
    }
    emit(plan, plan.total, "indexing", 0, 1, None)?;
    let summary = RankingSummary {
        schema_version: if p.v2.is_some() { 2 } else { 1 },
        input_count: plan.total,
        eligible_count: state.finished.iter().map(|s| s.eligible).sum(),
        parameters: p,
        ratings: state.finished,
        eligibility_counts: output.counts("eligibility")?,
        missing_counts: output.missing_counts()?,
        input_sha256: plan.input_sha256.clone(),
        created_at: input.meta("created_at")?,
    };
    output.finish(&summary)?;
    drop(output);
    fs::OpenOptions::new()
        .write(true)
        .open(&plan.output_path)
        .map_err(Error::io)?
        .sync_all()
        .map_err(Error::io)?;
    emit(plan, plan.total, "complete", 1, 1, None)
}

pub fn validate_output(path: &Path, plan: &WorkerPlan) -> Result<String> {
    validate_output_progress(path, plan, &mut |_, _, _| Ok(()))
}
pub fn validate_output_progress(
    path: &Path,
    plan: &WorkerPlan,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
) -> Result<String> {
    if worker::hash_file_progress(&plan.input_path, &mut |completed, total| {
        progress("validating_input", completed, total)
    })? != plan.input_sha256
    {
        return Err(Error::new("INPUT_CHANGED", "固定排名输入校验失败"));
    }
    let input = RankingInputTable::open(&plan.input_path)?;
    let output = RankingResultTable::open(path)?;
    let summary: RankingSummary = output.meta("summary")?;
    let p = parameters(&plan.run)?;
    if input.is_v2()? != p.v2.is_some()
        || output.is_v2()? != p.v2.is_some()
        || summary.schema_version != if p.v2.is_some() { 2 } else { 1 }
        || !output.meta::<bool>("complete")?
        || summary.parameters != p
        || summary.input_sha256 != plan.input_sha256
        || summary.input_count != plan.total
        || output.count()? != plan.total
        || input.count()? != plan.total
    {
        return Err(Error::new("RANKING_INVALID", "排名成果与固定任务不一致"));
    }
    let (mut after, mut count) = (None, 0u64);
    let mut last_progress = Instant::now();
    progress("validating", 0, plan.total)?;
    let mut counts = BTreeMap::<String, (u64, [u64; 3])>::new();
    let mut rank_seen = HashMap::<String, (Vec<bool>, Vec<bool>)>::new();
    let mut v2_rank_seen = HashMap::<String, (Vec<bool>, Vec<bool>)>::new();
    for s in &summary.ratings {
        let length = usize::try_from(s.eligible)
            .map_err(|_| Error::new("RANKING_INVALID", "排名长度无效"))?;
        if s.eligible > plan.total {
            return Err(Error::new("RANKING_INVALID", "排名数量超出固定范围"));
        }
        rank_seen.insert(s.rating.clone(), (vec![false; length], vec![false; length]));
        if p.v2.is_some() {
            v2_rank_seen.insert(s.rating.clone(), (vec![false; length], vec![false; length]));
        }
    }
    loop {
        let a = input.page(after, None)?;
        let b = output.all_page(after)?;
        if a.is_empty() && b.is_empty() {
            break;
        }
        if a.len() != b.len() {
            return Err(Error::new("RANKING_INVALID", "排名成果覆盖不完整"));
        }
        for (i, s) in a.iter().zip(&b) {
            if i.ordinal != s.ordinal || i.rating != s.rating {
                return Err(Error::new("RANKING_INVALID", "排名行与输入身份不一致"));
            }
            let classified = ineligible(i, &p);
            if classified.eligibility != s.eligibility || classified.duplicate_of != s.duplicate_of
            {
                return Err(Error::new("RANKING_INVALID", "排名用途资格不一致"));
            }
            if s.eligibility == RankingEligibility::Eligible {
                let rating = s.rating.clone().expect("eligible rating");
                let (g, c, a, v, t) = (s.g, s.c, s.a, s.v, s.t);
                let (Some(g), Some(c), Some(a), Some(v), Some(t)) = (g, c, a, v, t) else {
                    return Err(Error::new("RANKING_INVALID", "排名特征缺失"));
                };
                if [g, c, a, v, t]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(Error::new("RANKING_INVALID", "排名特征范围无效"));
                }
                let main = 100.0
                    * (g + p.time_weight * (c - g).max(0.0) - p.vote_weight * v - t)
                        .clamp(0.0, 1.0);
                let rescue = 100.0
                    * (c + p.artist_weight * a * (1.0 - c) - p.vote_weight * v - t).clamp(0.0, 1.0);
                if p.v2.is_some() {
                    studio_operators::ranking_v2::validate_scores(i, s, &p)?;
                    let v = s.v2.expect("validated v2");
                    let seen = v2_rank_seen
                        .get_mut(&rating)
                        .ok_or_else(|| Error::new("RANKING_INVALID", "v2 缺少分级统计"))?;
                    for (rank, flags) in [(v.direct_rank, &mut seen.0), (v.fused_rank, &mut seen.1)]
                    {
                        let at = rank
                            .checked_sub(1)
                            .and_then(|n| usize::try_from(n).ok())
                            .filter(|n| *n < flags.len())
                            .ok_or_else(|| Error::new("RANKING_INVALID", "v2 名次超出范围"))?;
                        if flags[at] {
                            return Err(Error::new("RANKING_INVALID", "v2 名次重复"));
                        }
                        flags[at] = true;
                    }
                } else if s
                    .main_score
                    .is_none_or(|x| !x.is_finite() || (x - main).abs() > 1e-9)
                    || s.rescue_score
                        .is_none_or(|x| !x.is_finite() || (x - rescue).abs() > 1e-9)
                {
                    return Err(Error::new("RANKING_INVALID", "排名分数与特征不一致"));
                }
                let seen = rank_seen
                    .get_mut(&rating)
                    .ok_or_else(|| Error::new("RANKING_INVALID", "排名缺少分级统计"))?;
                for (rank, flags) in [(s.main_rank, &mut seen.0), (s.rescue_rank, &mut seen.1)] {
                    let index = rank
                        .and_then(|v| v.checked_sub(1))
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|v| *v < flags.len())
                        .ok_or_else(|| Error::new("RANKING_INVALID", "名次超出范围"))?;
                    if flags[index] {
                        return Err(Error::new("RANKING_INVALID", "名次重复"));
                    }
                    flags[index] = true;
                }
                let value = counts.entry(rating).or_default();
                value.0 += 1;
                match s.selected_route {
                    RankingRoute::Main => value.1[0] += 1,
                    RankingRoute::Rescue => value.1[1] += 1,
                    RankingRoute::Audit => value.1[2] += 1,
                    RankingRoute::BudgetRejected if p.mode == RankingMode::Select => (),
                    RankingRoute::Ranked if p.mode == RankingMode::Rank => (),
                    _ => return Err(Error::new("RANKING_INVALID", "入选通道无效")),
                }
            } else if s.main_score.is_some()
                || s.main_rank.is_some()
                || s.selected_route != RankingRoute::Ineligible
            {
                return Err(Error::new("RANKING_INVALID", "未通过资格的图片不应有排名"));
            }
            count += 1;
        }
        after = a.last().map(|r| r.ordinal);
        if last_progress.elapsed() >= Duration::from_millis(500) {
            progress("validating", count, plan.total)?;
            last_progress = Instant::now();
        }
    }
    for s in &summary.ratings {
        let actual = counts.get(&s.rating).copied().unwrap_or_default();
        let q = if p.mode == RankingMode::Select {
            if let Some(v2) = &p.v2 {
                if s.selected.iter().sum::<u64>()
                    != s.eligible * u64::from(v2.keep_per_mille) / 1000
                {
                    return Err(Error::new("RANKING_INVALID", "v2 保留数量与预算不一致"));
                }
                s.selected
            } else {
                formula::quotas(s.eligible, p.quotas)
            }
        } else {
            [0; 3]
        };
        if actual.0 != s.eligible || actual.1 != s.selected || s.quotas != q || actual.1 != q {
            return Err(Error::new("RANKING_INVALID", "排名名额或分级数量不一致"));
        }
    }
    if count != plan.total || counts.values().map(|s| s.0).sum::<u64>() != summary.eligible_count {
        return Err(Error::new("RANKING_INVALID", "排名总数不一致"));
    }
    progress("validating", count, plan.total)?;
    worker::hash_file_progress(path, &mut |completed, total| {
        progress("output_checksum", completed, total)
    })
}

pub fn paths(store: &SqliteStore, pid: &str, aid: &str) -> Result<(Artifact, PathBuf, PathBuf)> {
    let item = store.artifact(pid, aid)?;
    if item.kind != RANKING_KIND
        || !matches!(item.schema_version, 1 | 2)
        || item.state != ArtifactState::Ready
    {
        return Err(Error::new("ARTIFACT_NOT_READY", "排名成果尚不可用"));
    }
    let main = format!("artifacts/{}.ranking.sqlite", item.job_id);
    let input = format!("artifacts/{}.ranking-input.sqlite", item.job_id);
    for name in [&main, &input] {
        if !item.files.iter().any(|f| &f.path == name) {
            return Err(Error::new("ARTIFACT_INVALID", "排名文件清单不完整"));
        }
    }
    Ok((
        item,
        crate::artifacts::controlled_path(store, pid, &main)?,
        crate::artifacts::controlled_path(store, pid, &input)?,
    ))
}
pub fn publish(
    store: &SqliteStore,
    job: &Job,
    plan: &WorkerPlan,
    primary: &Path,
    validated: Option<worker::ValidatedOutput>,
) -> Result<()> {
    let mut checked = match validated {
        Some(checked) => checked,
        None => worker::validate_once(primary, plan, &mut |name, completed, total| {
            if store.job(&job.project_id, &job.id)?.status == "cancelled" {
                return Err(Error::new("CANCELLED", "任务已取消"));
            }
            store.job_stage(
                &job.project_id,
                &job.id,
                &JobStage {
                    name: name.into(),
                    completed,
                    total,
                    ..Default::default()
                },
            )
        })?,
    };
    let table_sha = checked.digest(primary, plan)?.to_owned();
    store.job_stage(
        &job.project_id,
        &job.id,
        &JobStage {
            name: "publishing".into(),
            total: 1,
            ..Default::default()
        },
    )?;
    let table = RankingResultTable::open(primary)?;
    let summary: RankingSummary = table.meta("summary")?;
    drop(table);
    let base = format!("artifacts/{}", job.id);
    let input_name = format!("{base}.ranking-input.sqlite");
    let input_target = crate::artifacts::controlled_path(store, &job.project_id, &input_name)?;
    if input_target.exists() {
        if worker::hash_file(&input_target)? != plan.input_sha256 {
            return Err(Error::new("ARTIFACT_INVALID", "已有排名快照内容不一致"));
        }
    } else if fs::hard_link(&plan.input_path, &input_target).is_err() {
        let mut temporary =
            tempfile::NamedTempFile::new_in(input_target.parent().expect("artifact directory"))
                .map_err(Error::io)?;
        std::io::copy(
            &mut File::open(&plan.input_path).map_err(Error::io)?,
            &mut temporary,
        )
        .map_err(Error::io)?;
        temporary.as_file().sync_all().map_err(Error::io)?;
        temporary.persist(&input_target).map_err(Error::io)?;
    }
    let manifest_name = format!("{base}.ranking.json");
    let manifest_path = crate::artifacts::controlled_path(store, &job.project_id, &manifest_name)?;
    atomic_json(&manifest_path, &summary)?;
    let artifact_name = format!("{base}.ranking.sqlite");
    let files = vec![
        ArtifactFile {
            path: artifact_name.clone(),
            bytes: Some(fs::metadata(primary).map_err(Error::io)?.len()),
            sha256: Some(table_sha.clone()),
            media_type: "application/vnd.sqlite3".into(),
        },
        ArtifactFile {
            path: input_name,
            bytes: Some(fs::metadata(&input_target).map_err(Error::io)?.len()),
            sha256: Some(plan.input_sha256.clone()),
            media_type: "application/vnd.sqlite3".into(),
        },
        ArtifactFile {
            path: manifest_name,
            bytes: Some(fs::metadata(&manifest_path).map_err(Error::io)?.len()),
            sha256: Some(worker::hash_file(&manifest_path)?),
            media_type: "application/json".into(),
        },
    ];
    let artifact=Artifact{id:new_id(),project_id:job.project_id.clone(),job_id:job.id.clone(),output_id:"data".into(),name:if summary.schema_version==2 { "Danbooru 元数据排名 · v2" } else { "Danbooru 元数据排名" }.into(),kind:RANKING_KIND.into(),schema_version:summary.schema_version,state:ArtifactState::Publishing,count:Some(plan.total),created_at:job.created_at.clone(),files,
        provenance:ArtifactProvenance{run:Some(plan.run.clone()),input_scope:job.input_scope.clone(),input_sha256:Some(plan.input_sha256.clone()),attempt:Some(store.job(&job.project_id,&job.id)?.attempt),input_artifacts:store.job_scope_artifacts(&job.project_id,&job.id)?,fields_frozen:true,evidence:"immutable_typed_input; scope_and_observation_basis; finite_features_formula_quota_rank_permutation_and_membership_validated".into()},issue:None};
    let artifact = store.begin_artifact(&job.project_id, &artifact)?;
    if artifact.state == ArtifactState::Publishing {
        store.register_ranking_table(
            &job.project_id,
            &artifact.id,
            &table_sha,
            &plan.input_sha256,
            &summary,
        )?;
    }
    store.finish_artifacts(
        &job.project_id,
        &job.id,
        &[artifact.id],
        Some(&artifact_name),
    )?;
    store.job_stage(
        &job.project_id,
        &job.id,
        &JobStage {
            name: "complete".into(),
            completed: 1,
            total: 1,
            ..Default::default()
        },
    )?;
    // The registered immutable input is now authoritative; successful tasks cannot be retried.
    drop(checked);
    if let Err(error) = fs::remove_file(&plan.input_path) {
        tracing::warn!(%error,"ranking staging input cleanup deferred");
    }
    Ok(())
}
