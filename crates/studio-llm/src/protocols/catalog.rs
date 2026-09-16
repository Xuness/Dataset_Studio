use super::*;
use std::collections::BTreeMap;

pub fn page(
    kind: LlmProviderKind,
    value: &Value,
) -> LlmCallResult<(Vec<LlmCatalogModel>, Option<String>)> {
    let gemini = kind == LlmProviderKind::Gemini;
    let rows = value[if gemini { "models" } else { "data" }]
        .as_array()
        .ok_or_else(|| invalid("供应商模型目录格式不兼容，可手动添加模型"))?;
    let mut models = Vec::new();
    for row in rows {
        let id = string(row, if gemini { "name" } else { "id" })
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| invalid("模型目录包含无效模型 ID"))?;
        let mut capabilities = BTreeMap::new();
        if let Some(parameters) = row["supported_parameters"].as_array() {
            for parameter in parameters
                .iter()
                .take(128)
                .filter_map(Value::as_str)
                .filter(|s| s.len() <= 120)
            {
                let key = match parameter {
                    "max_tokens" | "max_completion_tokens" => "max_output_tokens",
                    "reasoning" => "reasoning_effort",
                    _ => parameter,
                };
                capabilities.insert(key.into(), LlmSupport::Supported);
            }
        }
        if gemini {
            if row["thinking"].as_bool() == Some(true) {
                capabilities.insert("reasoning_budget".into(), LlmSupport::Supported);
            }
            for (field, key) in [
                ("temperature", "temperature"),
                ("topP", "top_p"),
                ("topK", "top_k"),
            ] {
                if row[field].is_number() {
                    capabilities.insert(key.into(), LlmSupport::Supported);
                }
            }
        }
        let modalities = |key: &str| {
            row["architecture"][key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .take(16)
                        .filter(|s| s.len() <= 64)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let input_modalities = modalities("input_modalities");
        if input_modalities.iter().any(|s| s == "image") {
            capabilities.insert("input_image".into(), LlmSupport::Supported);
        }
        models.push(LlmCatalogModel {
            id,
            name: string(row, if gemini { "displayName" } else { "name" })
                .unwrap_or_default()
                .chars()
                .take(240)
                .collect(),
            input_token_limit: row[if gemini {
                "inputTokenLimit"
            } else {
                "context_length"
            }]
            .as_u64(),
            output_token_limit: if gemini {
                row["outputTokenLimit"].as_u64()
            } else {
                row["top_provider"]["max_completion_tokens"].as_u64()
            },
            input_modalities,
            output_modalities: modalities("output_modalities"),
            capabilities,
        });
    }
    let next = if gemini {
        string(value, "nextPageToken")
    } else if value["has_more"] == true {
        string(value, "last_id").or_else(|| models.last().map(|m| m.id.clone()))
    } else {
        None
    };
    Ok((models, next))
}
