use super::*;
use serde_json::json;
use studio_domain::llm::*;
fn parameters(v: serde_json::Value) -> LlmParameters {
    serde_json::from_value(v).unwrap()
}
#[test]
fn explicit_omission_and_false_zero_survive_layering() {
    let specs = parameter_specs(LlmProtocol::OpenaiChat, LlmProviderKind::OpenaiCompatible);
    let base = parameters(json!({"temperature":0.8,"store":true,"top_p":0.9}));
    let preset = parameters(json!({"temperature":0.4,"store":false}));
    let request = parameters(json!({"temperature":0,"top_p":null}));
    let (value, warnings) = resolve_parameters(&[&base, &preset, &request], &specs).unwrap();
    assert_eq!(value, parameters(json!({"temperature":0,"store":false})));
    assert_eq!(warnings.len(), 2);
}
#[test]
fn known_unsupported_can_be_omitted_but_not_silently_sent() {
    let mut specs = parameter_specs(LlmProtocol::OpenaiChat, LlmProviderKind::OpenaiCompatible);
    specs
        .iter_mut()
        .find(|s| s.key == "temperature")
        .unwrap()
        .support = LlmSupport::Unsupported;
    let base = parameters(json!({"temperature":0.8}));
    assert!(resolve_parameters(&[&base], &specs).is_err());
    assert!(resolve_parameters(&[&base, &parameters(json!({"temperature":null}))], &specs).is_ok());
}
#[test]
fn protocol_extensions_and_conflicts_are_not_lost() {
    let chat = parameter_specs(LlmProtocol::OpenaiChat, LlmProviderKind::OpenaiCompatible);
    assert!(validate_parameters(&parameters(json!({"model":"oops"})), &chat).is_err());
    assert!(validate_parameters(&parameters(json!({"openrouter.provider":{}})), &chat).is_err());
    let router = parameter_specs(LlmProtocol::OpenaiChat, LlmProviderKind::Openrouter);
    assert!(
        resolve_parameters(
            &[&parameters(
                json!({"reasoning_effort":"high","reasoning_budget":1000})
            )],
            &router
        )
        .is_err()
    );
    let responses = parameter_specs(LlmProtocol::OpenaiResponses, LlmProviderKind::Openai);
    assert!(!responses.iter().any(|s| s.key == "seed"));
    assert!(resolve_parameters(&[&parameters(json!({"top_logprobs":4}))], &chat).is_err());
}

#[test]
fn openrouter_cache_parameters_are_scoped_and_session_ids_are_bounded() {
    let router = parameter_specs(LlmProtocol::OpenaiChat, LlmProviderKind::Openrouter);
    assert!(validate_parameters(&parameters(json!({"openrouter.cache_affinity":true,"openrouter.cache_strategy":"system","service_tier":"flex"})), &router).is_ok());
    for id in [String::new(), "x".repeat(257), "line\nbreak".into()] {
        assert!(
            validate_parameters(&parameters(json!({"openrouter.session_id":id})), &router).is_err()
        );
    }
    let direct = parameter_specs(LlmProtocol::OpenaiChat, LlmProviderKind::OpenaiCompatible);
    assert!(
        validate_parameters(
            &parameters(json!({"openrouter.cache_strategy":"implicit"})),
            &direct
        )
        .is_err()
    );
}
