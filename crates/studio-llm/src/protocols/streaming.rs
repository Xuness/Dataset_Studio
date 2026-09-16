use super::*;
use std::collections::BTreeMap;

/// Protocol stream state is scoped to one request, never shared between models.
pub struct Collector {
    plan: LlmInvocationPlan,
    chat: Value,
    gemini: BTreeMap<u32, LlmOutput>,
    latest: Value,
    response: Option<LlmResponse>,
    pub terminal: bool,
}
impl Collector {
    pub fn new(plan: LlmInvocationPlan) -> Self {
        Self {
            plan,
            chat: json!({"choices":[]}),
            gemini: BTreeMap::new(),
            latest: json!({}),
            response: None,
            terminal: false,
        }
    }
    pub fn push(&mut self, data: &str) -> LlmCallResult<Vec<LlmEvent>> {
        if data == "[DONE]" {
            if self.plan.snapshot.protocol != LlmProtocol::OpenaiChat {
                return Err(invalid("当前协议不使用 [DONE] 终止标记"));
            }
            self.terminal = true;
            return Ok(Vec::new());
        }
        let value: Value =
            serde_json::from_str(data).map_err(|_| invalid("流事件不是有效 JSON"))?;
        if value.get("error").is_some_and(|v| !v.is_null()) || value["type"] == "error" {
            return Err(LlmFailure::new("LLM_UPSTREAM", "供应商在流中报告错误"));
        }
        match self.plan.snapshot.protocol {
            LlmProtocol::OpenaiChat => self.chat_event(&value),
            LlmProtocol::OpenaiResponses => self.responses_event(&value),
            LlmProtocol::Gemini => self.gemini_event(&value),
        }
    }
    fn chat_event(&mut self, value: &Value) -> LlmCallResult<Vec<LlmEvent>> {
        for key in ["id", "model", "usage"] {
            if !value[key].is_null() {
                self.chat[key] = value[key].clone();
            }
        }
        let mut events = Vec::new();
        if let Some(choices) = value["choices"].as_array() {
            for choice in choices {
                let index = choice["index"].as_u64().unwrap_or(0) as usize;
                if index >= 16 {
                    return Err(invalid("候选响应数量超过上限"));
                }
                let rows = self.chat["choices"]
                    .as_array_mut()
                    .expect("initialized array");
                while rows.len() <= index {
                    rows.push(json!({"index":rows.len(),"message":{"content":"","tool_calls":[]}}));
                }
                let row = &mut rows[index];
                if !choice["finish_reason"].is_null() {
                    row["finish_reason"] = choice["finish_reason"].clone();
                }
                for (key, kind) in [
                    ("content", "text"),
                    ("reasoning", "reasoning"),
                    ("reasoning_content", "reasoning"),
                    ("refusal", "refusal"),
                ] {
                    if let Some(text) = choice["delta"][key].as_str() {
                        append(&mut row["message"], key, text);
                        events.push(delta(index as u32, kind, text, None));
                    }
                }
                if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
                    for call in calls {
                        let ordinal = call["index"].as_u64().unwrap_or(0) as usize;
                        if ordinal >= 128 {
                            return Err(invalid("工具调用数量超过上限"));
                        }
                        let saved = row["message"]["tool_calls"]
                            .as_array_mut()
                            .expect("initialized array");
                        while saved.len() <= ordinal {
                            saved.push(json!({"id":"","function":{"name":"","arguments":""}}));
                        }
                        let saved = &mut saved[ordinal];
                        if let Some(id) = call["id"].as_str() {
                            append(saved, "id", id);
                        }
                        if let Some(name) = call["function"]["name"].as_str() {
                            append(&mut saved["function"], "name", name);
                        }
                        if let Some(text) = call["function"]["arguments"].as_str() {
                            append(&mut saved["function"], "arguments", text);
                            events.push(delta(
                                index as u32,
                                "tool_arguments",
                                text,
                                string(saved, "id"),
                            ));
                        }
                    }
                }
            }
        }
        Ok(events)
    }
    fn responses_event(&mut self, value: &Value) -> LlmCallResult<Vec<LlmEvent>> {
        let kind = value["type"]
            .as_str()
            .ok_or_else(|| invalid("Responses 事件缺少 type"))?;
        match kind {
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.reasoning_summary_text.delta"
            | "response.function_call_arguments.delta" => {
                let event_kind = match kind {
                    "response.refusal.delta" => "refusal",
                    "response.reasoning_summary_text.delta" => "reasoning",
                    "response.function_call_arguments.delta" => "tool_arguments",
                    _ => "text",
                };
                Ok(vec![delta(
                    0,
                    event_kind,
                    value["delta"].as_str().unwrap_or(""),
                    string(value, "item_id"),
                )])
            }
            "response.completed" | "response.incomplete" => {
                self.response = Some(responses::decode(&self.plan, &value["response"])?);
                self.terminal = true;
                Ok(Vec::new())
            }
            "response.failed" => Err(LlmFailure::new(
                "LLM_UPSTREAM",
                "Responses 在生成中报告失败",
            )),
            _ => Ok(Vec::new()),
        }
    }
    fn gemini_event(&mut self, value: &Value) -> LlmCallResult<Vec<LlmEvent>> {
        let mut events = Vec::new();
        for key in ["usageMetadata", "responseId", "modelVersion"] {
            if !value[key].is_null() {
                self.latest[key] = value[key].clone();
            }
        }
        if value.get("candidates").is_none() && value.get("promptFeedback").is_none() {
            return Ok(events);
        }
        let response = gemini::decode(&self.plan, value)?;
        for output in response.outputs {
            let saved = self
                .gemini
                .entry(output.index)
                .or_insert_with(|| LlmOutput {
                    index: output.index,
                    content: Vec::new(),
                    finish_reason: None,
                });
            if output.finish_reason.is_some() {
                saved.finish_reason = output.finish_reason;
            }
            for mut content in output.content {
                match &mut content {
                    LlmContent::Text { text } => {
                        events.push(delta(output.index, "text", text, None))
                    }
                    LlmContent::Reasoning { text } => {
                        events.push(delta(output.index, "reasoning", text, None))
                    }
                    LlmContent::ToolCall { id, arguments, .. } => {
                        *id = format!("gemini-{}-{}", output.index, saved.content.len());
                        events.push(delta(
                            output.index,
                            "tool_arguments",
                            &arguments.to_string(),
                            Some(id.clone()),
                        ));
                    }
                    _ => {}
                }
                saved.content.push(content);
            }
        }
        self.terminal =
            !self.gemini.is_empty() && self.gemini.values().all(|v| v.finish_reason.is_some());
        Ok(events)
    }
    pub fn finish(self, request_id: Option<String>) -> LlmCallResult<LlmResponse> {
        if !self.terminal {
            return Err(LlmFailure::new(
                "LLM_STREAM_INTERRUPTED",
                "流式响应缺少完成标记",
            ));
        }
        let mut response = match self.plan.snapshot.protocol {
            LlmProtocol::OpenaiChat => chat::decode(&self.plan, &self.chat)?,
            LlmProtocol::OpenaiResponses => self
                .response
                .ok_or_else(|| invalid("Responses 缺少最终响应"))?,
            LlmProtocol::Gemini => base_response(
                &self.plan,
                &self.latest,
                self.gemini.into_values().collect(),
                true,
            ),
        };
        response.provider_request_id = request_id;
        Ok(response)
    }
}
fn append(value: &mut Value, key: &str, text: &str) {
    if let Some(Value::String(saved)) = value.get_mut(key) {
        saved.push_str(text);
    } else {
        value[key] = json!(text);
    }
}
fn delta(index: u32, kind: &str, text: &str, id: Option<String>) -> LlmEvent {
    LlmEvent::Delta {
        index,
        kind: kind.into(),
        text: text.into(),
        tool_call_id: id,
    }
}
