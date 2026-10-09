//! Original media for "save as" and file export jobs. Lakes stay read-only;
//! only the folder or file the user chose is written.
use crate::source_indexes::SourceIndexService;
use crate::sources::{SourceRead, SourceService};
use serde_json::json;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::{
    Media, MediaInput, MediaSource, MetadataAdapter, ReadCancellation, SourceAdapter,
};
use studio_domain::*;
use studio_operators::export::{EXPORT_OPERATOR, ExportMetadata, ExportParameters};
use studio_storage::SqliteStore;

mod files;
use files::{FailureLog, Sidecar, StagedSidecar, locate, write_new_file};
pub use files::{validate_destination, validate_save_path, write_file};

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
    let destination = validate_destination(context.store, &params.destination)?;
    let mut output = BufWriter::new(File::create(&plan.output_path).map_err(Error::io)?);
    let manifest_part = destination.join(".manifest.jsonl.studio-part");
    let mut manifest = if params.manifest {
        Some(BufWriter::new(
            File::create(&manifest_part).map_err(Error::io)?,
        ))
    } else {
        None
    };
    let mut failures = FailureLog::new(&destination);
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
            }
            if outcome.error.is_some() || outcome.metadata_error.is_some() {
                failures.record(&row)?;
            }
            if let Some(manifest) = manifest.as_mut() {
                serde_json::to_writer(&mut *manifest, &row).map_err(Error::io)?;
                manifest.write_all(b"\n").map_err(Error::io)?;
            }
        }
        failures.flush()?;
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
    let failed = failures.finish()?;
    if failed == 0 {
        return Ok(true);
    }
    Err(Error::new(
        "EXPORT_PARTIAL",
        format!(
            "{failed} / {ordinal} 项未完整导出（原图或元数据），已导出的文件保留在目标文件夹。原因见 export-errors.jsonl；重试会复用内容一致的文件。"
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
    let mut verified = Vec::new();
    let index = context
        .sources
        .background(ReadClass::Index, 4 << 20, context.cancelled.clone())?;
    for (at, item) in items.iter().enumerate() {
        let aid = &item.asset.key.asset_id;
        let identity = index
            .verify_media_identity(source, aid)
            .and_then(|identity| {
                if identity.bytes > MAX_ORIGINAL_BYTES {
                    return Err(too_large(identity.bytes));
                }
                Ok(identity.bytes)
            });
        let error = match identity {
            Ok(bytes) => {
                verified.push((at, bytes));
                None
            }
            Err(error) if error.code == "CANCELLED" => return Err(error),
            Err(error) => Some(error.message),
        };
        outcomes.push(Outcome {
            file: None,
            bytes: None,
            status: "failed",
            error,
            sidecar: None,
            metadata_error: None,
        });
    }
    drop(index);

    let (metadata, metadata_error) =
        if params.metadata == ExportMetadata::None || verified.is_empty() {
            (None, None)
        } else {
            match metadata_reader(context, source) {
                Ok(read) => (Some(read), None),
                Err(error) if error.code == "CANCELLED" => return Err(error),
                Err(error) => (None, Some(error.message)),
            }
        };
    let mut staged = Vec::with_capacity(items.len());
    staged.resize_with(items.len(), || None);
    // (item index, admitted bytes) for originals still to be written.
    let mut pending = Vec::new();
    for (at, bytes) in verified {
        let item = &items[at];
        let outcome = &mut outcomes[at];
        outcome.metadata_error = metadata_error.clone();
        let sidecar = if let Some(read) = &metadata {
            match sidecar(context, read, source, params, item) {
                Ok(sidecar) => Some(sidecar),
                Err(error) if error.code == "CANCELLED" => return Err(error),
                Err(error) => {
                    outcome.metadata_error = Some(error.message);
                    None
                }
            }
        } else {
            None
        };
        let target = match locate(
            destination,
            &item.planned,
            bytes,
            &item.asset.key.asset_id,
            sidecar.as_ref(),
        ) {
            Ok(target) => target,
            Err(error) => {
                outcome.error = Some(error.message);
                continue;
            }
        };
        if let Some(sidecar) = &sidecar {
            match StagedSidecar::prepare(destination, &target.name, sidecar) {
                Ok(file) => staged[at] = file,
                Err(error) => outcome.metadata_error = Some(error.message),
            }
        }
        outcome.file = Some(target.name);
        if target.existing {
            outcome.status = "existing";
            outcome.bytes = Some(bytes);
        } else {
            outcome.status = "written";
            pending.push((at, bytes.max(1)));
        }
    }
    drop(metadata);
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
        let media = match read.read_many(source, &inputs) {
            Ok(media) if media.items.len() == chunk.len() => media,
            result => {
                let error = result.err().unwrap_or_else(|| {
                    Error::new("SOURCE_FORMAT_ERROR", "来源返回的原图数量与请求不一致")
                });
                if error.code == "CANCELLED" {
                    return Err(error);
                }
                for &(at, _) in &chunk {
                    outcomes[at].file = None;
                    outcomes[at].status = "failed";
                    outcomes[at].error = Some(error.message.clone());
                }
                continue;
            }
        };
        for (&(index, _), media) in chunk.iter().zip(media.items) {
            let outcome = &mut outcomes[index];
            let written = media.and_then(|media| {
                let name = outcome.file.as_deref().expect("pending item has a name");
                write_new_file(&destination.join(name), &media.bytes)?;
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
    for (outcome, staged) in outcomes.iter_mut().zip(staged) {
        if outcome.file.is_none() {
            continue;
        }
        if let Some(staged) = staged {
            match staged.publish(destination) {
                Ok(name) => outcome.sidecar = Some(name),
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

/// Read expected sidecar content before choosing a collision-free image pair.
fn sidecar(
    context: &ExportContext<'_>,
    read: &SourceRead,
    source: &Source,
    params: &ExportParameters,
    item: &Item,
) -> Result<Sidecar> {
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
            return Ok(Sidecar {
                extension: "txt",
                bytes: None,
            });
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
    Ok(Sidecar {
        extension: if params.metadata == ExportMetadata::Tags {
            "txt"
        } else {
            "json"
        },
        bytes: Some(bytes),
    })
}
