use serde_json::{Value, json};
use studio_application::llm::LlmCallResult;
use studio_domain::{Error, Result, llm::*};
pub mod catalog;
mod chat;
mod gemini;
mod responses;
pub mod streaming;
#[cfg(test)]
mod tests;

pub fn encode(plan: &LlmInvocationPlan, stream: bool) -> Result<Value> {
    match plan.snapshot.protocol {
        LlmProtocol::OpenaiChat => chat::encode(plan, stream),
        LlmProtocol::OpenaiResponses => responses::encode(plan, stream),
        LlmProtocol::Gemini => gemini::encode(plan),
    }
}
pub fn suffix(plan: &LlmInvocationPlan, stream: bool) -> String {
    match plan.snapshot.protocol {
        LlmProtocol::OpenaiChat => "chat/completions".into(),
        LlmProtocol::OpenaiResponses => "responses".into(),
        LlmProtocol::Gemini => {
            // Remote IDs are validated separately; encoded as a single URL path segment.
            let id = plan.snapshot.remote_model_id.trim_start_matches("models/");
            let encoded: String = id
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            format!(
                "models/{encoded}:{}",
                if stream {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                }
            )
        }
    }
}
pub fn decode(
    plan: &LlmInvocationPlan,
    value: &Value,
    request_id: Option<String>,
) -> LlmCallResult<LlmResponse> {
    if value.get("error").is_some_and(|v| !v.is_null()) {
        return Err(invalid("供应商返回错误对象"));
    }
    let mut response = match plan.snapshot.protocol {
        LlmProtocol::OpenaiChat => chat::decode(plan, value)?,
        LlmProtocol::OpenaiResponses => responses::decode(plan, value)?,
        LlmProtocol::Gemini => gemini::decode(plan, value)?,
    };
    response.provider_request_id = request_id;
    Ok(response)
}
pub(super) fn invalid(message: &str) -> LlmFailure {
    LlmFailure::new("LLM_INVALID_RESPONSE", message)
}
pub(super) fn string(v: &Value, name: &str) -> Option<String> {
    v.get(name).and_then(Value::as_str).map(str::to_owned)
}
pub(super) fn usage(v: &Value, gemini: bool) -> LlmUsage {
    let get = |key: &str| v.pointer(key).and_then(Value::as_u64);
    if gemini {
        LlmUsage {
            input_tokens: get("/promptTokenCount"),
            output_tokens: get("/candidatesTokenCount"),
            total_tokens: get("/totalTokenCount"),
            cached_input_tokens: get("/cachedContentTokenCount"),
            reasoning_tokens: get("/thoughtsTokenCount"),
            ..Default::default()
        }
    } else {
        LlmUsage {
            input_tokens: get("/input_tokens").or(get("/prompt_tokens")),
            output_tokens: get("/output_tokens").or(get("/completion_tokens")),
            total_tokens: get("/total_tokens"),
            cached_input_tokens: get("/input_tokens_details/cached_tokens")
                .or(get("/prompt_tokens_details/cached_tokens")),
            reasoning_tokens: get("/output_tokens_details/reasoning_tokens")
                .or(get("/completion_tokens_details/reasoning_tokens")),
            cache_write_tokens: get("/input_tokens_details/cache_write_tokens")
                .or(get("/prompt_tokens_details/cache_write_tokens")),
            cost_usd: v
                .get("cost")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0),
            ..Default::default()
        }
    }
}
pub(super) fn base_response(
    plan: &LlmInvocationPlan,
    v: &Value,
    outputs: Vec<LlmOutput>,
    gemini: bool,
) -> LlmResponse {
    let mut usage = usage(&v[if gemini { "usageMetadata" } else { "usage" }], gemini);
    usage.upstream_provider = string(v, "provider");
    usage.service_tier = string(v, "service_tier");
    LlmResponse {
        snapshot: plan.snapshot.clone(),
        provider_request_id: None,
        response_id: string(v, "id").or_else(|| string(v, "responseId")),
        model: string(v, "model").or_else(|| string(v, "modelVersion")),
        outputs,
        usage,
    }
}
pub(super) fn function_arguments(v: &Value) -> LlmCallResult<Value> {
    if let Some(s) = v.as_str() {
        serde_json::from_str(s).map_err(|_| invalid("工具调用参数不是有效 JSON"))
    } else if v.is_object() {
        Ok(v.clone())
    } else {
        Err(invalid("工具调用参数无效"))
    }
}
pub(super) fn text_content(text: &str) -> Value {
    json!({"type":"text", "text":text})
}
