use super::*;
pub(super) fn encode(plan: &LlmInvocationPlan, stream: bool) -> Result<Value> {
    let s = &plan.snapshot;
    let mut input = Vec::new();
    for message in &s.messages {
        let mut parts = Vec::new();
        // Preserve the sequence of messages and tool items.
        for block in &message.content {
            match block {
                LlmContent::Text { text } => parts.push(json!({"type":"input_text","text":text})),
                LlmContent::Image { url, detail } => {
                    let mut value = json!({"type":"input_image","image_url":url});
                    if let Some(detail) = detail { value["detail"] = json!(detail); }
                    parts.push(value);
                },
                LlmContent::ToolCall { id, name, arguments, signature } => {
                    if signature.is_some() { return Err(Error::invalid("Responses 不支持 Gemini thought signature")); }
                    if !parts.is_empty() { input.push(json!({"role":message.role,"content":std::mem::take(&mut parts)})); }
                    input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":serde_json::to_string(arguments).map_err(Error::io)?}));
                },
                LlmContent::ToolResult { id, value, .. } => input.push(json!({"type":"function_call_output","call_id":id,"output":serde_json::to_string(value).map_err(Error::io)?})),
                _ => return Err(Error::invalid("Responses 不支持该输入内容块")),
            }
        }
        if !parts.is_empty() {
            input.push(json!({"role":message.role,"content":parts}));
        }
    }
    let mut body = json!({"model":s.remote_model_id,"input":input,"stream":stream});
    for (key, value) in &s.parameters {
        match key.as_str() {
            "reasoning_effort" => body["reasoning"] = json!({"effort":value}),
            "verbosity" => {
                if body.get("text").is_none() {
                    body["text"] = json!({});
                }
                body["text"]["verbosity"] = value.clone();
            }
            "response_format" => {
                let format = if value["type"] == "json_schema" {
                    let mut format = value["json_schema"].clone();
                    format["type"] = json!("json_schema");
                    format
                } else {
                    value.clone()
                };
                if body.get("text").is_none() {
                    body["text"] = json!({});
                }
                body["text"]["format"] = format;
            }
            _ => body[key] = value.clone(),
        }
    }
    if !s.tools.is_empty() {
        body["tools"] = Value::Array(s.tools.iter().map(|t| json!({"type":"function","name":t.name,"description":t.description,"parameters":t.parameters,"strict":t.strict})).collect());
    }
    Ok(body)
}
pub(super) fn decode(plan: &LlmInvocationPlan, v: &Value) -> LlmCallResult<LlmResponse> {
    if v["status"] == "failed" {
        return Err(invalid("Responses 报告生成失败"));
    }
    let items = v["output"]
        .as_array()
        .ok_or_else(|| invalid("Responses 缺少 output"))?;
    let mut content = Vec::new();
    for item in items {
        match item["type"].as_str() {
            Some("message") => {
                if let Some(parts) = item["content"].as_array() {
                    for part in parts {
                        match part["type"].as_str() {
                            Some("output_text") => content.push(LlmContent::Text {
                                text: string(part, "text").unwrap_or_default(),
                            }),
                            Some("refusal") => content.push(LlmContent::Refusal {
                                text: string(part, "refusal").unwrap_or_default(),
                            }),
                            _ => return Err(invalid("Responses 返回尚不支持的消息内容")),
                        }
                    }
                }
            }
            Some("function_call") => content.push(LlmContent::ToolCall {
                id: string(item, "call_id").ok_or_else(|| invalid("工具调用缺少 call_id"))?,
                name: string(item, "name").ok_or_else(|| invalid("工具调用缺少名称"))?,
                arguments: function_arguments(&item["arguments"])?,
                signature: None,
            }),
            Some("reasoning") => {
                if let Some(summary) = item["summary"].as_array() {
                    for part in summary {
                        if let Some(text) = part["text"].as_str() {
                            content.push(LlmContent::Reasoning { text: text.into() });
                        }
                    }
                }
            }
            _ => return Err(invalid("Responses 返回尚不支持的输出类型")),
        }
    }
    let reason = string(&v["incomplete_details"], "reason").or_else(|| string(v, "status"));
    Ok(base_response(
        plan,
        v,
        vec![LlmOutput {
            index: 0,
            content,
            finish_reason: reason,
        }],
        false,
    ))
}
