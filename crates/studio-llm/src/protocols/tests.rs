use super::*;
mod caching;
fn plan(protocol: LlmProtocol, kind: LlmProviderKind, parameters: Value) -> LlmInvocationPlan {
    LlmInvocationPlan {
        provider: LlmProvider {
            id: "provider".into(),
            revision: 1,
            credential_ref: None,
            config: LlmConnectionConfig {
                name: "test".into(),
                kind,
                base_url: "http://127.0.0.1/v1".into(),
                enabled: true,
                network: Default::default(),
                headers: Default::default(),
            },
        },
        snapshot: LlmInvocationSnapshot {
            schema_version: LLM_SCHEMA_VERSION,
            invocation_id: "request".into(),
            provider_id: "provider".into(),
            provider_revision: 1,
            provider_kind: kind,
            base_url: "http://127.0.0.1/v1".into(),
            model_id: "model".into(),
            model_revision: 1,
            remote_model_id: "remote-model".into(),
            protocol,
            preset_id: None,
            preset_revision: None,
            system_prompt_id: None,
            system_prompt_revision: None,
            parameters: serde_json::from_value(parameters).unwrap(),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: vec![LlmContent::Text {
                    text: "测试".into(),
                }],
            }],
            tools: Vec::new(),
            warnings: Vec::new(),
        },
    }
}
#[test]
fn protocol_parameter_shapes_remain_distinct() {
    let chat = plan(
        LlmProtocol::OpenaiChat,
        LlmProviderKind::Openai,
        json!({"max_output_tokens":42,"reasoning_effort":"high"}),
    );
    let wire = encode(&chat, false).unwrap();
    assert_eq!(wire["max_completion_tokens"], 42);
    assert_eq!(wire["reasoning_effort"], "high");
    assert!(wire.get("store").is_none()); // Application defaults precede explicit omission.
    let router = plan(
        LlmProtocol::OpenaiChat,
        LlmProviderKind::Openrouter,
        json!({"reasoning_effort":"high","openrouter.provider":{"require_parameters":true}}),
    );
    let wire = encode(&router, false).unwrap();
    assert_eq!(wire["reasoning"], json!({"effort":"high"}));
    assert!(wire.get("reasoning_effort").is_none());
    assert_eq!(wire["provider"]["require_parameters"], true);
    let responses = plan(
        LlmProtocol::OpenaiResponses,
        LlmProviderKind::Openai,
        json!({"response_format":{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object"},"strict":true}},"verbosity":"low"}),
    );
    let wire = encode(&responses, false).unwrap();
    assert_eq!(wire["text"]["format"]["type"], "json_schema");
    assert_eq!(wire["text"]["verbosity"], "low");
    assert!(wire.get("messages").is_none());
    let gemini = plan(
        LlmProtocol::Gemini,
        LlmProviderKind::Gemini,
        json!({"max_output_tokens":42,"reasoning_budget":1000,"temperature":0}),
    );
    let wire = encode(&gemini, false).unwrap();
    assert_eq!(
        wire["generationConfig"],
        json!({"maxOutputTokens":42,"thinkingConfig":{"thinkingBudget":1000},"temperature":0})
    );
}
#[test]
fn null_error_is_not_a_failure_and_missing_usage_is_unknown() {
    let p = plan(
        LlmProtocol::OpenaiResponses,
        LlmProviderKind::Openai,
        json!({}),
    );
    let value = json!({"id":"response","error":null,"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]});
    let r = decode(&p, &value, None).unwrap();
    assert_eq!(r.usage.input_tokens, None);
    assert!(matches!(&r.outputs[0].content[0],LlmContent::Text {text} if text=="ok"));
}

#[test]
fn assistant_history_and_tool_only_messages_use_valid_input_shapes() {
    let mut p = plan(
        LlmProtocol::OpenaiResponses,
        LlmProviderKind::Openai,
        json!({}),
    );
    p.snapshot.messages[0].role = LlmRole::Assistant;
    assert_eq!(
        encode(&p, false).unwrap()["input"][0]["content"][0]["type"],
        "input_text"
    );
    p.snapshot.protocol = LlmProtocol::OpenaiChat;
    p.snapshot.messages[0].content = vec![LlmContent::ToolCall {
        id: "call-1".into(),
        name: "lookup".into(),
        arguments: json!({}),
        signature: None,
    }];
    let wire = encode(&p, false).unwrap();
    assert_eq!(wire["messages"][0]["content"], Value::Null);
    assert_eq!(
        wire["messages"][0]["tool_calls"][0]["function"]["arguments"],
        "{}"
    );
}
#[test]
fn chat_stream_requires_terminal_and_reassembles_tool_arguments() {
    let p = plan(
        LlmProtocol::OpenaiChat,
        LlmProviderKind::OpenaiCompatible,
        json!({}),
    );
    let mut c = streaming::Collector::new(p.clone());
    c.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","function":{"name":"test","arguments":"{\"x\":"}}]}}]}).to_string()).unwrap();
    c.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}]}).to_string()).unwrap();
    c.push("[DONE]").unwrap();
    let r = c.finish(None).unwrap();
    assert!(
        r.outputs[0]
            .content
            .iter()
            .any(|b| matches!(b,LlmContent::ToolCall {arguments,..} if arguments==&json!({"x":1})))
    );
    let c = streaming::Collector::new(p);
    assert_eq!(c.finish(None).unwrap_err().code, "LLM_STREAM_INTERRUPTED");
}
#[test]
fn gemini_signature_is_preserved_and_roles_do_not_flatten() {
    let mut p = plan(LlmProtocol::Gemini, LlmProviderKind::Gemini, json!({}));
    let r=decode(&p,&json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"test","args":{}},"thoughtSignature":"opaque"}]},"finishReason":"STOP"}]}),None).unwrap();
    assert!(
        matches!(&r.outputs[0].content[0],LlmContent::ToolCall {signature:Some(s),..} if s=="opaque")
    );
    p.snapshot.messages[0].role = LlmRole::Developer;
    assert!(encode(&p, false).is_err());
}
