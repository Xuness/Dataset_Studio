use std::collections::BTreeMap;
use studio_domain::{Error, Result, llm::*};

pub fn parameter_specs(protocol: LlmProtocol, kind: LlmProviderKind) -> Vec<LlmParameterSpec> {
    let mut specs = Vec::new();
    let mut add = |key: &str,
                   label: &str,
                   ty: &str,
                   group: &str,
                   min: Option<f64>,
                   max: Option<f64>,
                   choices: &[&str],
                   description: &str| {
        specs.push(LlmParameterSpec {
            key: key.into(),
            label: label.into(),
            value_type: ty.into(),
            group: group.into(),
            minimum: min,
            maximum: max,
            choices: choices.iter().map(|s| (*s).into()).collect(),
            description: description.into(),
            support: LlmSupport::Unknown,
            evidence: "protocol".into(),
        });
    };
    add(
        "temperature",
        "Temperature",
        "number",
        "sampling",
        Some(0.),
        Some(2.),
        &[],
        "采样温度；具体范围由模型决定",
    );
    add(
        "top_p",
        "Top P",
        "number",
        "sampling",
        Some(0.),
        Some(1.),
        &[],
        "累计概率采样",
    );
    add(
        "max_output_tokens",
        "最大输出 Token",
        "integer",
        "generation",
        Some(1.),
        Some(16_777_216.),
        &[],
        "协议会转换为相应的输出长度字段；可能包含推理消耗",
    );
    add(
        "stop",
        "停止序列",
        "array",
        "generation",
        None,
        None,
        &[],
        "字符串数组，最多 16 项",
    );
    add(
        "seed",
        "随机种子",
        "integer",
        "sampling",
        Some(-2_147_483_648.),
        Some(2_147_483_647.),
        &[],
        "可重复性取决于上游模型",
    );
    add(
        "presence_penalty",
        "存在惩罚",
        "number",
        "sampling",
        Some(-2.),
        Some(2.),
        &[],
        "降低重复主题",
    );
    add(
        "frequency_penalty",
        "频率惩罚",
        "number",
        "sampling",
        Some(-2.),
        Some(2.),
        &[],
        "降低重复词频",
    );
    add(
        "response_format",
        "输出格式",
        "object",
        "generation",
        None,
        None,
        &[],
        "{type: text | json_object | json_schema, json_schema?: {name, schema, strict}}",
    );
    add(
        "tool_choice",
        "工具选择",
        "string",
        "tools",
        None,
        None,
        &["auto", "none", "required"],
        "仅发送工具声明；工具执行由调用模块负责",
    );
    if protocol != LlmProtocol::Gemini {
        add(
            "reasoning_effort",
            "推理等级",
            "string",
            "reasoning",
            None,
            None,
            &["none", "minimal", "low", "medium", "high", "xhigh", "max"],
            "有效等级由具体模型决定",
        );
        add(
            "verbosity",
            "输出详细程度",
            "string",
            "generation",
            None,
            None,
            &["low", "medium", "high"],
            "具体模型需支持该参数",
        );
        add(
            "parallel_tool_calls",
            "并行工具调用",
            "boolean",
            "tools",
            None,
            None,
            &[],
            "允许返回多个工具调用",
        );
        add(
            "store",
            "供应商保存响应",
            "boolean",
            "advanced",
            None,
            None,
            &[],
            "官方 OpenAI 默认显式关闭；其他兼容服务仅在设置时发送",
        );
    }
    if protocol == LlmProtocol::OpenaiChat {
        add(
            "logprobs",
            "返回 Token 概率",
            "boolean",
            "advanced",
            None,
            None,
            &[],
            "请求输出 Token 概率",
        );
        add(
            "top_logprobs",
            "Top Logprobs",
            "integer",
            "advanced",
            Some(0.),
            Some(20.),
            &[],
            "需要同时启用 logprobs",
        );
        add(
            "logit_bias",
            "Token 偏置",
            "object",
            "advanced",
            None,
            None,
            &[],
            "Token ID 到 -100–100 偏置的映射",
        );
        add(
            "stream_usage",
            "流式用量信息",
            "boolean",
            "advanced",
            None,
            None,
            &[],
            "发送 stream_options.include_usage；兼容服务可能不支持",
        );
        add(
            "token_limit_field",
            "输出长度字段",
            "string",
            "advanced",
            None,
            None,
            &["max_tokens", "max_completion_tokens"],
            "兼容接口可选择长度字段；不改变 Token 语义",
        );
    }
    if protocol == LlmProtocol::OpenaiResponses {
        add(
            "truncation",
            "上下文截断",
            "string",
            "advanced",
            None,
            None,
            &["disabled", "auto"],
            "默认不主动截断输入",
        );
    }
    if protocol == LlmProtocol::OpenaiResponses || kind == LlmProviderKind::Openrouter {
        add(
            "service_tier",
            "服务等级",
            "string",
            "advanced",
            None,
            None,
            &["auto", "default", "flex", "priority"],
            "OpenRouter 的 flex 使用严格限制：上游路由 only 需填写具体 /flex 端点；无可用容量时返回错误，不转普通价",
        );
    }
    if protocol == LlmProtocol::Gemini || kind == LlmProviderKind::Openrouter {
        add(
            "top_k",
            "Top K",
            "integer",
            "sampling",
            Some(1.),
            Some(10000.),
            &[],
            "候选 Token 数量",
        );
        add(
            "reasoning_budget",
            "推理 Token 预算",
            "integer",
            "reasoning",
            Some(-1.),
            Some(16_777_216.),
            &[],
            "是否支持自动预算 -1 由协议决定",
        );
    }
    if protocol == LlmProtocol::Gemini {
        add(
            "gemini.thinking_level",
            "Thinking Level",
            "string",
            "reasoning",
            None,
            None,
            &["minimal", "low", "medium", "high"],
            "与推理预算互斥；值由具体模型决定",
        );
        add(
            "gemini.include_thoughts",
            "返回推理摘要",
            "boolean",
            "reasoning",
            None,
            None,
            &[],
            "请求上游提供的推理摘要",
        );
        add(
            "gemini.safety_settings",
            "Safety Settings",
            "array",
            "advanced",
            None,
            None,
            &[],
            "Gemini 原生 category/threshold 数组",
        );
        add(
            "gemini.cached_content",
            "缓存内容引用",
            "string",
            "advanced",
            None,
            None,
            &[],
            "已存在的 cachedContents 资源名",
        );
        add(
            "gemini.media_resolution",
            "媒体分辨率",
            "string",
            "advanced",
            None,
            None,
            &[
                "MEDIA_RESOLUTION_LOW",
                "MEDIA_RESOLUTION_MEDIUM",
                "MEDIA_RESOLUTION_HIGH",
            ],
            "按模型支持情况设置",
        );
    }
    if kind == LlmProviderKind::Openrouter {
        add(
            "openrouter.cache_strategy",
            "提示词缓存策略",
            "string",
            "caching",
            None,
            None,
            &["implicit", "system"],
            "implicit 保留上游自动缓存；system 为 Gemini/Claude 的完整 System 指令添加显式缓存标记。须达到模型最低缓存长度，会产生写入或存储费用；变化的图片不加入显式缓存",
        );
        add(
            "openrouter.cache_affinity",
            "固定提示词路由",
            "boolean",
            "caching",
            None,
            None,
            &[],
            "开启后生成稳定 session ID，美学评审按阶段固定。可减少图片变化引起的端点切换；手动 provider.order 会优先于粘性路由，建议用 only 限定可用端点",
        );
        add(
            "openrouter.session_id",
            "自定义缓存会话",
            "string",
            "caching",
            None,
            None,
            &[],
            "可选，最多 256 字符。覆盖自动生成的会话 ID；只影响路由及日志分组，不共享消息历史",
        );
        add(
            "openrouter.provider",
            "上游路由",
            "object",
            "routing",
            None,
            None,
            &[],
            "OpenRouter provider 对象，可设置 order、only、require_parameters 等",
        );
        add(
            "openrouter.transforms",
            "输入变换",
            "array",
            "routing",
            None,
            None,
            &[],
            "OpenRouter transforms 字符串数组",
        );
        add(
            "openrouter.repetition_penalty",
            "重复惩罚",
            "number",
            "sampling",
            Some(0.),
            Some(2.),
            &[],
            "具体上游需支持",
        );
        add(
            "openrouter.min_p",
            "Min P",
            "number",
            "sampling",
            Some(0.),
            Some(1.),
            &[],
            "相对概率阈值",
        );
        add(
            "openrouter.top_a",
            "Top A",
            "number",
            "sampling",
            Some(0.),
            Some(1.),
            &[],
            "动态候选阈值",
        );
    }
    if protocol == LlmProtocol::OpenaiResponses {
        specs.retain(|s| {
            ![
                "seed",
                "stop",
                "frequency_penalty",
                "presence_penalty",
                "top_k",
                "reasoning_budget",
            ]
            .contains(&s.key.as_str())
                && !s.key.starts_with("openrouter.")
        });
    }
    specs
}

