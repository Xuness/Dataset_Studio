use crate::worker;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};
use studio_application::ArtifactRepository;
use studio_domain::*;
use studio_storage::{SqliteStore, atomic_json};

pub fn controlled_path(store: &SqliteStore, pid: &str, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    let parts = path.components().collect::<Vec<_>>();
    if parts.len() != 2
        || parts[0] != Component::Normal("artifacts".as_ref())
        || !matches!(parts[1], Component::Normal(_))
    {
        return Err(Error::new(
            "ARTIFACT_PATH_INVALID",
            "成果必须位于项目 artifacts 目录",
        ));
    }
    let directory = store.directory(pid)?;
    let parent = directory
        .join("artifacts")
        .canonicalize()
        .map_err(Error::io)?;
    if !parent.starts_with(&directory) {
        return Err(Error::new("ARTIFACT_PATH_INVALID", "成果目录超出项目范围"));
    }
    let full = directory.join(path);
    if full.exists() && full.canonicalize().map_err(Error::io)? != full {
        return Err(Error::new("ARTIFACT_PATH_INVALID", "成果文件不能是链接"));
    }
    Ok(full)
}
fn rows(path: &Path, mut consume: impl FnMut(serde_json::Value) -> Result<()>) -> Result<u64> {
    let mut reader = BufReader::new(File::open(path).map_err(Error::io)?);
    let mut count = 0;
    loop {
        let mut line = String::new();
        let n = (&mut reader)
            .take(1024 * 1024 + 1)
            .read_line(&mut line)
            .map_err(Error::io)?;
        if n == 0 {
            break;
        }
        if n > 1024 * 1024 || !line.ends_with('\n') {
            return Err(Error::new("ARTIFACT_INVALID", "成果行超过预算或不完整"));
        }
        consume(serde_json::from_str(&line).map_err(Error::io)?)?;
        count += 1;
    }
    Ok(count)
}
fn file(relative: String, path: &Path, media_type: &str) -> Result<ArtifactFile> {
    Ok(ArtifactFile {
        path: relative,
        bytes: Some(fs::metadata(path).map_err(Error::io)?.len()),
        sha256: Some(worker::hash_file(path)?),
        media_type: media_type.into(),
    })
}
fn index(store: &SqliteStore, item: &Artifact, path: &Path) -> Result<()> {
    let mut batch = Vec::with_capacity(256);
    rows(path, |data| {
        let asset: Asset = serde_json::from_value(data["asset"].clone()).map_err(Error::io)?;
        let row = ArtifactRow {
            key: asset.key,
            ordinal: data["ordinal"]
                .as_u64()
                .ok_or_else(|| Error::new("ARTIFACT_INVALID", "成果缺少行序号"))?,
            scalar: data
                .get("scalar")
                .map(|v| serde_json::from_value(v.clone()).map_err(Error::io))
                .transpose()?,
            data,
        };
        batch.push(row);
        if batch.len() == 256 {
            store.append_artifact(&item.project_id, &item.id, &batch)?;
            batch.clear();
        }
        Ok(())
    })?;
    if !batch.is_empty() {
        store.append_artifact(&item.project_id, &item.id, &batch)?;
    }
    Ok(())
}
pub fn publish(
    store: &SqliteStore,
    job: &Job,
    plan: &WorkerPlan,
    primary: &Path,
    validated: Option<worker::ValidatedOutput>,
) -> Result<()> {
    if plan.version == 2 {
        return crate::ranking::publish(store, job, plan, primary, validated);
    }
    let mut checked = match validated {
        Some(checked) => checked,
        None => worker::validate_once(primary, plan, &mut |_, _, _| Ok(()))?,
    };
    let hash = checked.digest(primary, plan)?.to_owned();
    let run = store.job_run(&job.project_id, &job.id)?;
    let operator = studio_operators::registry()?.resolve(&plan.run)?;
    let descriptor = operator.descriptor();
    let attempt = store.job(&job.project_id, &job.id)?.attempt;
    let provenance=ArtifactProvenance {run:Some(plan.run.clone()),input_scope:job.input_scope.clone(),input_sha256:Some(plan.input_sha256.clone()),attempt:Some(attempt),input_artifacts:run.fields.iter().filter_map(|f|if let ScalarInput::Artifact{artifact_id}=f{Some(artifact_id.clone())}else{None}).collect(),fields_frozen:!run.fields.is_empty(),evidence:"validated_frozen_input_and_registered_operator_output; field_values_and_observation_basis_in_input_material".into()};
    let mut ids = Vec::new();
    for output in descriptor.outputs {
        let relative = if output.id == "data" {
            format!("artifacts/{}.jsonl", job.id)
        } else {
            format!("artifacts/{}.{}.jsonl", job.id, output.id)
        };
        let path = controlled_path(store, &job.project_id, &relative)?;
        let count = if output.id == "data" {
            job.total
        } else {
            let mut temporary = tempfile::NamedTempFile::new_in(
                path.parent()
                    .ok_or_else(|| Error::invalid("成果目录无效"))?,
            )
            .map_err(Error::io)?;
            let mut count = 0;
            rows(primary, |row| {
                if let Some(row) = operator.output_row(&output.id, &row)? {
                    serde_json::to_writer(&mut temporary, &row).map_err(Error::io)?;
                    temporary.write_all(b"\n").map_err(Error::io)?;
                    count += 1;
                }
                Ok(())
            })?;
            temporary.as_file().sync_all().map_err(Error::io)?;
            temporary.persist(&path).map_err(Error::io)?;
            count
        };
        let manifest_relative = relative.trim_end_matches(".jsonl").to_owned() + ".manifest.json";
        let manifest = controlled_path(store, &job.project_id, &manifest_relative)?;
        let data_file = file(relative, &path, "application/x-ndjson")?;
        atomic_json(
            &manifest,
            &serde_json::json!({"schema_version":1,"job_id":job.id,"output_id":output.id,"rows":count,"sha256":data_file.sha256,"input_sha256":plan.input_sha256,"input_scope":job.input_scope,"operator":plan.run.operator_id,"operator_version":plan.run.operator_version,"parameters_version":plan.run.parameters_version,"parameters":plan.run.parameters,"provenance":provenance,"primary_sha256":hash}),
        )?;
        let item = Artifact {
            id: if output.id == "data" {
                job.id.clone()
            } else {
                new_id()
            },
            project_id: job.project_id.clone(),
            job_id: job.id.clone(),
            output_id: output.id.clone(),
            name: format!("{} · {}", descriptor.name, output.name),
            kind: output.kind,
            schema_version: output.schema_version,
            state: ArtifactState::Publishing,
            count: Some(count),
            created_at: job.created_at.clone(),
            files: vec![
                data_file,
                file(manifest_relative, &manifest, "application/json")?,
            ],
            provenance: provenance.clone(),
            issue: None,
        };
        let item = store.begin_artifact(&job.project_id, &item)?;
        if item.state == ArtifactState::Publishing {
            index(store, &item, &path)?;
        }
        ids.push(item.id);
    }
    store.finish_artifacts(
        &job.project_id,
        &job.id,
        &ids,
        Some(&format!("artifacts/{}.jsonl", job.id)),
    )
}
/// File validation is explicit and lazy. A broken old artifact does not prevent project open.
pub fn verify(store: &SqliteStore, pid: &str, id: &str) -> Result<Artifact> {
    let mut item = store.artifact(pid, id)?;
    if item.state == ArtifactState::Released {
        return Ok(item);
    }
    let checked = (|| -> Result<()> {
        let mut files = Vec::new();
        for listed in &item.files {
            let path = controlled_path(store, pid, &listed.path)?;
            let actual = file(listed.path.clone(), &path, &listed.media_type)?;
            if listed
                .sha256
                .as_ref()
                .is_some_and(|h| actual.sha256.as_ref() != Some(h))
                || listed.bytes.is_some_and(|b| actual.bytes != Some(b))
            {
                return Err(Error::new("ARTIFACT_INVALID", "成果内容校验失败"));
            }
            files.push(actual);
        }
        if item.state == ArtifactState::Legacy {
            let data = files
                .first()
                .ok_or_else(|| Error::new("ARTIFACT_INVALID", "旧成果没有文件"))?;
            let path = controlled_path(store, pid, &data.path)?;
            let manifest_path = path.with_extension("manifest.json");
            if manifest_path.exists() {
                let manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(&manifest_path).map_err(Error::io)?)
                        .map_err(Error::io)?;
                if manifest["sha256"].as_str() != data.sha256.as_deref()
                    || manifest["rows"].as_u64() != item.count
                    || manifest["job_id"] != item.job_id
                {
                    return Err(Error::new("ARTIFACT_INVALID", "旧成果与原清单校验不一致"));
                }
                item.provenance.evidence="legacy_manifest_checksum_verified; missing_parameters_and_field_evidence_unknown".into();
                item.provenance.input_sha256 = manifest["input_sha256"].as_str().map(Into::into);
            } else {
                item.provenance.evidence =
                    "legacy_file_validated_now; original_checksum_and_field_evidence_unknown"
                        .into();
            }
            item.files = files;
            let item = store.begin_artifact(pid, &item)?;
            index(store, &item, &path)?;
            store.finish_artifacts(pid, &item.job_id, &[item.id], None)?;
        }
        Ok(())
    })();
    if let Err(error) = checked {
        store.artifact_unavailable(pid, id, &error.to_string())?;
        return Err(error);
    }
    store.artifact(pid, id)
}
