use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use studio_domain::{Error, Result, llm::LlmInvocationSnapshot};
pub fn apply(body: &mut Value, snapshot: &LlmInvocationSnapshot) -> Result<()> {
    let parameters = &snapshot.parameters;
    for (key, value) in parameters {
        if let Some(key) = key.strip_prefix("openrouter.")
            && !matches!(key, "cache_strategy" | "cache_affinity")
        {
            body[key] = value.clone();
        }
    }
    if let Some(effort) = parameters.get("reasoning_effort") {
        body["reasoning"] = json!({"effort":effort});
    }
    if let Some(budget) = parameters.get("reasoning_budget") {
        if budget.as_i64().is_some_and(|v| v < 0) {
            return Err(Error::invalid("OpenRouter 推理预算不能为负数"));
        }
        body["reasoning"] = json!({"max_tokens":budget});
    }
    strict_flex(body)?;
    if parameters.get("cache_strategy").is_some() {
        return Err(Error::invalid(
            "请使用 openrouter.cache_strategy 设置缓存策略",
        ));
    }
    if parameters
        .get("openrouter.cache_affinity")
        .and_then(Value::as_bool)
        == Some(true)
        && body.get("session_id").is_none()
        && let Some(system) = body["messages"].as_array().and_then(|messages| {
            messages
                .iter()
                .find(|m| matches!(m["role"].as_str(), Some("system" | "developer")))
        })
    {
        // No invocation IDs, image bytes, or changing user text in this fallback key.
        // Aesthetic stages provide their own frozen session_id for stage isolation.
        let prefix = serde_json::to_vec(&json!([
            "studio-cache-v1",
            snapshot.provider_id,
            snapshot.remote_model_id,
            system,
            snapshot.tools
        ]))
        .map_err(Error::io)?;
        body["session_id"] = json!(format!("studio-{:x}", Sha256::digest(prefix)));
    }
    if parameters
        .get("openrouter.cache_strategy")
        .and_then(Value::as_str)
        == Some("system")
    {
        cache_system(body, &snapshot.remote_model_id)?;
    }
    Ok(())
}

fn strict_flex(body: &mut Value) -> Result<()> {
    if body["service_tier"] != "flex" {
        return Ok(());
    }
    // OpenRouter may use standard rates if a model has no flex endpoints.
    // Exact endpoint restrictions prevent that fallback even in that case.
    let provider = body
        .as_object_mut()
        .ok_or_else(|| Error::invalid("请求必须为对象"))?
        .entry("provider")
        .or_insert_with(|| json!({}));
    let object = provider
        .as_object_mut()
        .ok_or_else(|| Error::invalid("上游路由必须为对象"))?;
    let selected = object
        .get("only")
        .or_else(|| object.get("order"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::invalid("严格 Flex 需要在上游路由 only 中指定以 /flex 结尾的端点"))?;
    if selected.is_empty()
        || selected
            .iter()
            .any(|v| v.as_str().is_none_or(|s| !s.ends_with("/flex")))
    {
        return Err(Error::invalid(
            "严格 Flex 只接受明确的 /flex 端点，请移除普通或 Priority 端点",
        ));
    }
    object.insert("only".into(), json!(selected));
    Ok(())
}

fn cache_system(body: &mut Value, model: &str) -> Result<()> {
    if !(model.starts_with("google/gemini-") || model.starts_with("anthropic/")) {
        return Err(Error::invalid(
            "显式 System 缓存当前支持 Gemini 和 Claude；其他模型请使用 implicit 自动缓存",
        ));
    }
    let messages = body["messages"]
        .as_array_mut()
        .ok_or_else(|| Error::invalid("缺少消息"))?;
    let system = messages
        .first_mut()
        .filter(|m| matches!(m["role"].as_str(), Some("system" | "developer")))
        .ok_or_else(|| {
            Error::invalid("显式 System 缓存要求第一条消息为固定 System 或 Developer 指令")
        })?;
    if let Some(text) = system["content"].as_str() {
        system["content"] = json!([{"type":"text", "text":text}]);
    }
    let parts = system["content"]
        .as_array_mut()
        .ok_or_else(|| Error::invalid("System 缓存需要文本内容"))?;
    if parts.is_empty() || parts.iter().any(|p| p["type"] != "text") {
        return Err(Error::invalid("System 缓存仅支持完整的固定文本指令"));
    }
    parts.last_mut().expect("nonempty")["cache_control"] = json!({"type":"ephemeral"});
    Ok(())
}
