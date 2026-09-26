use crate::sources::SourceService;
use crate::worker;
use futures::StreamExt;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::{ReadResources, SourceAdapter};
use studio_domain::*;
use studio_storage::{SqliteStore, atomic_json};
use tokio::process::Command;
use tokio_util::codec::{FramedRead, LinesCodec};

static ACTIVE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(Default::default);
struct ActiveAttempt(String, Arc<AtomicBool>);
struct AttemptHeartbeat(tokio::task::JoinHandle<()>);
impl Drop for AttemptHeartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl ActiveAttempt {
    fn enter(job: &Job) -> Result<Self> {
        let key = format!("{}:{}", job.project_id, job.id);
        let mut active = ACTIVE
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "执行器状态锁不可用"))?;
        if active.contains_key(&key) {
            return Err(Error::new("SOURCE_BUSY", "同一任务已有执行尝试"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        active.insert(key.clone(), cancel.clone());
        Ok(Self(key, cancel))
    }
}
pub fn cancel(pid: &str, jid: &str) {
    if let Ok(active) = ACTIVE.lock()
        && let Some(cancel) = active.get(&format!("{pid}:{jid}"))
    {
        cancel.store(true, Ordering::Release);
    }
}
pub fn shutdown() {
    if let Ok(active) = ACTIVE.lock() {
        for cancel in active.values() {
            cancel.store(true, Ordering::Release);
        }
    }
}
impl Drop for ActiveAttempt {
    fn drop(&mut self) {
        self.1.store(true, Ordering::Release);
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.0);
        }
    }
}
pub async fn wait_stopped(pid: &str, jid: &str) -> Result<()> {
    let key = format!("{pid}:{jid}");
    for _ in 0..100 {
        if !ACTIVE
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "执行器状态锁不可用"))?
            .contains_key(&key)
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err(Error::new("SOURCE_BUSY", "旧执行尝试正在停止，请稍后重试"))
}

