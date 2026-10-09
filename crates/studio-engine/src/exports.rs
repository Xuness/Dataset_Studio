//! Original media for "save as" and file export jobs. Lakes stay read-only;
//! only the folder or file the user chose is written.
use crate::source_indexes::SourceIndexService;
use crate::sources::{SourceRead, SourceService};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::{
    Media, MediaInput, MediaSource, MetadataAdapter, ProjectRepository, ReadCancellation,
    SourceAdapter,
};
use studio_domain::*;
use studio_operators::export::{EXPORT_OPERATOR, ExportMetadata, ExportParameters};
use studio_storage::SqliteStore;

/// The whole media read class is 64 MiB; one original must fit in it.
pub const MAX_ORIGINAL_BYTES: u64 = 64 << 20;
const BATCH_BYTES: u64 = 32 << 20;
// Source adapters accept at most 16 objects per media batch.
const BATCH_ITEMS: usize = 16;

pub fn is_export(operator_id: &str) -> bool {
    operator_id == EXPORT_OPERATOR
}

fn too_large(bytes: u64) -> Error {
    Error::new(
        "READ_BUDGET_EXCEEDED",
        format!(
            "原图 {:.1} MiB，超过单次读取上限 64 MiB",
            bytes as f64 / 1048576.0
        ),
    )
}

/// One original under the caller's priority. Errors keep source codes.
pub fn read_original(
    sources: &SourceService,
    source: &Source,
    asset_id: &str,
    priority: ReadPriority,
    cancelled: ReadCancellation,
) -> Result<Media> {
    let bytes = sources
        .read(ReadClass::Index, priority, 4 << 20, cancelled.clone())?
        .verify_media_identity(source, asset_id)?
        .bytes;
    if bytes > MAX_ORIGINAL_BYTES {
        return Err(too_large(bytes));
    }
    let read = sources.read(ReadClass::Media, priority, bytes.max(1), cancelled)?;
    SourceAdapter::read(&read, source, asset_id)
}

