use studio_domain::{Error, Result, llm::*};

pub(super) fn name(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 240 || value.chars().any(char::is_control) {
        return Err(Error::invalid("名称须为 1–240 字节且不包含控制字符"));
    }
    Ok(())
}
pub(super) fn compatible(kind: LlmProviderKind, protocol: LlmProtocol) -> Result<()> {
    let allowed = match kind {
        LlmProviderKind::Gemini => protocol == LlmProtocol::Gemini,
        LlmProviderKind::Openrouter => protocol == LlmProtocol::OpenaiChat,
        _ => protocol != LlmProtocol::Gemini,
    };
    if !allowed {
        return Err(Error::invalid("该连接类型不支持所选调用协议"));
    }
    Ok(())
}
pub(super) fn request(value: &LlmInvocationRequest) -> Result<()> {
    studio_domain::validate_id(&value.invocation_id)?;
    if value.messages.is_empty() || value.messages.len() > 256 || value.tools.len() > 128 {
        return Err(Error::invalid("消息须为 1–256 条，工具最多 128 个"));
    }
    let mut bytes = 0;
    for message in &value.messages {
        if message.content.is_empty() || message.content.len() > 64 {
            return Err(Error::invalid("每条消息须包含 1–64 个内容块"));
        }
        for content in &message.content {
            bytes += serde_json::to_vec(content).map_err(Error::io)?.len();
            let valid = match content {
                LlmContent::Text { .. } => message.role != LlmRole::Tool,
                LlmContent::Image { url, detail } => {
                    if !(url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with("data:image/")
                        || url.starts_with("gs://"))
                    {
                        return Err(Error::invalid(
                            "图片须为 URL 或带 MIME 类型的 base64 data URL",
                        ));
                    }
                    if detail
                        .as_deref()
                        .is_some_and(|d| !["auto", "low", "high", "original"].contains(&d))
                    {
                        return Err(Error::invalid("图片 detail 无效"));
                    }
                    message.role == LlmRole::User
                }
                LlmContent::ToolCall {
                    id,
                    name,
                    arguments,
                    ..
                } => {
                    !id.is_empty()
                        && !name.is_empty()
                        && arguments.is_object()
                        && message.role == LlmRole::Assistant
                }
                LlmContent::ToolResult { id, name, .. } => {
                    !id.is_empty() && !name.is_empty() && message.role == LlmRole::Tool
                }
                LlmContent::Reasoning { .. } | LlmContent::Refusal { .. } => false,
            };
            if !valid {
                return Err(Error::invalid(
                    "消息角色与内容块不匹配；推理及拒绝内容仅作为输出",
                ));
            }
        }
    }
    bytes += serde_json::to_vec(&value.tools).map_err(Error::io)?.len();
    if bytes > 16 * 1024 * 1024 {
        return Err(Error::invalid("调用输入最多 16 MiB"));
    }
    let mut names = std::collections::BTreeSet::new();
    for tool in &value.tools {
        if tool.name.is_empty()
            || tool.name.len() > 64
            || !tool
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            || !names.insert(&tool.name)
            || !tool.parameters.is_object()
        {
            return Err(Error::invalid("工具名称或参数 schema 无效"));
        }
    }
    Ok(())
}