pub async fn scheduler(
    store: Arc<SqliteStore>,
    resources: Arc<dyn ReadResources>,
    sources: Arc<SourceService>,
) {
    loop {
        let store2 = store.clone();
        let next = tokio::task::spawn_blocking(move || -> Result<Option<Job>> {
            store2.reap_closed()?;
            for id in store2.owned_projects()? {
                if let Err(error) = store2.resolve_job_scopes(&id) {
                    tracing::warn!(project_id=%id,%error,"scope preparation failed");
                    continue;
                }
                match store2.scheduled_jobs(&id, false) {
                    Ok(jobs) => {
                        if let Some(job) = jobs.into_iter().find(|j| j.status == "queued") {
                            return Ok(Some(job));
                        }
                    }
                    Err(error) => {
                        tracing::warn!(project_id=%id, %error, "project scheduling failed")
                    }
                }
            }
            Ok(None)
        })
        .await;
        match next {
            Ok(Ok(Some(job))) => {
                if let Err(error) = execute(
                    store.clone(),
                    job.clone(),
                    resources.clone(),
                    sources.clone(),
                )
                .await
                {
                    tracing::warn!(job_id=%job.id,code=error.code,message=%error.message,"task failed");
                    let _ = store.update_job(
                        &job.project_id,
                        &job.id,
                        "failed",
                        store
                            .job(&job.project_id, &job.id)
                            .map(|j| j.completed)
                            .unwrap_or(0),
                        Some(&error.to_string()),
                        None,
                    );
                }
            }
            Ok(Err(error)) => tracing::warn!(%error,"scheduler unavailable"),
            Err(error) => tracing::error!(%error,"scheduler panic"),
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
fn prepare(
    store: &SqliteStore,
    job: &Job,
    resources: &dyn ReadResources,
    sources: &SourceService,
    cancelled: &Arc<AtomicBool>,
) -> Result<(PathBuf, WorkerPlan)> {
    let frozen = store.job_run(&job.project_id, &job.id)?;
    let directory = store.directory(&job.project_id)?;
    let staging = directory.join(".staging").join(&job.id);
    fs::create_dir_all(&staging).map_err(Error::io)?;
    let staging = staging.canonicalize().map_err(Error::io)?;
    if !staging.starts_with(&directory) {
        return Err(Error::new("WORKER_PATH_INVALID", "任务目录超出项目范围"));
    }
    let path = staging.join("plan.json");
    if path.exists() {
        if is_ranking_operator(&frozen.run.operator_id) {
            store.job_stage(
                &job.project_id,
                &job.id,
                &JobStage {
                    name: "restoring_input".into(),
                    ..Default::default()
                },
            )?;
        }
        let plan = worker::load_plan(&path)?;
        if plan.job_id != job.id
            || plan.total != job.total
            || studio_operators::registry()?.normalize(plan.run.clone())?
                != studio_operators::registry()?.normalize(frozen.run)?
        {
            return Err(Error::new("CHECKPOINT_INVALID", "任务计划不一致"));
        }
        return Ok((path, plan));
    }
    if studio_operators::registry()?
        .resolve(&frozen.run)?
        .population()
    {
        return crate::ranking::prepare(
            store,
            job,
            &staging,
            resources,
            sources,
            cancelled.clone(),
        );
    }
    store.update_job(&job.project_id, &job.id, "preparing", 0, None, None)?;
    crate::tool_inputs::validate_versions(store, &job.project_id, &frozen, sources)?;
    let input_path = staging.join("input.jsonl");
    let mut input = File::create(&input_path).map_err(Error::io)?;
    let mut after = None;
    let mut revisions = HashMap::<String, String>::new();
    let mut count: u64 = 0;
    loop {
        if store.job(&job.project_id, &job.id)?.status == "cancelled" {
            return Err(Error::new("CANCELLED", "任务已取消"));
        }
        let keys = store.job_inputs(&job.project_id, &job.id, after.as_ref())?;
        if keys.is_empty() {
            break;
        }
        let mut groups = std::collections::BTreeMap::<String, Vec<AssetKey>>::new();
        for key in &keys {
            groups
                .entry(key.source_id.clone())
                .or_default()
                .push(key.clone());
        }
        for (id, keys) in groups {
            let permit = sources.background(ReadClass::Index, 32 << 20, cancelled.clone())?;
            let source = store.source(&job.project_id, &id)?;
            let revision = frozen
                .source_versions
                .iter()
                .find(|v| v.source_id == id)
                .map(|v| v.catalog_revision.as_str());
            let items = permit.freeze_at(&source, &keys, revision)?;
            drop(permit);
            for mut item in items {
                if count.is_multiple_of(8)
                    && store.job(&job.project_id, &job.id)?.status == "cancelled"
                {
                    return Err(Error::new("CANCELLED", "字段准备已取消"));
                }
                let class = if frozen
                    .fields
                    .iter()
                    .any(|f| matches!(f, ScalarInput::OriginWidth))
                {
                    ReadClass::NativeQuery
                } else {
                    ReadClass::Index
                };
                let metadata = sources.background(
                    class,
                    if class == ReadClass::NativeQuery {
                        METADATA_MEMORY_BYTES
                    } else {
                        1 << 20
                    },
                    cancelled.clone(),
                )?;
                crate::tool_inputs::project_fields(
                    store,
                    &job.project_id,
                    &source,
                    &mut item,
                    &frozen,
                    &metadata,
                )?;
                if let Some(old) = revisions.insert(id.clone(), item.source_revision.clone())
                    && old != item.source_revision
                {
                    return Err(Error::new(
                        "SOURCE_CHANGED",
                        "固定输入期间来源更新，请创建新任务",
                    ));
                }
                serde_json::to_writer(&mut input, &item).map_err(Error::io)?;
                input.write_all(b"\n").map_err(Error::io)?;
                count += 1;
            }
        }
        after = keys.last().cloned();
    }
    if count != job.total {
        return Err(Error::new("INPUT_CHANGED", "固定输入数量不一致"));
    }
    input.sync_all().map_err(Error::io)?;
    crate::tool_inputs::validate_versions(store, &job.project_id, &frozen, sources)?;
    let plan = WorkerPlan {
        version: 1,
        job_id: job.id.clone(),
        input_sha256: worker::hash_file(&input_path)?,
        input_path: "input.jsonl".into(),
        output_path: "output.jsonl".into(),
        checkpoint_path: "checkpoint.json".into(),
        total: count,
        delay_ms: store.job_delay(&job.project_id, &job.id)?,
        run: frozen.run,
    };
    atomic_json(&path, &plan)?;
    let resolved = worker::load_plan(&path)?;
    Ok((path, resolved))
}
async fn execute(
    store: Arc<SqliteStore>,
    job: Job,
    resources: Arc<dyn ReadResources>,
    sources: Arc<SourceService>,
) -> Result<()> {
    let _attempt = ActiveAttempt::enter(&job)?;
    let _heartbeat = if is_ranking_operator(&job.operator) {
        let heartbeat_store = store.clone();
        let pid = job.project_id.clone();
        let jid = job.id.clone();
        Some(AttemptHeartbeat(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let store = heartbeat_store.clone();
                let pid = pid.clone();
                let jid = jid.clone();
                let _ = tokio::task::spawn_blocking(move || store.job_heartbeat(&pid, &jid)).await;
            }
        })))
    } else {
        None
    };
    let _project_lease = store.operation_lease(&job.project_id)?;
    let s = store.clone();
    let j = job.clone();
    let cancel = _attempt.1.clone();
    let preparation_resources = resources.clone();
    let (plan_path, plan) = tokio::task::spawn_blocking(move || {
        prepare(&s, &j, preparation_resources.as_ref(), &sources, &cancel)
    })
    .await
    .map_err(Error::io)??;
    if store.job(&job.project_id, &job.id)?.status == "cancelled" {
        return Ok(());
    }
    let final_path =
        store
            .directory(&job.project_id)?
            .join("artifacts")
            .join(if plan.version == 2 {
                format!("{}.ranking.sqlite", job.id)
            } else {
                format!("{}.jsonl", job.id)
            });
    let artifacts = final_path
        .parent()
        .ok_or_else(|| Error::invalid("成果位置无效"))?
        .canonicalize()
        .map_err(Error::io)?;
    if !artifacts.starts_with(store.directory(&job.project_id)?) {
        return Err(Error::new("ARTIFACT_PATH_INVALID", "成果目录超出项目范围"));
    }
    if final_path.exists() && final_path.canonicalize().map_err(Error::io)? != final_path {
        return Err(Error::new("ARTIFACT_PATH_INVALID", "成果位置不能是链接"));
    }
    if final_path.exists() {
        let s = store.clone();
        tokio::task::spawn_blocking(move || {
            crate::artifacts::publish(&s, &job, &plan, &final_path, None)
        })
        .await
        .map_err(Error::io)??;
        return Ok(());
    }
    let _computation_lease = if plan.version == 2 {
        store.job_stage(
            &job.project_id,
            &job.id,
            &JobStage {
                name: "waiting_resources".into(),
                ..Default::default()
            },
        )?;
        let resources = resources.clone();
        let flag = _attempt.1.clone();
        let input_path = plan.input_path.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                let input = studio_storage::ranking_tables::RankingInputTable::open(&input_path)?;
                let memory: u64 = input.meta("memory_bytes")?;
                resources.acquire(
                    ReadRequest {
                        class: ReadClass::NativeQuery,
                        priority: ReadPriority::Background,
                        bytes: memory,
                    },
                    &flag,
                )
            })
            .await
            .map_err(Error::io)??,
        )
    } else {
        None
    };
    store.update_job(
        &job.project_id,
        &job.id,
        "running",
        job.completed,
        None,
        None,
    )?;
    let failure_path = plan_path.with_file_name("worker-error.json");
    if failure_path.exists() {
        fs::remove_file(&failure_path).map_err(Error::io)?;
    }
    let stderr = File::create(plan_path.with_file_name(format!(
        "attempt-{}.log",
        store.job(&job.project_id, &job.id)?.attempt
    )))
    .map_err(Error::io)?;
    let mut command = Command::new(std::env::current_exe().map_err(Error::io)?);
    command
        .arg("worker")
        .arg("--plan")
        .arg(&plan_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(Error::io)?;
    #[cfg(windows)]
    let _process_group = ProcessGroup::attach(&child)?;
    let mut lines = FramedRead::new(
        child
            .stdout
            .take()
            .ok_or_else(|| Error::new("WORKER_ERROR", "执行器输出不可用"))?,
        LinesCodec::new_with_max_length(4096),
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(150));
    loop {
        tokio::select! {
            line=lines.next()=>{match line{
                Some(line)=>{let line=line.map_err(|_|Error::new("WORKER_PROTOCOL_ERROR","进度消息无效或超过 4 KiB"))?;record_progress(&store,&job,&line)?;},
                None=>break
            }},
            _=interval.tick()=>{if store.job(&job.project_id,&job.id)?.status=="cancelled"{child.kill().await.map_err(Error::io)?;return Ok(());}}
        }
    }
    let status = child.wait().await.map_err(Error::io)?;
    if store.job(&job.project_id, &job.id)?.status == "cancelled" {
        return Ok(());
    }
    if !status.success() {
        let failure: Option<worker::Failure> = fs::metadata(&failure_path)
            .ok()
            .filter(|m| m.len() <= 65_536)
            .and_then(|_| fs::read(&failure_path).ok())
            .and_then(|v| serde_json::from_slice(&v).ok());
        return Err(Error::new(
            "WORKER_FAILED",
            failure
                .map(|f| format!("{}：{}", f.code, f.message))
                .unwrap_or_else(|| format!("执行器退出码：{status}")),
        ));
    }
    let s = store.clone();
    let j = job.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let validated =
            worker::validate_once(&plan.output_path, &plan, &mut |name, completed, total| {
                if plan.version == 2 {
                    if s.job(&j.project_id, &j.id)?.status == "cancelled" {
                        return Err(Error::new("CANCELLED", "任务已取消"));
                    }
                    s.job_stage(
                        &j.project_id,
                        &j.id,
                        &JobStage {
                            name: name.into(),
                            completed,
                            total,
                            ..Default::default()
                        },
                    )?;
                }
                Ok(())
            })?;
        fs::rename(&plan.output_path, &final_path).map_err(Error::io)?;
        crate::artifacts::publish(&s, &j, &plan, &final_path, Some(validated))
    })
    .await
    .map_err(Error::io)??;
    Ok(())
}

