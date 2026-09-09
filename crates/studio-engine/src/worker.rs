use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};
use studio_domain::*;
use studio_storage::atomic_json;

/// A process-local proof for the exact output just validated before atomic rename.
/// Recovery has no proof and must perform validation again.
pub struct ValidatedOutput {
    file: File,
    input: File,
    digest: String,
    bytes: u64,
    modified: std::time::SystemTime,
    job_id: String,
    input_sha256: String,
    total: u64,
    run: OperatorRun,
}
impl ValidatedOutput {
    pub fn digest(&mut self, path: &Path, plan: &WorkerPlan) -> Result<&str> {
        let published = protected_read(path, false)?;
        let metadata = published.metadata().map_err(Error::io)?;
        if self.job_id != plan.job_id
            || self.input_sha256 != plan.input_sha256
            || self.total != plan.total
            || self.run != plan.run
            || metadata.len() != self.bytes
            || metadata.modified().map_err(Error::io)? != self.modified
            || file_identity(&self.file)? != file_identity(&published)?
            || file_identity(&self.input)?
                != file_identity(&File::open(&plan.input_path).map_err(Error::io)?)?
        {
            return Err(Error::new("OUTPUT_CHANGED", "校验后的成果或执行计划已变化"));
        }
        self.file = published;
        Ok(&self.digest)
    }
}
fn protected_read(path: &Path, allow_rename: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
        options.share_mode(FILE_SHARE_READ | if allow_rename { FILE_SHARE_DELETE } else { 0 });
    }
    #[cfg(not(windows))]
    let _ = allow_rename;
    options.open(path).map_err(Error::io)
}
#[cfg(windows)]
fn file_identity(file: &File) -> Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    // The handle belongs to a live File; the output structure is initialized by Win32.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(Error::io(std::io::Error::last_os_error()));
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}
#[cfg(unix)]
fn file_identity(file: &File) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(Error::io)?;
    Ok((metadata.dev(), metadata.ino()))
}
pub fn validate_once(
    path: &Path,
    plan: &WorkerPlan,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
) -> Result<ValidatedOutput> {
    let file = protected_read(path, true)?;
    let input = protected_read(&plan.input_path, false)?;
    let before = file.metadata().map_err(Error::io)?;
    let digest = if plan.version == 2 {
        crate::ranking::validate_output_progress(path, plan, progress)?
    } else {
        validate_output(path, plan)?
    };
    let after = fs::metadata(path).map_err(Error::io)?;
    if before.len() != after.len()
        || before.modified().map_err(Error::io)? != after.modified().map_err(Error::io)?
        || file_identity(&file)? != file_identity(&File::open(path).map_err(Error::io)?)?
    {
        return Err(Error::new("OUTPUT_CHANGED", "成果在校验期间被修改"));
    }
    Ok(ValidatedOutput {
        file,
        input,
        digest,
        bytes: after.len(),
        modified: after.modified().map_err(Error::io)?,
        job_id: plan.job_id.clone(),
        input_sha256: plan.input_sha256.clone(),
        total: plan.total,
        run: plan.run.clone(),
    })
}

