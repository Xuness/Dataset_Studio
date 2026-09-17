use super::*;
use serde_json::json;

#[test]
fn v1_snapshot_without_system_prompt_metadata_remains_readable() {
    let legacy = json!({
        "schema_version": 1,
        "invocation_id": "call", "provider_id": "provider", "provider_revision": 1,
        "provider_kind": "openai", "base_url": "http://127.0.0.1/v1",
        "model_id": "model", "model_revision": 1, "remote_model_id": "remote-model",
        "protocol": "openai_chat", "parameters": {},
        "messages": [{"role":"system","content":[{"type":"text","text":"old instructions"}]}],
        "tools": [], "warnings": []
    });
    let dto: LlmInvocationSnapshot = serde_json::from_value(legacy).unwrap();
    let domain: studio_domain::llm::LlmInvocationSnapshot = dto.into();
    assert_eq!(domain.schema_version, 1);
    assert!(domain.system_prompt_id.is_none());
    assert!(domain.system_prompt_revision.is_none());
    let roundtrip = serde_json::to_value(LlmInvocationSnapshot::from(domain)).unwrap();
    assert_eq!(
        roundtrip["messages"][0]["content"][0]["text"],
        "old instructions"
    );
}
