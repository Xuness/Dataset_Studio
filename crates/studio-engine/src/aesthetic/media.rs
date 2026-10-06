use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use studio_application::{MediaInput, MediaSource, SourceAdapter};
use studio_domain::ReadClass;

pub(super) enum Prepared {
    Messages(Vec<LlmMessage>, Vec<AestheticImageInput>),
    Oversized,
    Rejected(Vec<(u64, String)>),
}

pub(super) fn prepare_images(
    state: &AppState,
    pid: &str,
    stage: &AestheticStage,
    batch: &mut AestheticBatch,
    cancelled: Arc<AtomicBool>,
    prepare_request: bool,
) -> Result<Prepared> {
    let router = state
        .sources
        .background(ReadClass::Media, 32 << 20, cancelled.clone())?;
    studio_application::aesthetic::validate_execution(&stage.config)?;
    let mut content = Vec::new();
    let mut image_inputs = Vec::new();
    let mut image_url_bytes = 0_u64;
    let max_edge = stage_policy(stage).image_max_edge;
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
                deadline: None,
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
    drop(router);
    let mut rejected = Vec::new();
    for member in &mut batch.members {
        studio_application::read_cancelled(&cancelled)?;
        // Resource backpressure is an execution failure, not an invalid-image decision.
        // Acquire outside the per-image rejection path so a full decode queue cannot
        // permanently mark an otherwise valid candidate as needing image review.
        let _decode = (prepare_request && max_edge.is_some())
            .then(|| {
                state.resources.acquire(
                    studio_domain::ReadRequest {
                        class: ReadClass::Decode,
                        priority: studio_domain::ReadPriority::Background,
                        bytes: studio_resources::IMAGE_INPUT_WORKSPACE_BYTES,
                    },
                    &cancelled,
                )
            })
            .transpose()?;
        let prepared = (|| -> Result<Option<(LlmContent, studio_domain::ImageInputInfo)>> {
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
            member.image_sha256 = Some(digest.clone());
            if !prepare_request {
                // Candidate freezing verifies the original only, without encoding the whole lake.
                return Ok(None);
            }
            studio_application::read_cancelled(&cancelled)?;
            let prepared = studio_resources::prepare_image_input(
                image,
                &digest,
                max_edge,
                Some(&state.previews.cache),
            )?;
            studio_application::read_cancelled(&cancelled)?;
            Ok(Some((
                LlmContent::Image {
                    url: format!(
                        "data:{};base64,{}",
                        prepared.media.content_type,
                        STANDARD.encode(&prepared.media.bytes)
                    ),
                    detail: None,
                },
                prepared.info,
            )))
        })();
        match prepared {
            Ok(Some((image, info))) => {
                if let LlmContent::Image { url, .. } = &image {
                    image_url_bytes = image_url_bytes.saturating_add(url.len() as u64);
                }
                if image_url_bytes > stage.config.max_request_bytes {
                    return Ok(Prepared::Oversized);
                }
                image_inputs.push(AestheticImageInput {
                    label: member.label.clone(),
                    image: info,
                });
                content.push(LlmContent::Text {
                    text: member.label.clone(),
                });
                content.push(image);
            }
            Ok(None) => {}
            Err(e) if e.code == "CANCELLED" => return Err(e),
            Err(e) => rejected.push((member.candidate.ordinal, e.to_string())),
        }
    }
    if !rejected.is_empty() {
        return Ok(Prepared::Rejected(rejected));
    }
    let messages = if prepare_request {
        studio_application::aesthetic::request_messages(&stage.config, content)?
    } else {
        Vec::new()
    };
    Ok(Prepared::Messages(messages, image_inputs))
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
    let reader = state.sources.background(
        ReadClass::NativeQuery,
        studio_domain::METADATA_MEMORY_BYTES,
        cancel.clone(),
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
        let frozen = reader.freeze(&source, chunk)?;
        if frozen
            .iter()
            .any(|f| f.source_revision != expected.catalog_revision)
        {
            return Err(Error::new("SOURCE_CHANGED", "来源版本已变化，请创建新阶段"));
        }
        let groups = reader.origin_groups(&source, chunk, expected)?;
        for (item, (rating, year, basis)) in frozen.into_iter().zip(groups) {
            let content_version = reader.content_version(&source, &item.asset.key.asset_id)?;
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
                blocked: false,
                blocking_batch: None,
            });
        }
        index = end;
    }
    drop(reader);
    // Creation validates bounded physical reads without submitting a model request.
    for chunk in rows.chunks_mut(16) {
        let mut probe = AestheticBatch {
            sequence: 0,
            stage_sequence: 0,
            attempt_count: 0,
            retry_at: None,
            recovery_deadline: None,
            resolution_reason: None,
            last_failure: None,
            has_raw_receipt: false,
            transfer: None,
            parent_sequence: None,
            replacement_sequences: vec![],
            sampling: None,
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
            prepare_images(state, pid, stage, &mut probe, cancel.clone(), false)?
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