#[derive(Serialize, Deserialize, Default)]
struct Checkpoint {
    job_id: String,
    input_offset: u64,
    output_offset: u64,
    completed: u64,
}
#[derive(Serialize, Deserialize)]
pub struct Progress {
    pub version: u32,
    pub completed: u64,
    pub total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<JobStage>,
}
pub fn report_progress(completed: u64, total: u64, stage: Option<JobStage>) -> Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(
        &mut output,
        &Progress {
            version: 1,
            completed,
            total,
            stage,
        },
    )
    .map_err(Error::io)?;
    output.write_all(b"\n").map_err(Error::io)?;
    output.flush().map_err(Error::io)
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
}
pub fn run_reported(plan: &Path) -> Result<()> {
    let result = run(plan);
    if let Err(error) = &result {
        // Separate, bounded status material keeps human-readable failures out of stdout progress.
        let _ = atomic_json(
            &plan.with_file_name("worker-error.json"),
            &Failure {
                code: error.code.into(),
                message: error.message.chars().take(4096).collect(),
            },
        );
    }
    result
}
pub fn hash_file(path: &Path) -> Result<String> {
    hash_file_progress(path, &mut |_, _| Ok(()))
}
pub fn hash_file_progress(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64) -> Result<()>,
) -> Result<String> {
    let mut file = File::open(path).map_err(Error::io)?;
    let total = file.metadata().map_err(Error::io)?.len();
    let mut read = 0;
    let mut last = std::time::Instant::now();
    progress(0, total)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(Error::io)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        read += n as u64;
        if last.elapsed() >= std::time::Duration::from_millis(500) {
            progress(read, total)?;
            last = std::time::Instant::now();
        }
    }
    progress(read, total)?;
    Ok(hex::encode(hash.finalize()))
}
pub fn load_plan(path: &Path) -> Result<WorkerPlan> {
    load_plan_progress(path, &mut |_, _, _| Ok(()))
}
fn load_plan_progress(
    path: &Path,
    progress: &mut dyn FnMut(&WorkerPlan, u64, u64) -> Result<()>,
) -> Result<WorkerPlan> {
    if fs::metadata(path).map_err(Error::io)?.len() > 1024 * 1024 {
        return Err(Error::new("WORKER_PROTOCOL_ERROR", "执行计划超过 1 MiB"));
    }
    let mut plan: WorkerPlan =
        serde_json::from_slice(&fs::read(path).map_err(Error::io)?).map_err(Error::io)?;
    if !matches!(plan.version, 1 | 2) {
        return Err(Error::new("WORKER_VERSION_MISMATCH", "执行器协议不兼容"));
    }
    validate_id(&plan.job_id)?;
    let population = studio_operators::registry()?
        .resolve(&plan.run)?
        .population();
    if population != (plan.version == 2) {
        return Err(Error::new(
            "WORKER_VERSION_MISMATCH",
            "执行计划与算子类型不一致",
        ));
    }
    let root = path
        .parent()
        .ok_or_else(|| Error::invalid("任务位置无效"))?
        .canonicalize()
        .map_err(Error::io)?;
    for (field, name) in [
        (
            &mut plan.input_path,
            if population {
                "input.sqlite"
            } else {
                "input.jsonl"
            },
        ),
        (
            &mut plan.output_path,
            if population {
                "output.sqlite"
            } else {
                "output.jsonl"
            },
        ),
        (&mut plan.checkpoint_path, "checkpoint.json"),
    ] {
        if field.as_path() != Path::new(name) {
            return Err(Error::new(
                "WORKER_PATH_INVALID",
                "任务资源必须使用受控相对路径",
            ));
        }
        let resolved = root.join(name);
        if resolved.exists() && resolved.canonicalize().map_err(Error::io)? != resolved {
            return Err(Error::new(
                "WORKER_PATH_INVALID",
                "任务资源不能指向其他位置",
            ));
        }
        *field = resolved;
    }
    if hash_file_progress(&plan.input_path, &mut |completed, total| {
        progress(&plan, completed, total)
    })? != plan.input_sha256
    {
        return Err(Error::new("INPUT_CHANGED", "固定输入内容已改变"));
    }
    Ok(plan)
}

