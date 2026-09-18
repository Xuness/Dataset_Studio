use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use studio_application::{MediaInput, MediaSource, SourceAdapter};
use studio_domain::{ReadClass, ReadPriority, ReadRequest};

pub(super) fn prepare_images(
    state: &AppState,
    pid: &str,
    stage: &AestheticStage,
    batch: &mut AestheticBatch,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<LlmMessage>> {
    let _read = state.resources.acquire(
        ReadRequest {
            class: ReadClass::Media,
            priority: ReadPriority::Background,
            bytes: 32 << 20,
        },
        &cancelled,
    )?;
    let router = studio_sources::SourceRouter;
    let mut content = vec![LlmContent::Text {
        text: studio_application::aesthetic::OUTPUT_INSTRUCTIONS.into(),
    }];
    let mut bytes = 0;
    // Physical source batches remain bounded. The model sees the frozen randomized order.
    let mut media = std::collections::BTreeMap::new();
    let sources = batch
        .members
        .iter()
        .map(|m| m.candidate.key.source_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for source_id in sources {
        let source = state.store.source(pid, &source_id)?;
        let selected = batch
            .members
            .iter()
            .filter(|m| m.candidate.key.source_id == source_id)
            .collect::<Vec<_>>();
        let mut inputs = Vec::new();
        for member in &selected {
            let identity = router.verify_media_identity(&source, &member.candidate.key.asset_id)?;
            if identity.content_version != member.candidate.content_version {
                return Err(Error::new("SOURCE_CHANGED", "图片内容版本已变化"));
            }
            if identity.bytes > stage.config.max_image_bytes {
                return Err(Error::invalid(
                    "图片超过本阶段 2 MiB 原图上限；请先选择合适的评审图片规格",
                ));
            }
            inputs.push(MediaInput {
                asset_id: member.candidate.key.asset_id.clone(),
                cancelled: cancelled.clone(),
                byte_limit: stage.config.max_image_bytes,
            });
        }
        let read = router.read_many(&source, &inputs)?;
        for (member, item) in selected.into_iter().zip(read.items) {
            media.insert(member.label.clone(), item?);
        }
    }
    for member in &mut batch.members {
        studio_application::read_cancelled(&cancelled)?;
        let image = media
            .remove(&member.label)
            .ok_or_else(|| Error::invalid("批次缺少图片"))?;
        if !matches!(
            image.content_type.as_str(),
            "image/webp" | "image/jpeg" | "image/png"
        ) {
            return Err(Error::invalid(
                "第一版评审图片支持 WebP、JPEG、PNG 原始字节",
            ));
        }
        let digest = hex::encode(Sha256::digest(&image.bytes));
        if member
            .candidate
            .content_version
            .strip_prefix("sha256:")
            .is_some_and(|v| v != digest)
        {
            return Err(Error::new("SOURCE_CHANGED", "图片 SHA-256 校验失败"));
        }
        if member.image_sha256.as_ref().is_some_and(|v| v != &digest) {
            return Err(Error::new("SOURCE_CHANGED", "重试图片与首次发送内容不一致"));
        }
        member.image_sha256 = Some(digest);
        let url = format!(
            "data:{};base64,{}",
            image.content_type,
            STANDARD.encode(&image.bytes)
        );
        bytes += url.len();
        if bytes as u64 > stage.config.max_request_bytes {
            return Err(Error::invalid("16 图批次超过 12 MiB 请求预算，尚未发送"));
        }
        content.push(LlmContent::Text {
            text: member.label.clone(),
        });
        content.push(LlmContent::Image { url, detail: None });
    }
    let mut messages = stage
        .config
        .model
        .messages
        .iter()
        .filter(|m| matches!(m.role, LlmRole::System | LlmRole::Developer))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(LlmMessage {
        role: LlmRole::User,
        content,
    });
    Ok(messages)
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
            });
        }
        index = end;
    }
    db.append_candidates(&stage.id, rows)?;
    Ok(true)
}
