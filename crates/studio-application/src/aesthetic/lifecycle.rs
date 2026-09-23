//! Paid workflow rules, independent of HTTP, SQLite and the executor.
use studio_domain::{Error, Result, aesthetic::*, llm::*};

pub fn capabilities() -> AestheticCapabilities {
    AestheticCapabilities {
        version: 1,
        max_stage_candidates: AESTHETIC_MAX_CANDIDATES,
        batch_size: AESTHETIC_BATCH_SIZE as u32,
        max_image_bytes: 2 << 20,
        max_request_bytes: 48 << 20,
        default_request_bytes: 32 << 20,
    }
}

pub fn validate_capacity(total: u64) -> Result<()> {
    if total == 0 {
        return Err(Error::new("EVALUATION_EMPTY", "评审需要非空工作集"));
    }
    if total > AESTHETIC_MAX_CANDIDATES {
        return Err(Error::new(
            "EVALUATION_CAPACITY_EXCEEDED",
            "整个评审阶段最多支持 1000000 个候选；超出离线分析能力，尚未发送",
        ));
    }
    Ok(())
}

pub fn same_request(a: &AestheticCreate, b: &AestheticCreate) -> Result<()> {
    if serde_json::to_value(a).map_err(Error::io)? != serde_json::to_value(b).map_err(Error::io)? {
        return Err(Error::new(
            "IDEMPOTENCY_CONFLICT",
            "评审创建键已被不同请求使用",
        ));
    }
    Ok(())
}

pub fn terminal(state: &str) -> bool {
    matches!(
        state,
        "completed" | "completed_with_exclusions" | "cancelled"
    )
}

pub fn control_state(stage: &AestheticStage, action: &str) -> Result<&'static str> {
    match action {
        "start"
            if matches!(
                stage.state.as_str(),
                "ready" | "paused" | "needs_attention" | "failed"
            ) =>
        {
            validate_capacity(stage.total)?;
            validate_execution(&stage.config)?;
            if stage.attempts >= u64::from(stage.config.request.max_calls) {
                return Err(Error::invalid("阶段调用预算已用尽，请创建新阶段"));
            }
            Ok(if stage.frozen < stage.total {
                "preparing"
            } else {
                "running"
            })
        }
        "pause" if matches!(stage.state.as_str(), "running" | "preparing") => Ok("pausing"),
        "cancel" if matches!(stage.state.as_str(), "running" | "preparing" | "pausing") => {
            Ok("cancelling")
        }
        "cancel" if !terminal(&stage.state) => Ok("cancelled"),
        _ => Err(Error::new("REVISION_CONFLICT", "当前评审状态不允许此操作")),
    }
}

pub fn settled_state(stage: &AestheticStage, pending_batches: bool, has_error: bool) -> &str {
    match stage.state.as_str() {
        "completed" | "completed_with_exclusions" | "cancelled" | "paused" => &stage.state,
        "cancelling" => "cancelled",
        "pausing" => "paused",
        _ if has_error => "needs_attention",
        "running" if !pending_batches && stage.unresolved == 0 && stage.frozen == stage.total => {
            if stage.excluded > 0 {
                "completed_with_exclusions"
            } else {
                "completed"
            }
        }
        "running" => "needs_attention",
        _ => &stage.state,
    }
}

pub fn execution(input_version: String) -> AestheticExecution {
    AestheticExecution {
        template_version: "aesthetic_labels_v2".into(),
        business_schema_version: AESTHETIC_VERSION,
        encoder_version: "native_json_v1".into(),
        sampler_version: "rating_year_mix_v1".into(),
        input_version,
    }
}

