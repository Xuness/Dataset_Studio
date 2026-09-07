use crate::worker;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
};
use studio_application::SourceAdapter;
use studio_domain::*;
use studio_sources::SourceRouter;
use studio_storage::{SqliteStore, atomic_json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

pub async fn scheduler(store: Arc<SqliteStore>) {
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
                if let Err(error) = execute(store.clone(), job.clone()).await {
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
fn prepare(store: &SqliteStore, job: &Job) -> Result<(PathBuf, WorkerPlan)> {
    let directory = store.directory(&job.project_id)?;
    let staging = directory.join(".staging").join(&job.id);
    fs::create_dir_all(&staging).map_err(Error::io)?;
    let staging = staging.canonicalize().map_err(Error::io)?;
    if !staging.starts_with(&directory) {
        return Err(Error::new("WORKER_PATH_INVALID", "任务目录超出项目范围"));
    }
    let path = staging.join("plan.json");
    if path.exists() {
        let plan = worker::load_plan(&path)?;
        if plan.job_id != job.id || plan.total != job.total {
            return Err(Error::new("CHECKPOINT_INVALID", "任务计划不一致"));
        }
        return Ok((path, plan));
    }
    store.update_job(&job.project_id, &job.id, "preparing", 0, None, None)?;
    let input_path = staging.join("input.jsonl");
    let mut input = File::create(&input_path).map_err(Error::io)?;
    let mut after = None;
    let mut revisions = HashMap::<String, String>::new();
    let mut count = 0;
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
            let source = store.source(&job.project_id, &id)?;
            let items = SourceRouter.freeze(&source, &keys)?;
            for item in items {
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
    let plan = WorkerPlan {
        version: 1,
        job_id: job.id.clone(),
        input_sha256: worker::hash_file(&input_path)?,
        input_path: "input.jsonl".into(),
        output_path: "output.jsonl".into(),
        checkpoint_path: "checkpoint.json".into(),
        total: count,
        delay_ms: store.job_delay(&job.project_id, &job.id)?,
    };
    atomic_json(&path, &plan)?;
    let resolved = worker::load_plan(&path)?;
    Ok((path, resolved))
}
async fn execute(store: Arc<SqliteStore>, job: Job) -> Result<()> {
    let _project_lease = store.operation_lease(&job.project_id)?;
    let s = store.clone();
    let j = job.clone();
    let (plan_path, plan) = tokio::task::spawn_blocking(move || prepare(&s, &j))
        .await
        .map_err(Error::io)??;
    if store.job(&job.project_id, &job.id)?.status == "cancelled" {
        return Ok(());
    }
    let final_path = store
        .directory(&job.project_id)?
        .join("artifacts")
        .join(format!("{}.jsonl", job.id));
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
        let hash = worker::validate_output(&final_path, &plan)?;
        atomic_json(
            &final_path.with_extension("manifest.json"),
            &serde_json::json!({"schema_version":1,"job_id":job.id,"rows":job.total,"sha256":hash,"input_sha256":plan.input_sha256,"input_scope":job.input_scope,"operator":"core.manifest","operator_version":1}),
        )?;
        store.update_job(
            &job.project_id,
            &job.id,
            "succeeded",
            job.total,
            None,
            Some(&format!("artifacts/{}.jsonl", job.id)),
        )?;
        return Ok(());
    }
    store.update_job(
        &job.project_id,
        &job.id,
        "running",
        job.completed,
        None,
        None,
    )?;
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
    let mut lines = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| Error::new("WORKER_ERROR", "执行器输出不可用"))?,
    )
    .lines();
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(150));
    loop {
        tokio::select! {
            line=lines.next_line()=>{match line.map_err(Error::io)?{
                Some(line)=>{let progress:worker::Progress=serde_json::from_str(&line).map_err(Error::io)?;if progress.total!=job.total||progress.completed>job.total{return Err(Error::new("WORKER_PROTOCOL_ERROR","无效的任务进度"));}store.update_job(&job.project_id,&job.id,"running",progress.completed,None,None)?;},
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
        return Err(Error::new(
            "WORKER_FAILED",
            format!("执行器退出码：{status}"),
        ));
    }
    let s = store.clone();
    let j = job.clone();
    tokio::task::spawn_blocking(move||->Result<()>{
        let hash=worker::validate_output(&plan.output_path,&plan)?;
        fs::rename(&plan.output_path,&final_path).map_err(Error::io)?;
        atomic_json(&final_path.with_extension("manifest.json"),&serde_json::json!({"schema_version":1,"job_id":j.id,"rows":j.total,"sha256":hash,"input_sha256":plan.input_sha256,"input_scope":j.input_scope,"operator":"core.manifest","operator_version":1}))?;
        s.update_job(&j.project_id,&j.id,"succeeded",j.total,None,Some(&format!("artifacts/{}.jsonl",j.id)))?;Ok(())
    }).await.map_err(Error::io)??;
    Ok(())
}

#[cfg(windows)]
struct ProcessGroup(usize);
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
