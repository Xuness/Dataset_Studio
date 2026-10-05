use super::*;
pub(super) fn encode(plan: &LlmInvocationPlan, stream: bool) -> Result<Value> {
    let s = &plan.snapshot;
    let mut messages = Vec::new();
    for message in &s.messages {
        if message.role == LlmRole::Tool {
            for block in &message.content {
                if let LlmContent::ToolResult { id, value, .. } = block {
                    messages.push(json!({"role":"tool","tool_call_id":id,"content":serde_json::to_string(value).map_err(Error::io)?}));
                }
            }
            continue;
        }
        let mut parts = Vec::new();
        let mut calls = Vec::new();
        for block in &message.content {
            match block {
                LlmContent::Text { text } => parts.push(text_content(text)),
                LlmContent::Image { url, detail } => {
                    let mut image = json!({"url":url});
                    if let Some(detail) = detail {
                        image["detail"] = json!(detail);
                    }
                    parts.push(json!({"type":"image_url", "image_url":image}));
                }
                LlmContent::ToolCall {
                    id,
                    name,
                    arguments,
                    signature,
                } => {
                    if signature.is_some() {
                        return Err(Error::invalid(
                            "当前 Chat Completions 适配器不支持 Gemini thought signature",
                        ));
                    }
                    calls.push(json!({"id":id,"type":"function","function":{"name":name,"arguments":serde_json::to_string(arguments).map_err(Error::io)?}}));
                }
                _ => return Err(Error::invalid("Chat Completions 不支持该输入内容块")),
            }
        }
        let content = if parts.is_empty() {
            Value::Null
        } else if parts.len() == 1 && parts[0]["type"] == "text" {
            parts[0]["text"].clone()
        } else {
            json!(parts)
        };
        let mut item = json!({"role":message.role,"content":content});
        if !calls.is_empty() {
            item["tool_calls"] = json!(calls);
        }
        messages.push(item);
    }
    let mut body = json!({"model":s.remote_model_id,"messages":messages,"stream":stream});
    for (key, value) in &s.parameters {
        match key.as_str() {
            "max_output_tokens" => {
                let default_field = if s.provider_kind == LlmProviderKind::Openai {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                };
                body[s
                    .parameters
                    .get("token_limit_field")
                    .and_then(Value::as_str)
                    .unwrap_or(default_field)] = value.clone();
            }
            "stream_usage" => {
                if stream {
                    body["stream_options"] = json!({"include_usage":value});
                }
            }
            "token_limit_field" => {}
            key if key.starts_with("openrouter.")
                || (s.provider_kind == LlmProviderKind::Openrouter
                    && matches!(key, "reasoning_effort" | "reasoning_budget")) => {}
            _ => {
                body[key] = value.clone();
            }
        }
    }
    if !s.tools.is_empty() {
        body["tools"] = Value::Array(s.tools.iter().map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters,"strict":t.strict}})).collect());
    }
    if s.provider_kind == LlmProviderKind::Openrouter {
        crate::providers::openrouter::apply(&mut body, s)?;
    }
    Ok(body)
}
pub(super) fn decode(plan: &LlmInvocationPlan, v: &Value) -> LlmCallResult<LlmResponse> {
    let choices = v["choices"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| invalid("Chat Completions 缺少 choices"))?;
    let mut outputs = Vec::new();
    for choice in choices {
        let message = &choice["message"];
        if !message.is_object() {
            return Err(invalid("Chat Completions 缺少 message"));
        }
        if message.get("audio").is_some_and(|v| !v.is_null())
            || message
                .get("images")
                .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        {
            return Err(invalid("当前调用契约不支持音频或图片输出"));
        }
        let mut content = Vec::new();
        if let Some(text) = message["content"].as_str() {
            content.push(LlmContent::Text { text: text.into() });
        }
        if let Some(parts) = message["content"].as_array() {
            for part in parts {
                match part["type"].as_str() {
                    Some("text") => content.push(LlmContent::Text {
                        text: string(part, "text").ok_or_else(|| invalid("文本内容无效"))?,
                    }),
                    Some("refusal") => content.push(LlmContent::Refusal {
                        text: string(part, "refusal").ok_or_else(|| invalid("拒绝内容无效"))?,
                    }),
                    _ => return Err(invalid("供应商返回尚不支持的内容块")),
                }
            }
        } else if !message["content"].is_null() && !message["content"].is_string() {
            return Err(invalid("Chat Completions content 格式无效"));
        }
        if let Some(text) = message["reasoning"]
            .as_str()
            .or_else(|| message["reasoning_content"].as_str())
        {
            content.push(LlmContent::Reasoning { text: text.into() });
        }
        if let Some(text) = message["refusal"].as_str() {
            content.push(LlmContent::Refusal { text: text.into() });
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                content.push(LlmContent::ToolCall {
                    id: string(call, "id").ok_or_else(|| invalid("工具调用缺少 id"))?,
                    name: string(&call["function"], "name")
                        .ok_or_else(|| invalid("工具调用缺少名称"))?,
                    arguments: function_arguments(&call["function"]["arguments"])?,
                    signature: None,
                });
            }
        }
        outputs.push(LlmOutput {
            index: choice["index"].as_u64().unwrap_or(0) as u32,
            content,
            finish_reason: string(choice, "finish_reason"),
        });
    }
    Ok(base_response(plan, v, outputs, false))
}