pub fn validate_parameters(values: &LlmParameters, specs: &[LlmParameterSpec]) -> Result<()> {
    if serde_json::to_vec(values).map_err(Error::io)?.len() > 65536 {
        return Err(Error::invalid("参数配置最多 64 KiB"));
    }
    for (key, value) in values {
        let spec = specs
            .iter()
            .find(|s| &s.key == key)
            .ok_or_else(|| Error::invalid(format!("协议未定义参数：{key}")))?;
        if value.is_null() {
            continue;
        }
        if key == "openrouter.session_id"
            && value.as_str().is_none_or(|s| {
                s.trim().is_empty() || s.chars().count() > 256 || s.chars().any(char::is_control)
            })
        {
            return Err(Error::invalid("缓存会话 ID 须为 1–256 个非控制字符"));
        }
        if spec.support == LlmSupport::Unsupported {
            return Err(Error::invalid(format!("模型明确不支持参数：{key}")));
        }
        let valid = match spec.value_type.as_str() {
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            "string" => value.is_string(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => false,
        };
        if !valid
            || value.as_f64().is_some_and(|n| {
                spec.minimum.is_some_and(|v| n < v) || spec.maximum.is_some_and(|v| n > v)
            })
            || (!spec.choices.is_empty()
                && !value
                    .as_str()
                    .is_some_and(|v| spec.choices.iter().any(|c| c == v)))
        {
            return Err(Error::invalid(format!(
                "参数 {key} 的类型、范围或枚举值无效"
            )));
        }
        if matches!(key.as_str(), "stop" | "openrouter.transforms")
            && value
                .as_array()
                .is_some_and(|a| a.len() > 16 || a.iter().any(|v| !v.is_string()))
        {
            return Err(Error::invalid(format!("{key} 须为最多 16 项的字符串数组")));
        }
        if key == "logit_bias"
            && value.as_object().is_some_and(|a| {
                a.iter().any(|(k, v)| {
                    k.parse::<u32>().is_err()
                        || !v.as_f64().is_some_and(|n| (-100.0..=100.0).contains(&n))
                })
            })
        {
            return Err(Error::invalid(
                "logit_bias 须为 Token ID 到 -100–100 的映射",
            ));
        }
        if key == "response_format" {
            match value.get("type").and_then(|v| v.as_str()) {
                Some("text" | "json_object") => {}
                Some("json_schema")
                    if value
                        .pointer("/json_schema/schema")
                        .is_some_and(|v| v.is_object())
                        && value
                            .pointer("/json_schema/name")
                            .is_some_and(|v| v.is_string()) => {}
                _ => {
                    return Err(Error::invalid(
                        "response_format 须包含有效 type；json_schema 须包含 name 与 schema 对象",
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn resolve_parameters(
    layers: &[&LlmParameters],
    specs: &[LlmParameterSpec],
) -> Result<(LlmParameters, Vec<String>)> {
    let mut result = BTreeMap::new();
    for layer in layers {
        validate_parameters(
            layer,
            &specs
                .iter()
                .cloned()
                .map(|mut s| {
                    s.support = LlmSupport::Unknown;
                    s
                })
                .collect::<Vec<_>>(),
        )?;
        for (key, value) in *layer {
            if value.is_null() {
                result.remove(key);
            } else {
                result.insert(key.clone(), value.clone());
            }
        }
    }
    validate_parameters(&result, specs)?;
    if result.contains_key("top_logprobs")
        && result.get("logprobs") != Some(&serde_json::Value::Bool(true))
    {
        return Err(Error::invalid("top_logprobs 需要 logprobs=true"));
    }
    if result.contains_key("reasoning_budget")
        && (result.contains_key("reasoning_effort") || result.contains_key("gemini.thinking_level"))
    {
        return Err(Error::invalid("推理预算与推理等级不可同时设置"));
    }
    let warnings = specs
        .iter()
        .filter(|s| s.support == LlmSupport::Unknown && result.contains_key(&s.key))
        .map(|s| format!("{}：模型能力未知，将按配置发送", s.key))
        .collect();
    Ok((result, warnings))
}