pub fn run(plan_path: &Path) -> Result<()> {
    let plan = load_plan_progress(plan_path, &mut |plan, completed, total| {
        if plan.version == 2 {
            report_progress(
                0,
                plan.total,
                Some(JobStage {
                    name: "input_checksum".into(),
                    completed,
                    total,
                    ..Default::default()
                }),
            )?;
        }
        Ok(())
    })?;
    let operator = studio_operators::registry()?.resolve(&plan.run)?;
    if operator.population() {
        return crate::ranking::run(&plan);
    }
    if plan.version != 1 {
        return Err(Error::new("WORKER_VERSION_MISMATCH", "执行器协议不兼容"));
    }
    let mut checkpoint: Checkpoint = if plan.checkpoint_path.exists() {
        if fs::metadata(&plan.checkpoint_path)
            .map_err(Error::io)?
            .len()
            > 65536
        {
            return Err(Error::new("CHECKPOINT_INVALID", "检查点超过 64 KiB"));
        }
        serde_json::from_slice(&fs::read(&plan.checkpoint_path).map_err(Error::io)?)
            .map_err(Error::io)?
    } else {
        Checkpoint {
            job_id: plan.job_id.clone(),
            ..Default::default()
        }
    };
    if checkpoint.job_id != plan.job_id || checkpoint.completed > plan.total {
        return Err(Error::new("CHECKPOINT_INVALID", "检查点与任务不匹配"));
    }
    let mut input = File::open(&plan.input_path).map_err(Error::io)?;
    if checkpoint.input_offset > input.metadata().map_err(Error::io)?.len() {
        return Err(Error::new("CHECKPOINT_INVALID", "输入检查点越界"));
    }
    input
        .seek(SeekFrom::Start(checkpoint.input_offset))
        .map_err(Error::io)?;
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&plan.output_path)
        .map_err(Error::io)?;
    if checkpoint.output_offset > output.metadata().map_err(Error::io)?.len() {
        return Err(Error::new(
            "CHECKPOINT_INVALID",
            "成果文件缺少已提交的检查点",
        ));
    }
    output
        .set_len(checkpoint.output_offset)
        .map_err(Error::io)?;
    output
        .seek(SeekFrom::Start(checkpoint.output_offset))
        .map_err(Error::io)?;
    let mut input = BufReader::new(input);
    let mut line = String::new();
    let mut stdout = std::io::stdout().lock();
    loop {
        line.clear();
        let bytes = bounded_line(&mut input, &mut line)?;
        if bytes == 0 {
            break;
        }
        let item: FrozenInput = serde_json::from_str(&line).map_err(Error::io)?;
        let row = operator.row(&item, checkpoint.completed, &plan.run.parameters)?;
        serde_json::to_writer(&mut output, &row).map_err(Error::io)?;
        output.write_all(b"\n").map_err(Error::io)?;
        checkpoint.input_offset += bytes as u64;
        checkpoint.completed += 1;
        if checkpoint.completed > plan.total {
            return Err(Error::new("INPUT_CHANGED", "输入数量超过固定快照"));
        }
        if checkpoint.completed.is_multiple_of(8) || checkpoint.completed == plan.total {
            output.flush().map_err(Error::io)?;
            output.sync_all().map_err(Error::io)?;
            checkpoint.output_offset = output.stream_position().map_err(Error::io)?;
            atomic_json(&plan.checkpoint_path, &checkpoint)?;
            serde_json::to_writer(
                &mut stdout,
                &Progress {
                    version: 1,
                    completed: checkpoint.completed,
                    total: plan.total,
                    stage: None,
                },
            )
            .map_err(Error::io)?;
            stdout.write_all(b"\n").map_err(Error::io)?;
            stdout.flush().map_err(Error::io)?;
        }
        if plan.delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(plan.delay_ms.min(1000)));
        }
    }
    if checkpoint.completed != plan.total {
        return Err(Error::new("INPUT_CHANGED", "固定输入不完整"));
    }
    Ok(())
}