pub fn validate_execution(config: &AestheticConfig) -> Result<()> {
    let compatible = match (&config.execution, config.version) {
        (None, 1) => true,
        (Some(v), 2) => {
            matches!(
                v.template_version.as_str(),
                "aesthetic_labels_v1" | "aesthetic_labels_v2"
            ) && v.business_schema_version == AESTHETIC_VERSION
                && v.encoder_version == "native_json_v1"
                && v.sampler_version == "rating_year_mix_v1"
        }
        _ => false,
    };
    if !compatible || config.image_policy != "stored_original_v1" {
        return Err(Error::new(
            "EVALUATION_CONFIG_UNSUPPORTED",
            "此阶段的冻结执行版本不受支持，请创建新阶段",
        ));
    }
    // Legacy snapshots with no provable user template must never silently use today's constant.
    if !config.model.messages.iter().any(|m| {
        m.role == LlmRole::User
            && m.content
                .iter()
                .any(|c| matches!(c, LlmContent::Text { text } if !text.trim().is_empty()))
    }) || config
        .model
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|c| !matches!(c, LlmContent::Text { .. }))
        || !config.model.tools.is_empty()
    {
        return Err(Error::new(
            "EVALUATION_CONFIG_UNSUPPORTED",
            "阶段缺少可验证的冻结文本模板，请创建新阶段",
        ));
    }
    Ok(())
}

/// Append labels/images to the existing frozen user message, retaining every frozen instruction.
pub fn request_messages(
    config: &AestheticConfig,
    images: Vec<LlmContent>,
) -> Result<Vec<LlmMessage>> {
    validate_execution(config)?;
    let mut messages = config.model.messages.clone();
    let user = messages
        .iter_mut()
        .rfind(|m| m.role == LlmRole::User)
        .ok_or_else(|| Error::new("EVALUATION_CONFIG_UNSUPPORTED", "缺少冻结 User 指令"))?;
    user.content.extend(images);
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_is_shared_and_rejects_the_first_unsupported_candidate() {
        validate_capacity(AESTHETIC_MAX_CANDIDATES).unwrap();
        assert_eq!(
            validate_capacity(AESTHETIC_MAX_CANDIDATES + 1)
                .unwrap_err()
                .code,
            "EVALUATION_CAPACITY_EXCEEDED"
        );
        assert_eq!(validate_capacity(0).unwrap_err().code, "EVALUATION_EMPTY");
        assert_eq!(
            capabilities().max_stage_candidates,
            crate::aesthetic_analysis::estimator::MAX_CANDIDATES
        );
    }
    #[test]
    fn frozen_template_survives_software_change_and_unproven_legacy_config_is_rejected() {
        let mut config:AestheticConfig=serde_json::from_value(serde_json::json!({
            "version":1,"request":{"idempotency_key":"x","name":"test","collection_id":"x","model_id":"x","system_prompt_id":"x","exposures":1,"max_calls":10,"concurrency":1},
            "model":{"schema_version":1,"invocation_id":"x","provider_id":"x","provider_revision":1,"provider_kind":"openai_compatible","base_url":"http://localhost","model_id":"x","model_revision":1,"remote_model_id":"fixture","protocol":"openai_chat","parameters":{},"messages":[{"role":"system","content":[{"type":"text","text":"frozen system"}]},{"role":"user","content":[{"type":"text","text":"old frozen user template"}]}],"tools":[],"warnings":[]},
            "sources":[],"image_policy":"stored_original_v1","grouping_policy":"test","observation_policy":"meaningful_indifference_v1","max_image_bytes":2097152,"max_request_bytes":12582912
        })).unwrap();
        let messages = request_messages(
            &config,
            vec![LlmContent::Text {
                text: "img01".into(),
            }],
        )
        .unwrap();
        assert!(
            matches!(&messages[1].content[0],LlmContent::Text{text} if text=="old frozen user template")
        );
        assert_eq!(messages[1].content.len(), 2);
        config.version = 2;
        config.execution = Some(execution("a".repeat(64)));
        config.execution.as_mut().unwrap().encoder_version = "future_encoder".into();
        assert_eq!(
            validate_execution(&config).unwrap_err().code,
            "EVALUATION_CONFIG_UNSUPPORTED"
        );
        config.version = 1;
        config.execution = None;
        config.model.messages.retain(|m| m.role != LlmRole::User);
        assert!(validate_execution(&config).is_err());
    }
}
