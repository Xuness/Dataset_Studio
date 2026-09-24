use super::*;
use sha2::{Digest, Sha256};
use studio_application::aesthetic::{
    capabilities, execution, same_request, validate_capacity, validate_create,
};

fn input(
    state: &AppState,
    pid: &str,
    request: &AestheticCreate,
) -> Result<(u64, String, Vec<studio_domain::QuerySourceVersion>, String)> {
    let (total, project_version) = state.store.evaluation_input(pid, &request.collection_id)?;
    let scope = studio_domain::ScopeRef {
        project_id: pid.into(),
        target: studio_domain::ScopeTarget::Workset {
            collection_id: request.collection_id.clone(),
        },
    };
    let mut sources = Vec::new();
    for id in state.store.scope_source_ids(pid, &scope)? {
        let source = state.store.source(pid, &id)?;
        let spec = studio_domain::QuerySpec {
            version: 1,
            source_ids: vec![id],
            conditions: if source.kind == "danbooru" {
                vec![studio_domain::QueryCondition {
                    field: "rating".into(),
                    operator: studio_domain::QueryOperator::IsPresent,
                    value: None,
                }]
            } else {
                vec![]
            },
            observation_rule: studio_domain::ObservationRule::AnyObservation,
            order: studio_domain::QueryOrder::AssetKeyAsc,
            input_scope: None,
        };
        sources.push(studio_sources::QueryReader::default().query_version(&source, &spec)?);
    }
    sources.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let input_version = hex::encode(Sha256::digest(
        serde_json::to_vec(&(&project_version, &sources)).map_err(Error::io)?,
    ));
    Ok((total, project_version, sources, input_version))
}

pub(super) const MIN_STORAGE_HEADROOM: u64 = 256 << 20;
pub(super) fn check_storage(directory: &std::path::Path) -> Result<u64> {
    let available = fs2::available_space(directory).map_err(Error::io)?;
    if available < MIN_STORAGE_HEADROOM {
        return Err(Error::new(
            "EVALUATION_STORAGE_FULL",
            "评审存储可用空间不足 256 MiB，尚未派发",
        ));
    }
    Ok(available)
}

pub fn preflight(
    state: &AppState,
    pid: &str,
    request: AestheticCreate,
) -> Result<AestheticPreflight> {
    validate_create(&request)?;
    let (total, _, _, input_version) = input(state, pid, &request)?;
    let available_storage_bytes =
        fs2::available_space(state.store.directory(pid)?).map_err(Error::io)?;
    let rejection = validate_capacity(total)
        .and_then(|_| {
            request.sampling.as_ref().map_or(Ok(()), |p| {
                studio_application::aesthetic::sampling::validate(p, total)
            })
        })
        .and_then(|_| check_storage(&state.store.directory(pid)?).map(|_| ()))
        .err();
    Ok(AestheticPreflight {
        capabilities: capabilities(),
        available_storage_bytes,
        minimum_calls_lower_bound: total
            .saturating_mul(u64::from(request.exposures))
            .div_ceil(16),
        total,
        input_version,
        admitted: rejection.is_none(),
        rejection_code: rejection.as_ref().map(|e| e.code.into()),
        rejection_reason: rejection.map(|e| e.message),
    })
}

pub fn create(state: &AppState, pid: &str, request: AestheticCreate) -> Result<AestheticStage> {
    validate_create(&request)?;
    let db = state.store.evaluation(pid)?;
    if let Some(intent) = state
        .store
        .evaluation_intent(pid, &request.idempotency_key)?
    {
        same_request(&intent.config.request, &request)?;
        if matches!(intent.state.as_str(), "abandoned" | "cancelled") {
            return Err(Error::new(
                "OBJECT_REMOVED",
                "创建意图已明确放弃，请使用新创建键",
            ));
        }
        if intent.state == "materialized"
            && let Err(error) = db.stage(&request.idempotency_key)
        {
            return Err(if error.code == "NOT_FOUND" {
                Error::new("EVALUATION_MISSING", "已建成阶段缺少账本记录，请恢复备份")
            } else {
                error
            });
        }
        return materialize(state, pid, &db, intent);
    }
    match db.stage(&request.idempotency_key) {
        Ok(stage) => {
            same_request(&stage.config.request, &request)?;
            state.store.sync_evaluation(pid, &stage)?;
            if stage.state == "cancelled" {
                return Err(Error::new(
                    "OBJECT_REMOVED",
                    "评审阶段已取消，请使用新创建键",
                ));
            }
            return Ok(stage);
        }
        Err(e) if e.code == "NOT_FOUND" => {}
        Err(e) => return Err(e),
    }
    let (total, project_version, sources, input_version) = input(state, pid, &request)?;
    validate_capacity(total)?;
    if let Some(policy) = &request.sampling {
        studio_application::aesthetic::sampling::validate(policy, total)?;
    }
    check_storage(&state.store.directory(pid)?)?;
    if request
        .expected_input_version
        .as_ref()
        .is_some_and(|v| v != &input_version)
    {
        return Err(Error::new(
            "EVALUATION_INPUT_CHANGED",
            "预检后输入版本已变化，请重新预检",
        ));
    }
    let plan = state.llm.prepare(LlmInvocationRequest {
        invocation_id: request.idempotency_key.clone(),
        model_id: request.model_id.clone(),
        expected_model_revision: None,
        expected_provider_revision: None,
        preset_id: None,
        expected_preset_revision: None,
        system_prompt_id: Some(request.system_prompt_id.clone()),
        expected_system_prompt_revision: None,
        overrides: request.overrides.clone(),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: vec![LlmContent::Text {
                text: studio_application::aesthetic::OUTPUT_INSTRUCTIONS.into(),
            }],
        }],
        tools: vec![],
    })?;
    let caps = capabilities();
    let max_request_bytes = u64::from(request.max_request_mib.unwrap_or(32)) << 20;
    let mut frozen_execution = execution(input_version);
    if let Some(policy) = &request.sampling {
        frozen_execution.sampler_version =
            studio_application::aesthetic::sampling::version(policy).into();
    }
    let intent = state.store.register_evaluation(
        pid,
        AestheticConfig {
            version: 2,
            request,
            model: plan.snapshot,
            sources,
            image_policy: "stored_original_v1".into(),
            grouping_policy: "origin_rating_agreement_min_post_created_year_v1".into(),
            observation_policy: "meaningful_indifference_v1".into(),
            max_image_bytes: caps.max_image_bytes,
            max_request_bytes,
            execution: Some(frozen_execution),
        },
        &project_version,
    )?;
    materialize(state, pid, &db, intent)
}

fn materialize(
    state: &AppState,
    pid: &str,
    _db: &EvaluationDb,
    intent: AestheticCreationIntent,
) -> Result<AestheticStage> {
    state
        .store
        .materialize_evaluation(pid, &intent.config.request.idempotency_key)
}