pub fn validate_output(path: &Path, plan: &WorkerPlan) -> Result<String> {
    let operator = studio_operators::registry()?.resolve(&plan.run)?;
    if operator.population() {
        return crate::ranking::validate_output(path, plan);
    }
    if hash_file(&plan.input_path)? != plan.input_sha256 {
        return Err(Error::new("INPUT_CHANGED", "发布前输入验证失败"));
    }
    let mut input = BufReader::new(File::open(&plan.input_path).map_err(Error::io)?);
    let mut output = BufReader::new(File::open(path).map_err(Error::io)?);
    let mut count = 0;
    let mut line = String::new();
    let mut input_line = String::new();
    let mut hash = Sha256::new();
    loop {
        line.clear();
        input_line.clear();
        if bounded_line(&mut output, &mut line)? == 0 {
            break;
        }
        if !line.ends_with('\n') || bounded_line(&mut input, &mut input_line)? == 0 {
            return Err(Error::new("ARTIFACT_INVALID", "成果行不完整"));
        }
        let expected: FrozenInput = serde_json::from_str(&input_line).map_err(Error::io)?;
        let value: serde_json::Value = serde_json::from_str(&line).map_err(Error::io)?;
        if value != operator.row(&expected, count, &plan.run.parameters)? {
            return Err(Error::new("ARTIFACT_INVALID", "成果与任务输入不一致"));
        }
        count += 1;
        hash.update(line.as_bytes());
    }
    let mut extra = [0; 1];
    if count != plan.total || input.read(&mut extra).map_err(Error::io)? != 0 {
        return Err(Error::new("ARTIFACT_INVALID", "成果数量不一致"));
    }
    Ok(hex::encode(hash.finalize()))
}
fn bounded_line(reader: &mut impl BufRead, line: &mut String) -> Result<usize> {
    let count = reader
        .take(1024 * 1024 + 1)
        .read_line(line)
        .map_err(Error::io)?;
    if count > 1024 * 1024 {
        return Err(Error::new(
            "WORKER_PROTOCOL_ERROR",
            "输入输出单行超过 1 MiB",
        ));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_proof_survives_rename_but_rejects_other_files_and_plans() {
        let directory = tempfile::tempdir().unwrap();
        let input_path = directory.path().join("input.jsonl");
        let output_path = directory.path().join("output.jsonl");
        let published = directory.path().join("published.jsonl");
        let frozen: FrozenInput = serde_json::from_value(serde_json::json!({
            "asset": {"key":{"source_id":new_id(),"asset_id":"00".repeat(32)},"name":"fixture","bytes":123,"extension":"png","source_name":"fixture"},
            "source_revision":"fixture-v1","fields":[]
        })).unwrap();
        let run = OperatorRun {
            operator_id: "core.manifest".into(),
            operator_version: 1,
            parameters_version: 1,
            parameters: serde_json::json!({}),
        };
        let value = studio_operators::registry()
            .unwrap()
            .resolve(&run)
            .unwrap()
            .row(&frozen, 0, &run.parameters)
            .unwrap();
        fs::write(
            &input_path,
            format!("{}\n", serde_json::to_string(&frozen).unwrap()),
        )
        .unwrap();
        fs::write(
            &output_path,
            format!("{}\n", serde_json::to_string(&value).unwrap()),
        )
        .unwrap();
        let plan = WorkerPlan {
            version: 1,
            job_id: new_id(),
            input_path: input_path.clone(),
            input_sha256: hash_file(&input_path).unwrap(),
            output_path: output_path.clone(),
            checkpoint_path: directory.path().join("checkpoint.json"),
            total: 1,
            delay_ms: 0,
            run,
        };
        let mut proof = validate_once(&output_path, &plan, &mut |_, _, _| Ok(())).unwrap();
        fs::rename(&output_path, &published).unwrap();
        assert_eq!(
            proof.digest(&published, &plan).unwrap(),
            hash_file(&published).unwrap()
        );
        #[cfg(windows)]
        {
            assert!(fs::write(&input_path, b"changed").is_err());
            assert!(fs::write(&published, b"changed").is_err());
        }
        let mut wrong_plan = plan.clone();
        wrong_plan.total = 2;
        assert_eq!(
            proof.digest(&published, &wrong_plan).unwrap_err().code,
            "OUTPUT_CHANGED"
        );
        let other = directory.path().join("other.jsonl");
        fs::copy(&published, &other).unwrap();
        {
            let file = OpenOptions::new().write(true).open(&other).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(proof.modified))
                .unwrap();
        }
        assert_eq!(
            proof.digest(&other, &plan).unwrap_err().code,
            "OUTPUT_CHANGED"
        );
        drop(proof);
        let mut corrupt = value;
        corrupt["ordinal"] = serde_json::json!(99);
        fs::write(
            &published,
            format!("{}\n", serde_json::to_string(&corrupt).unwrap()),
        )
        .unwrap();
        assert!(validate_once(&published, &plan, &mut |_, _, _| Ok(())).is_err());
    }
}