/// Writes through a sibling temporary file so readers never see a partial image.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| Error::invalid("保存位置缺少文件名"))?
        .to_string_lossy();
    let part = path.with_file_name(format!(".{name}.studio-part"));
    let result = (|| {
        let mut file = File::create(&part)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&part, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result.map_err(|e| Error::new("EXPORT_WRITE_FAILED", format!("写入 {name} 失败：{e}")))
}

fn inside_lake(store: &SqliteStore, pid: &str, path: &Path) -> Result<bool> {
    for source in store.sources(pid)? {
        for root in [source.index_root, source.media_root].into_iter().flatten() {
            if root.canonicalize().is_ok_and(|root| path.starts_with(root)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// A single "save as" target: an absolute file path outside every lake.
pub fn validate_save_path(store: &SqliteStore, pid: &str, path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(Error::invalid("保存位置必须是绝对路径的文件"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::invalid("保存位置缺少文件夹"))?
        .canonicalize()
        .map_err(|e| Error::new("DESTINATION_UNAVAILABLE", format!("保存文件夹不可用：{e}")))?;
    if inside_lake(store, pid, &parent)? {
        return Err(Error::invalid("数据湖目录只读，请保存到其他位置"));
    }
    Ok(parent.join(path.file_name().expect("checked file name")))
}

/// Export folders must exist, be writable, and stay outside every lake.
pub fn validate_destination(store: &SqliteStore, pid: &str, destination: &str) -> Result<PathBuf> {
    let path = Path::new(destination);
    let unavailable =
        |e: std::io::Error| Error::new("DESTINATION_UNAVAILABLE", format!("导出文件夹不可用：{e}"));
    if !fs::metadata(path).map_err(unavailable)?.is_dir() {
        return Err(Error::invalid("导出目标必须是已存在的文件夹"));
    }
    let path = path.canonicalize().map_err(unavailable)?;
    if inside_lake(store, pid, &path)? {
        return Err(Error::invalid("数据湖目录只读，请选择其他导出文件夹"));
    }
    let probe = path.join(format!(".studio-export-probe-{}", new_id()));
    File::create(&probe).map_err(unavailable)?;
    let _ = fs::remove_file(probe);
    Ok(path)
}

enum Target {
    Existing(String),
    New(String),
}
/// An existing file is this image when its size matches and, for SHA-256
/// image identities, its content hash matches too.
fn same_image(path: &Path, bytes: u64, asset_id: &str) -> Result<bool> {
    let meta = fs::metadata(path).map_err(Error::io)?;
    if !meta.is_file() || meta.len() != bytes {
        return Ok(false);
    }
    if asset_id.len() != 64 || !asset_id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(true);
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut File::open(path).map_err(Error::io)?, &mut hash).map_err(Error::io)?;
    Ok(hex::encode(hash.finalize()).eq_ignore_ascii_case(asset_id))
}
/// Reuses a file holding the same image (resume); otherwise takes the first
/// free `_n` name.
fn locate(folder: &Path, planned: &str, bytes: u64, asset_id: &str) -> Result<Target> {
    let (stem, extension) = match planned.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (planned, String::new()),
    };
    for n in 1..=99 {
        let name = if n == 1 {
            planned.to_owned()
        } else {
            format!("{stem}_{n}{extension}")
        };
        match fs::symlink_metadata(folder.join(&name)) {
            Ok(_) if same_image(&folder.join(&name), bytes, asset_id)? => {
                return Ok(Target::Existing(name));
            }
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Target::New(name)),
            Err(e) => return Err(Error::io(e)),
        }
    }
    Err(Error::new(
        "EXPORT_NAME_CONFLICT",
        format!("{planned} 已有过多同名文件"),
    ))
}
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

struct Item {
    ordinal: u64,
    asset: Asset,
    planned: String,
}
struct Outcome {
    file: Option<String>,
    bytes: Option<u64>,
    status: &'static str,
    error: Option<String>,
    sidecar: Option<String>,
    metadata_error: Option<String>,
}

pub struct ExportContext<'a> {
    pub store: &'a SqliteStore,
    pub sources: &'a SourceService,
    pub indexes: &'a Arc<SourceIndexService>,
    pub job: &'a Job,
    pub cancelled: &'a Arc<AtomicBool>,
}
impl ExportContext<'_> {
    fn check_cancelled(&self) -> Result<bool> {
        Ok(self.cancelled.load(Ordering::Acquire)
            || self.store.job(&self.job.project_id, &self.job.id)?.status == "cancelled")
    }
}

/// Writes the deterministic plan rows to `output_path` and the files to the
/// destination. Returns `false` when the job was cancelled.
pub fn run(context: &ExportContext<'_>, plan: &WorkerPlan) -> Result<bool> {
    let params = ExportParameters::parse(&plan.run.parameters)?;
    let operator = studio_operators::registry()?.resolve(&plan.run)?;
    let destination =
        validate_destination(context.store, &context.job.project_id, &params.destination)?;
    let mut output = BufWriter::new(File::create(&plan.output_path).map_err(Error::io)?);
    let manifest_part = destination.join(".manifest.jsonl.studio-part");
    let mut manifest = if params.manifest {
        Some(BufWriter::new(
            File::create(&manifest_part).map_err(Error::io)?,
        ))
    } else {
        None
    };
    let mut failures = Vec::<Value>::new();
    let mut sources = HashMap::<String, Source>::new();
    let mut batch = Vec::<FrozenInput>::new();
    let mut batch_bytes = 0;
    let mut ordinal = 0u64;
    let mut completed = 0u64;
    let mut flush = |batch: &mut Vec<FrozenInput>, first: u64| -> Result<bool> {
        if batch.is_empty() {
            return Ok(true);
        }
        if context.check_cancelled()? {
            return Ok(false);
        }
        let sid = batch[0].asset.key.source_id.clone();
        if !sources.contains_key(&sid) {
            let source = context.store.source(&context.job.project_id, &sid)?;
            sources.insert(sid.clone(), source);
        }
        let source = &sources[&sid];
        let items = batch
            .drain(..)
            .enumerate()
            .map(|(i, input)| Item {
                ordinal: first + i as u64,
                planned: studio_operators::export::planned_file_name(
                    first + i as u64,
                    &input.asset,
                ),
                asset: input.asset,
            })
            .collect::<Vec<_>>();
        let outcomes = export_batch(context, source, &destination, &params, &items)?;
        for (item, outcome) in items.iter().zip(outcomes) {
            let mut row = json!({
                "ordinal": item.ordinal,
                "file": outcome.file,
                "status": outcome.status,
                "asset": item.asset.key,
                "name": item.asset.name,
                "source_name": item.asset.source_name,
                "bytes": outcome.bytes,
            });
            if let Some(sidecar) = &outcome.sidecar {
                row["metadata_file"] = json!(sidecar);
            }
            if let Some(error) = &outcome.metadata_error {
                row["metadata_error"] = json!(error);
            }
            if let Some(error) = &outcome.error {
                row["error"] = json!(error);
                failures.push(row.clone());
            }
            if let Some(manifest) = manifest.as_mut() {
                serde_json::to_writer(&mut *manifest, &row).map_err(Error::io)?;
                manifest.write_all(b"\n").map_err(Error::io)?;
            }
        }
        completed += items.len() as u64;
        context.store.update_job(
            &context.job.project_id,
            &context.job.id,
            "running",
            completed,
            None,
            None,
        )?;
        Ok(true)
    };
    let input = BufReader::new(File::open(&plan.input_path).map_err(Error::io)?);
    let mut first = 0;
    for line in input.lines() {
        let item: FrozenInput =
            serde_json::from_str(&line.map_err(Error::io)?).map_err(Error::io)?;
        let row = operator.row(&item, ordinal, &plan.run.parameters)?;
        serde_json::to_writer(&mut output, &row).map_err(Error::io)?;
        output.write_all(b"\n").map_err(Error::io)?;
        // Planning only; each original's admitted size comes from its identity.
        let bytes = item.asset.bytes.min(MAX_ORIGINAL_BYTES);
        if !batch.is_empty()
            && (batch[0].asset.key.source_id != item.asset.key.source_id
                || batch.len() >= BATCH_ITEMS
                || batch_bytes + bytes > BATCH_BYTES)
        {
            if !flush(&mut batch, first)? {
                return Ok(false);
            }
            batch_bytes = 0;
        }
        if batch.is_empty() {
            first = ordinal;
        }
        batch_bytes += bytes;
        batch.push(item);
        ordinal += 1;
    }
    if !flush(&mut batch, first)? {
        return Ok(false);
    }
    output.flush().map_err(Error::io)?;
    output.get_ref().sync_all().map_err(Error::io)?;
    if let Some(mut manifest) = manifest {
        manifest.flush().map_err(Error::io)?;
        drop(manifest);
        fs::rename(&manifest_part, destination.join("manifest.jsonl")).map_err(Error::io)?;
    }
    let errors = destination.join("export-errors.jsonl");
    if failures.is_empty() {
        let _ = fs::remove_file(errors);
        return Ok(true);
    }
    let mut text = String::new();
    for row in &failures {
        text.push_str(&row.to_string());
        text.push('\n');
    }
    write_file(&errors, text.as_bytes())?;
    Err(Error::new(
        "EXPORT_PARTIAL",
        format!(
            "{} / {} 项未能导出，已导出的文件保留在目标文件夹。原因见 export-errors.jsonl；重试会跳过已写入的文件。",
            failures.len(),
            ordinal
        ),
    ))
}

fn export_batch(
    context: &ExportContext<'_>,
    source: &Source,
    destination: &Path,
    params: &ExportParameters,
    items: &[Item],
) -> Result<Vec<Outcome>> {
    let mut outcomes = Vec::with_capacity(items.len());
    // (item index, admitted bytes) for originals still to be written.
    let mut pending = Vec::new();
    let index = context
        .sources
        .background(ReadClass::Index, 4 << 20, context.cancelled.clone())?;
    for (at, item) in items.iter().enumerate() {
        let aid = &item.asset.key.asset_id;
        let target = index
            .verify_media_identity(source, aid)
            .and_then(|identity| {
                if identity.bytes > MAX_ORIGINAL_BYTES {
                    return Err(too_large(identity.bytes));
                }
                Ok((
                    identity.bytes,
                    locate(destination, &item.planned, identity.bytes, aid)?,
                ))
            });
        let (file, bytes, status, error) = match target {
            Ok((bytes, Target::Existing(name))) => (Some(name), Some(bytes), "existing", None),
            Ok((bytes, Target::New(name))) => {
                pending.push((at, bytes.max(1)));
                (Some(name), None, "written", None)
            }
            Err(error) if error.code == "CANCELLED" => return Err(error),
            Err(error) => (None, None, "failed", Some(error.message)),
        };
        outcomes.push(Outcome {
            file,
            bytes,
            status,
            error,
            sidecar: None,
            metadata_error: None,
        });
    }
    drop(index);
    // Admit media reads in chunks that fit the 64 MiB media class.
    let mut chunks: Vec<Vec<(usize, u64)>> = Vec::new();
    for entry in pending {
        match chunks.last_mut() {
            Some(chunk) if chunk.iter().map(|(_, b)| b).sum::<u64>() + entry.1 <= BATCH_BYTES => {
                chunk.push(entry)
            }
            _ => chunks.push(vec![entry]),
        }
    }
    for chunk in chunks {
        let bytes = chunk.iter().map(|(_, b)| b).sum();
        let read =
            context
                .sources
                .background(ReadClass::Media, bytes, context.cancelled.clone())?;
        let inputs = chunk
            .iter()
            .map(|&(at, bytes)| MediaInput {
                asset_id: items[at].asset.key.asset_id.clone(),
                cancelled: context.cancelled.clone(),
                deadline: None,
                byte_limit: bytes,
            })
            .collect::<Vec<_>>();
        let media = read.read_many(source, &inputs)?;
        for (&(index, _), media) in chunk.iter().zip(media.items) {
            let outcome = &mut outcomes[index];
            let written = media.and_then(|media| {
                let name = outcome.file.as_deref().expect("pending item has a name");
                write_file(&destination.join(name), &media.bytes)?;
                outcome.bytes = Some(media.bytes.len() as u64);
                Ok(())
            });
            if let Err(error) = written {
                if error.code == "CANCELLED" {
                    return Err(error);
                }
                outcome.file = None;
                outcome.status = "failed";
                outcome.error = Some(error.message);
            }
        }
    }
    if params.metadata != ExportMetadata::None {
        let read = metadata_reader(context, source)?;
        for (item, outcome) in items.iter().zip(outcomes.iter_mut()) {
            let Some(file) = outcome.file.clone() else {
                continue;
            };
            match sidecar(context, &read, source, destination, params, item, &file) {
                Ok(name) => outcome.sidecar = name,
                Err(error) if error.code == "CANCELLED" => return Err(error),
                Err(error) => outcome.metadata_error = Some(error.message),
            }
        }
    }
    Ok(outcomes)
}

fn metadata_reader(context: &ExportContext<'_>, source: &Source) -> Result<SourceRead> {
    loop {
        let index =
            context
                .sources
                .background(ReadClass::Index, 32 << 20, context.cancelled.clone())?;
        if context.indexes.prepare_identity_index(source, &index)? {
            break;
        }
        drop(index);
        if context.check_cancelled()? {
            return Err(Error::new("CANCELLED", "导出已取消"));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    context.sources.background(
        ReadClass::NativeQuery,
        METADATA_MEMORY_BYTES,
        context.cancelled.clone(),
    )
}

/// Writes `<stem>.txt` (tags) or `<stem>.json` (records with their origin
/// observation). Returns the sidecar name, or `None` when no tags exist.
fn sidecar(
    context: &ExportContext<'_>,
    read: &SourceRead,
    source: &Source,
    destination: &Path,
    params: &ExportParameters,
    item: &Item,
    file: &str,
) -> Result<Option<String>> {
    let name = format!(
        "{}.{}",
        stem(file),
        if params.metadata == ExportMetadata::Tags {
            "txt"
        } else {
            "json"
        }
    );
    if destination.join(&name).is_file() {
        return Ok(Some(name));
    }
    let aid = &item.asset.key.asset_id;
    let request = |observation_id: Option<String>| MetadataRequest {
        observation_id,
        cursor: None,
        limit: Some(20),
        version: None,
    };
    let overview =
        read.metadata_cancelled(source, aid, request(None), context.cancelled.clone())?;
    let mut records = Vec::new();
    for record in &overview.records {
        let observation = match &record.origin_observation_id {
            Some(id) => read
                .observations_cancelled(
                    source,
                    aid,
                    &record.record_id,
                    request(Some(id.clone())),
                    context.cancelled.clone(),
                )?
                .items
                .into_iter()
                .next(),
            None => None,
        };
        records.push((record.clone(), observation));
    }
    let bytes = if params.metadata == ExportMetadata::Tags {
        let tags = records
            .iter()
            .filter_map(|(_, observation)| observation.as_ref())
            .flat_map(|o| &o.fields)
            .find_map(|field| match (&field.name[..], &field.value) {
                ("tags", Some(MetadataValue::Tags(tags))) if !tags.is_empty() => Some(tags),
                _ => None,
            });
        let Some(tags) = tags else {
            return Ok(None);
        };
        params.tag_style.format(tags).into_bytes()
    } else {
        let value = json!({
            "schema_version": 1,
            "asset": item.asset,
            "metadata_version": studio_protocol::ReadVersion::from(overview.version),
            "records": records
                .into_iter()
                .map(|(record, observation)| json!({
                    "record": studio_protocol::AssetRecord::from(record),
                    "origin_observation": observation.map(studio_protocol::Observation::from),
                }))
                .collect::<Vec<_>>(),
            "records_truncated": overview.next_cursor.is_some(),
        });
        serde_json::to_vec_pretty(&value).map_err(Error::io)?
    };
    write_file(&destination.join(&name), &bytes)?;
    Ok(Some(name))
}