#[cfg(windows)]
struct ProcessGroup(usize);

fn record_progress(store: &SqliteStore, job: &Job, line: &str) -> Result<()> {
    let progress: worker::Progress = serde_json::from_str(line).map_err(Error::io)?;
    if progress.version != 1 || progress.total != job.total || progress.completed > job.total {
        return Err(Error::new("WORKER_PROTOCOL_ERROR", "无效的任务进度"));
    }
    if let Some(stage) = progress.stage {
        if stage.name.len() > 80 || stage.completed > stage.total {
            return Err(Error::new("WORKER_PROTOCOL_ERROR", "无效任务阶段"));
        }
        store.job_stage(&job.project_id, &job.id, &stage)?;
    }
    store.update_job(
        &job.project_id,
        &job.id,
        "running",
        progress.completed,
        None,
        None,
    )?;
    Ok(())
}
#[cfg(windows)]
impl ProcessGroup {
    fn attach(child: &tokio::process::Child) -> Result<Self> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(Error::io(std::io::Error::last_os_error()));
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let process = child
                .raw_handle()
                .ok_or_else(|| Error::new("WORKER_ERROR", "执行器进程不可用"))?;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, process as _) == 0
            {
                CloseHandle(handle);
                return Err(Error::io(std::io::Error::last_os_error()));
            }
            Ok(Self(handle as usize))
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
