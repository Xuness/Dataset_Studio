use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use studio_application::{MediaInput, MediaSource, SourceAdapter};
use studio_domain::{ReadClass, ReadPriority, ReadRequest};

pub(super) enum Prepared {
    Messages(Vec<LlmMessage>),
    Rejected(Vec<(u64, String)>),
}

pub(super) fn prepare_images(
    state: &AppState,
    pid: &str,
    stage: &AestheticStage,
    batch: &mut AestheticBatch,
    cancelled: Arc<AtomicBool>,
) -> Result<Prepared> {
    let _read = state.resources.acquire(
        ReadRequest {
            class: ReadClass::Media,
            priority: ReadPriority::Background,
            bytes: 32 << 20,
        },
        &cancelled,
    )?;
    let router = studio_sources::SourceRouter;
    studio_application::aesthetic::validate_execution(&stage.config)?;
    let mut content = Vec::new();
    let mut media = std::collections::BTreeMap::new();
    let sources = batch
        .members
        .iter()
        .map(|m| m.candidate.key.source_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for sid in sources {
        let source = state.store.source(pid, &sid)?;
        let mut selected = Vec::new();
        let mut inputs = Vec::new();
        for member in batch
            .members
            .iter()
            .filter(|m| m.candidate.key.source_id == sid)
        {
            let check = router
                .verify_media_identity(&source, &member.candidate.key.asset_id)
                .and_then(|identity| {
                    if identity.content_version != member.candidate.content_version {
                        Err(Error::new("SOURCE_CHANGED", "图片内容版本已变化"))
                    } else if identity.bytes > stage.config.max_image_bytes {
                        Err(Error::invalid("图片超过本阶段 2 MiB 原图上限"))
                    } else {
                        Ok(())
                    }
                });
            if let Err(error) = check {
                media.insert(member.label.clone(), Err(error));
                continue;
            }
            selected.push(member);
            inputs.push(MediaInput {
                asset_id: member.candidate.key.asset_id.clone(),
                cancelled: cancelled.clone(),
                byte_limit: stage.config.max_image_bytes,
            });
        }
        if !inputs.is_empty() {
            let read = router.read_many(&source, &inputs)?;
            if read.items.len() != selected.len() {
                return Err(Error::invalid("来源返回的图片数量与请求不一致"));
            }
            for (member, item) in selected.into_iter().zip(read.items) {
                media.insert(member.label.clone(), item);
            }
        }
    }
    let mut rejected = Vec::new();
    for member in &mut batch.members {
        studio_application::read_cancelled(&cancelled)?;
        let prepared = (|| -> Result<LlmContent> {
            let image = media
                .remove(&member.label)
                .ok_or_else(|| Error::invalid("来源未返回图片"))??;
            if !matches!(
                image.content_type.as_str(),
                "image/webp" | "image/jpeg" | "image/png"
            ) {
                return Err(Error::invalid("评审支持 WebP、JPEG、PNG"));
            }
            let signature = match image.content_type.as_str() {
                "image/png" => image.bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "image/jpeg" => image.bytes.starts_with(b"\xff\xd8\xff"),
                "image/webp" => {
                    image.bytes.starts_with(b"RIFF") && image.bytes.get(8..12) == Some(b"WEBP")
                }
                _ => false,
            };
            if !signature {
                return Err(Error::invalid("图片文件签名与格式不一致"));
            }
            let digest = hex::encode(Sha256::digest(&image.bytes));
            if member
                .candidate
                .content_version
                .strip_prefix("sha256:")
                .is_some_and(|v| v != digest)
                || member.image_sha256.as_ref().is_some_and(|v| v != &digest)
            {
                return Err(Error::new(
                    "SOURCE_CHANGED",
                    "图片 SHA-256 与冻结身份不一致",
                ));
            }
            member.image_sha256 = Some(digest);
            Ok(LlmContent::Image {
                url: format!(
                    "data:{};base64,{}",
                    image.content_type,
                    STANDARD.encode(&image.bytes)
                ),
                detail: None,
            })
        })();
        match prepared {
            Ok(image) => {
                content.push(LlmContent::Text {
                    text: member.label.clone(),
                });
                content.push(image);
            }
            Err(e) if e.code == "CANCELLED" => return Err(e),
            Err(e) => rejected.push((member.candidate.ordinal, e.to_string())),
        }
    }
    if !rejected.is_empty() {
        return Ok(Prepared::Rejected(rejected));
    }
    studio_application::aesthetic::request_messages(&stage.config, content).map(Prepared::Messages)
}

pub(super) fn freeze_page(
    state: &AppState,
    pid: &str,
    stage: &AestheticStage,
    db: &EvaluationDb,
    cancel: Arc<AtomicBool>,
) -> Result<bool> {
    let last = db.last_candidate(&stage.id)?;
    let keys = state.store.collection_keys(
        pid,
        &stage.config.request.collection_id,
        last.as_ref().map(|v| &v.key),
        64,
    )?;
    if keys.is_empty() {
        db.finish_freeze(&stage.id)?;
        return Ok(false);
    }
    let _permit = state.resources.acquire(
        ReadRequest {
            class: ReadClass::NativeQuery,
            priority: ReadPriority::Background,
            bytes: studio_domain::METADATA_MEMORY_BYTES,
        },
        &cancel,
    )?;
    let mut rows = Vec::new();
    let mut index = 0;
    while index < keys.len() {
        let end = keys[index..]
            .iter()
            .position(|k| k.source_id != keys[index].source_id)
            .map(|n| index + n)
            .unwrap_or(keys.len());
        let chunk = &keys[index..end];
        let source = state.store.source(pid, &keys[index].source_id)?;
        let expected = stage
            .config
            .sources
            .iter()
            .find(|s| s.source_id == source.id)
            .ok_or_else(|| Error::invalid("冻结来源缺少版本"))?;
        let frozen = studio_sources::SourceRouter.freeze(&source, chunk)?;
        if frozen
            .iter()
            .any(|f| f.source_revision != expected.catalog_revision)
        {
            return Err(Error::new("SOURCE_CHANGED", "来源版本已变化，请创建新阶段"));
        }
        let groups = state
            .metadata
            .aesthetic_groups(&source, chunk, expected, cancel.clone())?;
        for (item, (rating, year, basis)) in frozen.into_iter().zip(groups) {
            let content_version =
                studio_sources::SourceRouter.content_version(&source, &item.asset.key.asset_id)?;
            rows.push(AestheticCandidate {
                ordinal: stage.frozen + rows.len() as u64,
                key: item.asset.key,
                rating,
                year,
                basis,
                content_version,
                bytes: item.asset.bytes,
                exposures: 0,
                protected: false,
                disposition: Default::default(),
                disposition_reason: None,
            });
        }
        index = end;
    }
    drop(_permit);
    // Creation validates bounded physical reads without submitting a model request.
    for chunk in rows.chunks_mut(16) {
        let mut probe = AestheticBatch {
            sequence: 0,
            parent_sequence: None,
            replacement_sequences: vec![],
            stage_id: stage.id.clone(),
            rating: String::new(),
            state: "preparing".into(),
            members: chunk
                .iter()
                .enumerate()
                .map(|(i, c)| AestheticMember {
                    label: format!("img{:02}", i + 1),
                    candidate: c.clone(),
                    image_sha256: None,
                })
                .collect(),
            attempt_id: None,
            error: None,
            observation: None,
        };
        if let Prepared::Rejected(rejected) =
            prepare_images(state, pid, stage, &mut probe, cancel.clone())?
        {
            for row in chunk {
                if let Some((_, reason)) =
                    rejected.iter().find(|(ordinal, _)| *ordinal == row.ordinal)
                {
                    row.disposition = AestheticDisposition::NeedsReview;
                    row.disposition_reason = Some(reason.clone());
                }
            }
        }
    }
    db.append_candidates(&stage.id, rows)?;
    Ok(true)
}
