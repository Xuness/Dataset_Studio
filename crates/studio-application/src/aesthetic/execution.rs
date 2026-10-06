use studio_domain::{Error, Result, aesthetic::*, llm::*};

pub fn validate_execution_policy(p: &AestheticExecutionPolicy) -> Result<()> {
    studio_domain::validate_image_max_edge(p.image_max_edge)?;
    if !(1..=AESTHETIC_MAX_CONCURRENCY).contains(&p.concurrency)
        || p.memory_budget_mib
            .is_some_and(|v| !(256..=1_048_576).contains(&v))
        || p.upload_bytes_per_second
            .is_some_and(|v| v != 0 && !(100_000..=10_000_000_000).contains(&v))
        || p.failure_halt_threshold
            .is_some_and(|v| !(1..=AESTHETIC_MAX_CONCURRENCY).contains(&v))
        || !(100..=120_000).contains(&p.connect_timeout_ms)
        || !(100..=600_000).contains(&p.first_response_timeout_ms)
        || !(100..=600_000).contains(&p.idle_timeout_ms)
        || !(100..=3_600_000).contains(&p.request_timeout_ms)
        || !(p.request_timeout_ms..=7_200_000).contains(&p.batch_timeout_ms)
        || p.first_response_timeout_ms > p.request_timeout_ms
        || p.max_retries > 2
        || !matches!(p.exhausted.as_str(), "pause" | "defer")
    {
        return Err(Error::invalid(
            "执行设置无效：并发 1–1024、内存预算 256 MiB–1 TiB、上传速率 0（不限）或 0.1–10000 MB/s、连续失败停止阈值 1–1024、自动重试 0–2 次，首包和单次时限不得超过对应总预算",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod image_input_tests {
    use super::*;
    #[test]
    fn image_input_size_is_optional_bounded_and_preserves_legacy_policy() {
        let mut policy = AestheticExecutionPolicy::default();
        let legacy = serde_json::to_value(&policy).unwrap();
        assert!(legacy.get("image_max_edge").is_none());
        assert_eq!(
            serde_json::from_value::<AestheticExecutionPolicy>(legacy)
                .unwrap()
                .image_max_edge,
            None
        );
        for value in [None, Some(128), Some(1024), Some(1536), Some(8192)] {
            policy.image_max_edge = value;
            validate_execution_policy(&policy).unwrap();
        }
        for value in [0, 127, 8193, u32::MAX] {
            policy.image_max_edge = Some(value);
            assert_eq!(
                validate_execution_policy(&policy).unwrap_err().code,
                "INVALID_INPUT"
            );
        }
    }
}

pub fn recorded_options(p: &AestheticExecutionPolicy) -> LlmRecordedOptions {
    LlmRecordedOptions {
        stream: p.stream,
        connect_timeout_ms: p.connect_timeout_ms,
        first_response_timeout_ms: p.first_response_timeout_ms,
        idle_timeout_ms: p.idle_timeout_ms,
        request_timeout_ms: p.request_timeout_ms,
    }
}

pub fn validate_call_budget(request: &AestheticCreate, total: u64) -> Result<()> {
    if request.budget_mode.as_deref() == Some("complete")
        && u64::from(request.max_calls)
            < total
                .saturating_mul(u64::from(request.exposures))
                .div_ceil(16)
    {
        return Err(Error::new(
            "EVALUATION_BUDGET_INSUFFICIENT",
            "调用预算低于最低曝光下界；请增加预算或明确选择小预算试跑",
        ));
    }
    Ok(())
}

/// Only a stage policy can authorize another paid attempt after an unknown outcome.
pub fn retryable_failure(f: &LlmFailure, p: &AestheticExecutionPolicy) -> bool {
    let transient = matches!(
        f.code.as_str(),
        "LLM_NETWORK"
            | "LLM_TIMEOUT"
            | "LLM_STREAM_INTERRUPTED"
            | "LLM_UPSTREAM"
            | "EVALUATION_NO_COMPARABLE_EVIDENCE"
    ) || f.http_status.is_some_and(|v| v == 429 || v >= 500);
    transient
        && if f.outcome_unknown {
            p.retry_unknown
        } else {
            f.retryable
        }
}

pub fn stage_failure(f: &LlmFailure) -> bool {
    matches!(
        f.code.as_str(),
        "LLM_AUTHENTICATION"
            | "LLM_CONFIGURATION"
            | "LLM_CREDENTIAL_UNAVAILABLE"
            | "LLM_DISABLED"
            | "LLM_NOT_FOUND"
            | "LLM_INVALID_REQUEST"
            | "LLM_REDIRECT"
    ) || f.http_status == Some(402)
}

/// Connection/model revisions may change operational defaults; the frozen request must not.
pub fn same_execution_standard(
    old: &LlmInvocationSnapshot,
    new: &LlmInvocationSnapshot,
) -> Result<()> {
    if old.provider_id != new.provider_id
        || old.provider_kind != new.provider_kind
        || old.base_url != new.base_url
        || old.model_id != new.model_id
        || old.remote_model_id != new.remote_model_id
        || old.protocol != new.protocol
        || old.parameters != new.parameters
        || serde_json::to_value(&old.messages).map_err(Error::io)?
            != serde_json::to_value(&new.messages).map_err(Error::io)?
        || serde_json::to_value(&old.tools).map_err(Error::io)?
            != serde_json::to_value(&new.tools).map_err(Error::io)?
    {
        return Err(Error::new(
            "EVALUATION_STANDARD_CHANGED",
            "当前模型、端点或请求参数改变了冻结评审标准；请复制为新阶段",
        ));
    }
    Ok(())
}
