use serde_json::{Value, json};
use studio_domain::{Error, Result, llm::LlmParameters};
pub fn apply(body: &mut Value, parameters: &LlmParameters) -> Result<()> {
    for (key, value) in parameters {
        if let Some(key) = key.strip_prefix("openrouter.") {
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
    Ok(())
}
