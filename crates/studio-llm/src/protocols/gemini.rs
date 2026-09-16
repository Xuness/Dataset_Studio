use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) fn encode(plan: &LlmInvocationPlan) -> Result<Value> {
    let s = &plan.snapshot;
    let mut contents = Vec::new();
    let mut system = None;
    for (ordinal, message) in s.messages.iter().enumerate() {
        if message.role == LlmRole::Developer {
            return Err(Error::invalid(
                "Gemini 不提供独立 developer 优先级，请由调用方选择 system 消息",
            ));
        }
        let mut parts = Vec::new();
        for block in &message.content {
            match block {
                LlmContent::Text { text } => parts.push(json!({"text":text})),
                LlmContent::Image { url, detail } => {
                    if detail.is_some() { return Err(Error::invalid("Gemini 图片精度请使用 gemini.media_resolution")); }
                    if let Some(data) = url.strip_prefix("data:") {
                        let (header, data) = data.split_once(',').ok_or_else(|| Error::invalid("图片 data URL 无效"))?;
                        let mime = header.strip_suffix(";base64").ok_or_else(|| Error::invalid("Gemini 内联图片需要 base64"))?;
                        STANDARD.decode(data).map_err(|_| Error::invalid("图片 base64 无效"))?;
                        parts.push(json!({"inlineData":{"mimeType":mime,"data":data}}));
                    } else {
                        // Gemini Files URI is already prepared by the caller; no filesystem access.
                        if !url.starts_with("https://generativelanguage.googleapis.com/") && !url.starts_with("gs://") {
                            return Err(Error::invalid("Gemini 图片请传入 data URL 或已上传的 Gemini/Cloud Storage URI"));
                        }
                        parts.push(json!({"fileData":{"fileUri":url}}));
                    }
                },
                LlmContent::ToolCall { name, arguments, signature, .. } => {
                    let mut part = json!({"functionCall":{"name":name,"args":arguments}});
                    if let Some(signature) = signature { part["thoughtSignature"] = json!(signature); }
                    parts.push(part);
                },
                LlmContent::ToolResult { name, value, .. } => parts.push(json!({"functionResponse":{"name":name,"response":if value.is_object() { value.clone() } else { json!({"result":value}) }}})),
                _ => return Err(Error::invalid("Gemini 不支持该输入内容块")),
            }
        }
        if message.role == LlmRole::System {
            if ordinal != 0
                || system.is_some()
                || message
                    .content
                    .iter()
                    .any(|p| !matches!(p, LlmContent::Text { .. }))
            {
                return Err(Error::invalid(
                    "Gemini 支持一条位于开头的纯文本 system 消息",
                ));
            }
            system = Some(json!({"parts":parts}));
        } else {
            contents.push(json!({"role":if message.role == LlmRole::Assistant { "model" } else { "user" },"parts":parts}));
        }
    }
    if contents.is_empty() {
        return Err(Error::invalid("Gemini 至少需要一条非 system 消息"));
    }
    let mut body = json!({"contents":contents});
    if let Some(system) = system {
        body["systemInstruction"] = system;
    }
    let mut generation = json!({});
    let mut thinking = json!({});
    for (key, value) in &s.parameters {
        match key.as_str() {
            "temperature" | "seed" => generation[key] = value.clone(),
            "top_p" => generation["topP"] = value.clone(),
            "top_k" => generation["topK"] = value.clone(),
            "max_output_tokens" => generation["maxOutputTokens"] = value.clone(),
            "stop" => generation["stopSequences"] = value.clone(),
            "presence_penalty" => generation["presencePenalty"] = value.clone(),
            "frequency_penalty" => generation["frequencyPenalty"] = value.clone(),
            "reasoning_budget" => thinking["thinkingBudget"] = value.clone(),
            "gemini.thinking_level" => thinking["thinkingLevel"] = value.clone(),
            "gemini.include_thoughts" => thinking["includeThoughts"] = value.clone(),
            "gemini.media_resolution" => generation["mediaResolution"] = value.clone(),
            "gemini.safety_settings" => body["safetySettings"] = value.clone(),
            "gemini.cached_content" => body["cachedContent"] = value.clone(),
            "response_format" => {
                generation["responseMimeType"] = json!(if value["type"] == "text" {
                    "text/plain"
                } else {
                    "application/json"
                });
                if value["type"] == "json_schema" {
                    generation["responseJsonSchema"] = value["json_schema"]["schema"].clone();
                }
            }
            "tool_choice" => {
                body["toolConfig"] = json!({"functionCallingConfig":{"mode":match value.as_str() { Some("required") => "ANY", Some("none") => "NONE", _ => "AUTO" }}})
            }
            _ => return Err(Error::invalid(format!("Gemini 尚未映射参数：{key}"))),
        }
    }
    if thinking.as_object().is_some_and(|v| !v.is_empty()) {
        generation["thinkingConfig"] = thinking;
    }
    if generation.as_object().is_some_and(|v| !v.is_empty()) {
        body["generationConfig"] = generation;
    }
    if !s.tools.is_empty() {
        if s.tools.iter().any(|t| t.strict) {
            return Err(Error::invalid(
                "Gemini 工具接口不提供 OpenAI strict 语义，请使用 strict=false",
            ));
        }
        body["tools"] = json!([{"functionDeclarations":s.tools.iter().map(|t| json!({"name":t.name,"description":t.description,"parametersJsonSchema":t.parameters})).collect::<Vec<_>>()}]);
    }
    Ok(body)
}
pub(super) fn decode(plan: &LlmInvocationPlan, v: &Value) -> LlmCallResult<LlmResponse> {
    let mut outputs = Vec::new();
    if let Some(candidates) = v["candidates"].as_array() {
        for (ordinal, candidate) in candidates.iter().enumerate() {
            let index = candidate["index"].as_u64().unwrap_or(ordinal as u64) as u32;
            let mut content = Vec::new();
            if let Some(parts) = candidate["content"]["parts"].as_array() {
                for (part_index, part) in parts.iter().enumerate() {
                    if let Some(text) = part["text"].as_str() {
                        content.push(if part["thought"] == true {
                            LlmContent::Reasoning { text: text.into() }
                        } else {
                            LlmContent::Text { text: text.into() }
                        });
                    } else if let Some(call) = part.get("functionCall") {
                        content.push(LlmContent::ToolCall {
                            id: string(call, "id")
                                .unwrap_or_else(|| format!("gemini-{index}-{part_index}")),
                            name: string(call, "name")
                                .ok_or_else(|| invalid("Gemini 工具调用缺少名称"))?,
                            arguments: function_arguments(&call["args"])?,
                            signature: string(part, "thoughtSignature"),
                        });
                    } else {
                        return Err(invalid("Gemini 返回尚不支持的内容块"));
                    }
                }
            }
            outputs.push(LlmOutput {
                index,
                content,
                finish_reason: string(candidate, "finishReason"),
            });
        }
    }
    if outputs.is_empty() {
        if let Some(reason) = string(&v["promptFeedback"], "blockReason") {
            outputs.push(LlmOutput {
                index: 0,
                content: vec![LlmContent::Refusal {
                    text: "供应商阻止了本次输入".into(),
                }],
                finish_reason: Some(reason),
            });
        } else {
            return Err(invalid("Gemini 缺少 candidates 或拒绝原因"));
        }
    }
    Ok(base_response(plan, v, outputs, true))
}
