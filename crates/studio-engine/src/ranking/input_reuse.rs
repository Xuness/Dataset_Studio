//! Reuse a verified full-source snapshot by copying it into this job's staging
//! area. Source artifacts retain their bytes and no new dependency is required.
use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

#[allow(clippy::too_many_arguments)]
pub(super) fn copy(
    store: &SqliteStore,
    job: &Job,
    frozen: &JobRun,
    requested: &RankingParameters,
    staging: &Path,
    target: &Path,
    cancelled: &Arc<AtomicBool>,
) -> Result<bool> {
    if frozen.source_versions.len() != 1 {
        return Ok(false);
    }
    for artifact in store.ranking_snapshot_candidates(
        &job.project_id,
        &job.id,
        &frozen.source_versions,
        job.total,
    )? {
        read_cancelled(cancelled)?;
        let prior = store.job_run(&job.project_id, &artifact.job_id)?;
        if prior.run.operator_id != frozen.run.operator_id
            || prior.run.operator_version != frozen.run.operator_version
            || prior.run.parameters_version != frozen.run.parameters_version
            || !parameters(&prior.run).is_ok_and(|p| p.same_projection(requested))
        {
            continue;
        }
        let Some(file) = artifact
            .files
            .iter()
            .find(|f| f.path.ends_with(".ranking-input.sqlite"))
        else {
            continue;
        };
        let Some(expected) = file.sha256.as_deref().filter(|s| s.len() == 64) else {
            continue;
        };
        let path = crate::artifacts::controlled_path(store, &job.project_id, &file.path)?;
        let Ok(mut source) = File::open(&path) else {
            continue;
        };
        let bytes = source.metadata().map_err(Error::io)?.len();
        if bytes > STAGE_LIMIT || file.bytes != Some(bytes) {
            continue;
        }
        let compatible = (|| -> Result<bool> {
            let input = RankingInputTable::open(&path)?;
            let captured: JobRun = input.meta("job_run")?;
            Ok(input.meta::<bool>("complete")?
                && input.meta::<String>("membership")? == "retained_source_snapshot"
                && captured.source_versions == frozen.source_versions
                && captured.run == prior.run
                && input.count()? == job.total)
        })()
        .unwrap_or(false);
        if !compatible {
            continue;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(Error::io)?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        let mut copied = 0u64;
        let mut last = Instant::now() - Duration::from_secs(1);
        store.job_stage(
            &job.project_id,
            &job.id,
            &JobStage {
                name: "snapshot_reuse".into(),
                total: bytes,
                ..Default::default()
            },
        )?;
        loop {
            read_cancelled(cancelled)?;
            let count = source.read(&mut buffer).map_err(Error::io)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count]).map_err(Error::io)?;
            digest.update(&buffer[..count]);
            copied += count as u64;
            if copied > bytes {
                return Err(Error::new("INPUT_CHANGED", "复用的快照长度已改变"));
            }
            if last.elapsed() >= Duration::from_millis(700) {
                store.job_stage(
                    &job.project_id,
                    &job.id,
                    &JobStage {
                        name: "snapshot_reuse".into(),
                        completed: copied,
                        total: bytes,
                        ..Default::default()
                    },
                )?;
                last = Instant::now();
            }
        }
        drop(output);
        if copied != bytes || hex::encode(digest.finalize()) != expected {
            remove_partial(staging, "input.sqlite")?;
            tracing::warn!(artifact_id=%artifact.id,"ranking snapshot reuse skipped: checksum mismatch");
            continue;
        }
        tracing::info!(job_id=%job.id,artifact_id=%artifact.id,bytes,"ranking snapshot reused");
        return Ok(true);
    }
    Ok(false)
}
