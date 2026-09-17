use super::{LlmService, validation};
use studio_domain::{Error, Result, llm::*, new_id};

impl LlmService {
    pub fn save_system_prompt(
        &self,
        id: Option<String>,
        expected: u64,
        config: LlmSystemPromptConfig,
    ) -> Result<LlmSystemPrompt> {
        if expected >= i64::MAX as u64 {
            return Err(Error::invalid("配置版本超出范围"));
        }
        validate_config(&config)?;
        self.repository.save_system_prompt(
            LlmSystemPrompt {
                id: id.unwrap_or_else(new_id),
                revision: expected + 1,
                config,
            },
            expected,
        )
    }

    pub(super) fn resolve_system_prompt(
        &self,
        request: &mut LlmInvocationRequest,
    ) -> Result<Option<LlmSystemPrompt>> {
        let Some(id) = &request.system_prompt_id else {
            if request.expected_system_prompt_revision.is_some() {
                return Err(Error::invalid("System Prompt 版本校验需要预设 ID"));
            }
            return Ok(None);
        };
        if request
            .messages
            .iter()
            .any(|m| matches!(m.role, LlmRole::System | LlmRole::Developer))
        {
            return Err(Error::invalid(
                "选用 System Prompt 预设时，不可同时传入 system 或 developer 消息",
            ));
        }
        let prompt = self.repository.system_prompt(id)?;
        if request
            .expected_system_prompt_revision
            .is_some_and(|r| r != prompt.revision)
        {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "System Prompt 预设已更新，请重新准备调用",
            ));
        }
        validate_config(&prompt.config)?;
        request.messages.insert(
            0,
            LlmMessage {
                role: LlmRole::System,
                content: vec![LlmContent::Text {
                    text: prompt.config.text.clone(),
                }],
            },
        );
        // The resolved prompt participates in the same message count and input size limits.
        validation::request(request)?;
        Ok(Some(prompt))
    }
}

fn validate_config(config: &LlmSystemPromptConfig) -> Result<()> {
    validation::name(&config.name)?;
    if config.description.len() > 2000 {
        return Err(Error::invalid("预设备注最多 2000 字节"));
    }
    if config.text.trim().is_empty() || config.text.len() > 64 * 1024 {
        return Err(Error::invalid(
            "System Prompt 正文不能为空，且最多 64 KiB（UTF-8）",
        ));
    }
    Ok(())
}
