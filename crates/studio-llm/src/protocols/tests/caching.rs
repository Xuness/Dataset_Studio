use super::*;

fn router(parameters: Value) -> LlmInvocationPlan {
    let mut p = plan(
        LlmProtocol::OpenaiChat,
        LlmProviderKind::Openrouter,
        parameters,
    );
    p.snapshot.remote_model_id = "google/gemini-3.8-flash".into();
    p.snapshot.messages.insert(
        0,
        LlmMessage {
            role: LlmRole::System,
            content: vec![LlmContent::Text {
                text: "Stable evaluation criteria".into(),
            }],
        },
    );
    p.snapshot.messages[1].content.push(LlmContent::Image {
        url: "data:image/png;base64,first".into(),
        detail: None,
    });
    p
}

#[test]
fn stable_session_ignores_images_attempts_and_user_text_but_changes_with_system_and_model() {
    let mut p = router(json!({"openrouter.cache_affinity":true}));
    let first = encode(&p, true).unwrap();
    p.snapshot.invocation_id = "retry".into();
    p.snapshot.messages[1].content = vec![LlmContent::Text {
        text: "Different user".into(),
    }];
    assert_eq!(
        first["session_id"],
        encode(&p, false).unwrap()["session_id"]
    );
    p.snapshot.messages[0].content = vec![LlmContent::Text {
        text: "Changed criteria".into(),
    }];
    assert_ne!(
        first["session_id"],
        encode(&p, false).unwrap()["session_id"]
    );
    p.snapshot
        .parameters
        .insert("openrouter.session_id".into(), json!("stage-session"));
    assert_eq!(encode(&p, false).unwrap()["session_id"], "stage-session");
    assert!(first.get("cache_affinity").is_none());
    assert!(
        encode(&router(json!({})), false)
            .unwrap()
            .get("session_id")
            .is_none()
    );
}

#[test]
fn explicit_cache_marks_only_the_complete_system_and_preserves_dynamic_images() {
    let p = router(json!({"openrouter.cache_strategy":"system"}));
    let wire = encode(&p, false).unwrap();
    assert_eq!(
        wire["messages"][0]["content"][0]["cache_control"],
        json!({"type":"ephemeral"})
    );
    assert_eq!(
        wire["messages"][0]["content"][0]["text"],
        "Stable evaluation criteria"
    );
    assert_eq!(
        wire["messages"][1]["content"][1]["image_url"]["url"],
        "data:image/png;base64,first"
    );
    assert!(!wire["messages"][1].to_string().contains("cache_control"));
    assert!(wire.get("cache_strategy").is_none());
    let mut muse = p.clone();
    muse.snapshot.remote_model_id = "meta/muse-spark-1.3-contributor".into();
    assert!(encode(&muse, false).is_err());
    muse.snapshot
        .parameters
        .insert("openrouter.cache_strategy".into(), json!("implicit"));
    assert!(
        !encode(&muse, false)
            .unwrap()
            .to_string()
            .contains("cache_control")
    );
}

#[test]
fn flex_is_bounded_by_exact_endpoints_even_when_no_provider_can_serve_the_model() {
    let p = router(
        json!({"service_tier":"flex","openrouter.provider":{"order":["google-ai-studio/flex","google-vertex/global/flex"],"allow_fallbacks":true}}),
    );
    let wire = encode(&p, false).unwrap();
    assert_eq!(wire["provider"]["only"], wire["provider"]["order"]);
    assert_eq!(wire["service_tier"], "flex");
    for provider in [
        json!({}),
        json!({"only":[]}),
        json!({"only":["google-ai-studio"]}),
        json!({"only":["google-ai-studio/flex","google-vertex"]}),
    ] {
        assert!(
            encode(
                &router(json!({"service_tier":"flex","openrouter.provider":provider})),
                false
            )
            .is_err()
        );
    }
}

#[test]
fn streaming_and_json_keep_cache_cost_provider_and_actual_tier_and_old_usage_stays_unknown() {
    let p = router(json!({}));
    let value = json!({"id":"test","provider":"Google AI Studio","service_tier":"flex", "choices":[{"index":0,"message":{"content":"ok"},"finish_reason":"stop"}], "usage":{"prompt_tokens":8000,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":6000,"cache_write_tokens":0},"cost":0.0009}});
    let complete = decode(&p, &value, None).unwrap();
    let mut c = streaming::Collector::new(p);
    c.push(&json!({"provider":value["provider"],"service_tier":value["service_tier"],"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}).to_string()).unwrap();
    c.push(&json!({"choices":[],"usage":value["usage"]}).to_string())
        .unwrap();
    c.push("[DONE]").unwrap();
    let streamed = c.finish(None).unwrap();
    assert_eq!(
        serde_json::to_value(&complete.usage).unwrap(),
        serde_json::to_value(&streamed.usage).unwrap()
    );
    assert_eq!(streamed.usage.cached_input_tokens, Some(6000));
    assert_eq!(streamed.usage.cache_write_tokens, Some(0));
    assert_eq!(streamed.usage.cost_usd, Some(0.0009));
    assert_eq!(streamed.usage.service_tier.as_deref(), Some("flex"));
    let legacy: LlmUsage =
        serde_json::from_value(json!({"input_tokens":17,"cached_input_tokens":0})).unwrap();
    assert_eq!(legacy.cache_write_tokens, None);
    assert_eq!(legacy.cost_usd, None);
}
