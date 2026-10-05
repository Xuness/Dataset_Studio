use super::*;
use studio_application::aesthetic::{same_execution_standard, validate_execution_policy};
pub fn check_execution(state: &AppState, stage: &AestheticStage) -> Result<()> {
    prepare(
        state,
        stage,
        stage.config.model.messages.clone(),
        studio_domain::new_id(),
        false,
    )
    .map(|_| ())
    .map_err(|error| {
        if error.code == "REVISION_CONFLICT" {
            Error::new(
                "REVISION_CONFLICT",
                "连接或模型版本已变化；请在本阶段的执行设置中重新关联当前连接，原评审证据会保留",
            )
        } else {
            error
        }
    })
}

pub(super) fn prepare(
    state: &AppState,
    stage: &AestheticStage,
    messages: Vec<LlmMessage>,
    invocation_id: String,
    rebind: bool,
) -> Result<LlmInvocationPlan> {
    let expected = stage
        .execution_settings
        .as_ref()
        .map(|s| (s.model_revision, s.provider_revision))
        .unwrap_or((
            stage.config.model.model_revision,
            stage.config.model.provider_revision,
        ));
    // Explicitly omit newly added model defaults when replaying a frozen stage.
    let model = state.llm.repository.model(&stage.config.model.model_id)?;
    let mut overrides: LlmParameters = model
        .config
        .parameters
        .keys()
        .map(|key| (key.clone(), serde_json::Value::Null))
        .collect();
    overrides.extend(stage.config.model.parameters.clone());
    let plan = state.llm.prepare(LlmInvocationRequest {
        invocation_id,
        model_id: stage.config.model.model_id.clone(),
        expected_model_revision: (!rebind).then_some(expected.0),
        expected_provider_revision: (!rebind).then_some(expected.1),
        preset_id: None,
        expected_preset_revision: None,
        system_prompt_id: None,
        expected_system_prompt_revision: None,
        overrides,
        messages: messages.clone(),
        tools: stage.config.model.tools.clone(),
    })?;
    let mut old = stage.config.model.clone();
    old.messages = messages;
    same_execution_standard(&old, &plan.snapshot)?;
    Ok(plan)
}

pub fn configure_execution(
    state: &AppState,
    pid: &str,
    id: &str,
    value: AestheticExecutionUpdate,
) -> Result<AestheticStage> {
    validate_execution_policy(&value.policy)?;
    let _lease = state.store.operation_lease(pid)?;
    let db = state.store.evaluation(pid)?;
    if db.execution_update_applied(id, &value)? {
        return db.stage(id);
    }
    let stage = db.stage(id)?;
    let plan = prepare(
        state,
        &stage,
        stage.config.model.messages.clone(),
        studio_domain::new_id(),
        true,
    )?;
    let options = studio_application::aesthetic::recorded_options(&value.policy);
    state.llm.preview_recorded(&plan, &options)?;
    db.configure_execution(
        id,
        value,
        plan.snapshot.provider_revision,
        plan.snapshot.model_revision,
    )
}
