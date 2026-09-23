use super::{validation, *};
use std::sync::Arc;
use studio_domain::{Error, Result, llm::*, new_id};

pub struct LlmService {
    pub repository: Arc<dyn LlmRepository>,
    backend: Arc<dyn LlmBackend>,
    credentials: Arc<dyn LlmCredentials>,
}
impl LlmService {
    pub fn generate_recorded(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
        sink: Arc<dyn LlmReceiptSink>,
    ) -> futures::future::BoxFuture<'_, LlmCallResult<LlmResponse>> {
        self.backend.generate_recorded(plan, cancel, sink)
    }
    pub fn reparse(
        &self,
        plan: &LlmInvocationSnapshot,
        receipt: &LlmRawReceipt,
    ) -> LlmCallResult<LlmResponse> {
        self.backend.reparse(plan, receipt)
    }
    pub fn preview(&self, plan: &LlmInvocationPlan) -> Result<serde_json::Value> {
        self.backend.preview(plan)
    }
    pub fn discover(
        &self,
        provider: LlmProvider,
        cancel: LlmCancellation,
    ) -> futures::future::BoxFuture<'_, LlmCallResult<LlmCatalog>> {
        self.backend.discover(provider, cancel)
    }
    pub fn generate(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
    ) -> futures::future::BoxFuture<'_, LlmCallResult<LlmResponse>> {
        self.backend.generate(plan, cancel)
    }
    pub fn stream(
        &self,
        plan: LlmInvocationPlan,
        cancel: LlmCancellation,
    ) -> futures::stream::BoxStream<'_, LlmEvent> {
        self.backend.stream(plan, cancel)
    }
    pub fn new(
        repository: Arc<dyn LlmRepository>,
        backend: Arc<dyn LlmBackend>,
        credentials: Arc<dyn LlmCredentials>,
    ) -> Self {
        Self {
            repository,
            backend,
            credentials,
        }
    }
    pub fn save_provider(
        &self,
        id: Option<String>,
        expected: u64,
        config: LlmConnectionConfig,
        secret: Option<LlmSecret>,
        clear: bool,
    ) -> Result<LlmProvider> {
        validation::name(&config.name)?;
        self.backend.validate_connection(&config)?;
        if secret.is_some() && clear {
            return Err(Error::invalid("设置凭据与清除凭据不可同时执行"));
        }
        let previous = id
            .as_ref()
            .map(|id| self.repository.provider(id))
            .transpose()?;
        if previous.as_ref().map_or(0, |p| p.revision) != expected {
            return Err(Error::new("REVISION_CONFLICT", "连接已被修改，请重新载入"));
        }
        if let Some(previous) = &previous {
            for model in self.repository.models(&previous.id)? {
                validation::compatible(config.kind, model.config.protocol)?;
            }
        }
        let old_ref = previous.as_ref().and_then(|p| p.credential_ref.clone());
        let new_ref = secret.map(|key| self.credentials.put(key)).transpose()?;
        let credential_ref = if clear {
            None
        } else {
            new_ref.clone().or(old_ref.clone())
        };
        let result = self.repository.save_provider(
            LlmProvider {
                id: id.unwrap_or_else(new_id),
                revision: expected + 1,
                config,
                credential_ref,
            },
            expected,
        );
        match result {
            Ok(value) => {
                if (clear || new_ref.is_some())
                    && let Some(reference) = old_ref
                {
                    // The committed connection is authoritative. Old refs are no longer usable by it.
                    let _ = self.credentials.remove(&reference);
                }
                Ok(value)
            }
            Err(error) => {
                if let Some(reference) = new_ref {
                    let _ = self.credentials.remove(&reference);
                }
                Err(error)
            }
        }
    }
    pub fn remove_provider(&self, id: &str, expected: u64) -> Result<()> {
        let previous = self.repository.provider(id)?;
        self.repository.remove_provider(id, expected)?;
        if let Some(reference) = previous.credential_ref {
            self.credentials.remove(&reference)?;
        }
        Ok(())
    }
    pub fn save_model(
        &self,
        id: Option<String>,
        provider_id: String,
        expected: u64,
        config: LlmModelConfig,
    ) -> Result<LlmModel> {
        if expected >= i64::MAX as u64 {
            return Err(Error::invalid("配置版本超出范围"));
        }
        validation::name(&config.name)?;
        if config.remote_model_id.is_empty()
            || config.remote_model_id.len() > 256
            || config.remote_model_id.chars().any(char::is_control)
        {
            return Err(Error::invalid("远端模型 ID 无效"));
        }
        let provider = self.repository.provider(&provider_id)?;
        validation::compatible(provider.config.kind, config.protocol)?;
        let specs = parameter_specs(config.protocol, provider.config.kind);
        validate_parameters(&config.parameters, &specs)?;
        if config.capability_overrides.len() > 128
            || config.capability_overrides.keys().any(|k| {
                !["input_image", "tools"].contains(&k.as_str())
                    && !specs.iter().any(|s| &s.key == k)
            })
        {
            return Err(Error::invalid("能力覆盖包含协议未定义的能力"));
        }
        self.repository.save_model(
            LlmModel {
                id: id.unwrap_or_else(new_id),
                provider_id,
                revision: expected + 1,
                config,
            },
            expected,
        )
    }
    pub fn save_preset(
        &self,
        id: Option<String>,
        expected: u64,
        config: LlmPresetConfig,
    ) -> Result<LlmPreset> {
        if expected >= i64::MAX as u64 {
            return Err(Error::invalid("配置版本超出范围"));
        }
        validation::name(&config.name)?;
        let kind = if config.protocol == LlmProtocol::Gemini {
            LlmProviderKind::Gemini
        } else {
            LlmProviderKind::Openrouter
        };
        validate_parameters(&config.parameters, &parameter_specs(config.protocol, kind))?;
        self.repository.save_preset(
            LlmPreset {
                id: id.unwrap_or_else(new_id),
                revision: expected + 1,
                config,
            },
            expected,
        )
    }
    pub fn specs(&self, model: &LlmModel, provider: &LlmProvider) -> Result<Vec<LlmParameterSpec>> {
        let mut specs = parameter_specs(model.config.protocol, provider.config.kind);
        let catalog = self.repository.catalog(&provider.id)?;
        let entry = catalog
            .as_ref()
            .filter(|c| c.provider_revision == provider.revision)
            .and_then(|c| {
                c.models
                    .iter()
                    .find(|m| m.id == model.config.remote_model_id)
            });
        for spec in &mut specs {
            if let Some(state) = entry.and_then(|e| e.capabilities.get(&spec.key)) {
                spec.support = *state;
                spec.evidence = "remote_catalog".into();
            }
            if let Some(state) = model.config.capability_overrides.get(&spec.key) {
                spec.support = *state;
                spec.evidence = "user_override".into();
            }
        }
        Ok(specs)
    }
    pub fn prepare(&self, mut request: LlmInvocationRequest) -> Result<LlmInvocationPlan> {
        validation::request(&request)?;
        let system_prompt = self.resolve_system_prompt(&mut request)?;
        let model = self.repository.model(&request.model_id)?;
        let provider = self.repository.provider(&model.provider_id)?;
        if !model.config.enabled || !provider.config.enabled {
            return Err(Error::new("LLM_DISABLED", "模型或供应商连接已停用"));
        }
        self.backend.validate_connection(&provider.config)?;
        validation::compatible(provider.config.kind, model.config.protocol)?;
        if request
            .expected_model_revision
            .is_some_and(|r| r != model.revision)
            || request
                .expected_provider_revision
                .is_some_and(|r| r != provider.revision)
        {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "模型或连接已更新，请重新准备调用",
            ));
        }
        let preset = request
            .preset_id
            .as_ref()
            .map(|id| self.repository.preset(id))
            .transpose()?;
        if preset
            .as_ref()
            .is_some_and(|p| p.config.protocol != model.config.protocol)
        {
            return Err(Error::invalid("预设与模型协议不一致"));
        }
        if request
            .expected_preset_revision
            .is_some_and(|r| preset.as_ref().is_none_or(|p| r != p.revision))
        {
            return Err(Error::new("REVISION_CONFLICT", "参数预设已更新"));
        }
        let specs = self.specs(&model, &provider)?;
        let empty = LlmParameters::new();
        let mut defaults = LlmParameters::new();
        if provider.config.kind == LlmProviderKind::Openai {
            defaults.insert("store".into(), serde_json::Value::Bool(false));
        }
        let (parameters, mut warnings) = resolve_parameters(
            &[
                &defaults,
                &model.config.parameters,
                preset.as_ref().map_or(&empty, |p| &p.config.parameters),
                &request.overrides,
            ],
            &specs,
        )?;
        let has_images = request
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .any(|c| matches!(c, LlmContent::Image { .. }));
        let catalog = self.repository.catalog(&provider.id)?;
        let catalog_model = catalog
            .as_ref()
            .filter(|c| c.provider_revision == provider.revision)
            .and_then(|c| {
                c.models
                    .iter()
                    .find(|m| m.id == model.config.remote_model_id)
            });
        for (key, present) in [
            ("input_image", has_images),
            ("tools", !request.tools.is_empty()),
        ] {
            if present {
                match model
                    .config
                    .capability_overrides
                    .get(key)
                    .or_else(|| catalog_model.and_then(|m| m.capabilities.get(key)))
                {
                    Some(LlmSupport::Unsupported) => {
                        return Err(Error::invalid(format!("模型明确不支持 {key}")));
                    }
                    Some(LlmSupport::Supported) => {}
                    _ => warnings.push(format!("{key}：未验证模型能力")),
                }
            }
        }
        if parameters.get("tool_choice").is_some_and(|v| v != "none") && request.tools.is_empty() {
            return Err(Error::invalid("工具选择需要工具声明"));
        }
        let snapshot = LlmInvocationSnapshot {
            schema_version: LLM_SCHEMA_VERSION,
            invocation_id: request.invocation_id,
            provider_id: provider.id.clone(),
            provider_revision: provider.revision,
            provider_kind: provider.config.kind,
            base_url: provider.config.base_url.clone(),
            model_id: model.id,
            model_revision: model.revision,
            remote_model_id: model.config.remote_model_id,
            protocol: model.config.protocol,
            preset_id: preset.as_ref().map(|p| p.id.clone()),
            preset_revision: preset.as_ref().map(|p| p.revision),
            system_prompt_id: system_prompt.as_ref().map(|p| p.id.clone()),
            system_prompt_revision: system_prompt.as_ref().map(|p| p.revision),
            parameters,
            messages: request.messages,
            tools: request.tools,
            warnings,
        };
        let plan = LlmInvocationPlan { provider, snapshot };
        self.backend.preview(&plan)?;
        Ok(plan)
    }
}
