use super::*;
use studio_application::llm::{LlmSecret, parameter_specs};

#[utoipa::path(operation_id="llm_providers",get,path="/providers",responses((status=200,body=LlmProviders)))]
pub(crate) async fn providers(State(s): State<AppState>) -> ApiResult<LlmProviders> {
    Ok(Json(LlmProviders {
        items: blocking(move || s.llm.repository.providers())
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(operation_id="llm_save_provider",post,path="/providers",request_body=SaveLlmProvider,responses((status=200,body=LlmProviderView)))]
pub(crate) async fn save_provider(
    State(s): State<AppState>,
    Body(v): Body<SaveLlmProvider>,
) -> ApiResult<LlmProviderView> {
    Ok(Json(
        blocking(move || {
            s.llm.save_provider(
                v.id,
                v.expected_revision,
                v.config.into(),
                v.api_key.map(LlmSecret::new),
                v.clear_credential,
            )
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(operation_id="llm_remove_provider",post,path="/providers/{id}/remove",params(("id"=String,Path)),request_body=LlmRevision,responses((status=200,body=OkResponse)))]
pub(crate) async fn remove_provider(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(v): Body<LlmRevision>,
) -> ApiResult<OkResponse> {
    blocking(move || s.llm.remove_provider(&id, v.expected_revision)).await?;
    Ok(Json(OkResponse { ok: true }))
}
#[utoipa::path(operation_id="llm_models",get,path="/providers/{id}/models",params(("id"=String,Path)),responses((status=200,body=LlmModels)))]
pub(crate) async fn models(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LlmModels> {
    Ok(Json(LlmModels {
        items: blocking(move || s.llm.repository.models(&id))
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(operation_id="llm_save_model",post,path="/models",request_body=SaveLlmModel,responses((status=200,body=LlmModel)))]
pub(crate) async fn save_model(
    State(s): State<AppState>,
    Body(v): Body<SaveLlmModel>,
) -> ApiResult<LlmModel> {
    Ok(Json(
        blocking(move || {
            s.llm
                .save_model(v.id, v.provider_id, v.expected_revision, v.config.into())
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(operation_id="llm_remove_model",post,path="/models/{id}/remove",params(("id"=String,Path)),request_body=LlmRevision,responses((status=200,body=OkResponse)))]
pub(crate) async fn remove_model(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(v): Body<LlmRevision>,
) -> ApiResult<OkResponse> {
    blocking(move || s.llm.repository.remove_model(&id, v.expected_revision)).await?;
    Ok(Json(OkResponse { ok: true }))
}
#[utoipa::path(operation_id="llm_presets",get,path="/presets",responses((status=200,body=LlmPresets)))]
pub(crate) async fn presets(State(s): State<AppState>) -> ApiResult<LlmPresets> {
    Ok(Json(LlmPresets {
        items: blocking(move || s.llm.repository.presets())
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(operation_id="llm_save_preset",post,path="/presets",request_body=SaveLlmPreset,responses((status=200,body=LlmPreset)))]
pub(crate) async fn save_preset(
    State(s): State<AppState>,
    Body(v): Body<SaveLlmPreset>,
) -> ApiResult<LlmPreset> {
    Ok(Json(
        blocking(move || {
            s.llm
                .save_preset(v.id, v.expected_revision, v.config.into())
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(operation_id="llm_remove_preset",post,path="/presets/{id}/remove",params(("id"=String,Path)),request_body=LlmRevision,responses((status=200,body=OkResponse)))]
pub(crate) async fn remove_preset(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(v): Body<LlmRevision>,
) -> ApiResult<OkResponse> {
    blocking(move || s.llm.repository.remove_preset(&id, v.expected_revision)).await?;
    Ok(Json(OkResponse { ok: true }))
}
#[derive(Deserialize, utoipa::IntoParams)]
pub(crate) struct ParametersQuery {
    protocol: LlmProtocol,
    kind: LlmProviderKind,
}
#[utoipa::path(operation_id="llm_parameters",get,path="/parameters",params(ParametersQuery),responses((status=200,body=LlmParameters)))]
pub(crate) async fn parameters(Query(q): Query<ParametersQuery>) -> Json<LlmParameters> {
    Json(LlmParameters {
        items: parameter_specs(q.protocol.into(), q.kind.into())
            .into_iter()
            .map(Into::into)
            .collect(),
    })
}
#[utoipa::path(operation_id="llm_model_parameters",get,path="/models/{id}/parameters",params(("id"=String,Path)),responses((status=200,body=LlmParameters)))]
pub(crate) async fn model_parameters(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LlmParameters> {
    Ok(Json(LlmParameters {
        items: blocking(move || {
            let model = s.llm.repository.model(&id)?;
            let provider = s.llm.repository.provider(&model.provider_id)?;
            s.llm.specs(&model, &provider)
        })
        .await?
        .into_iter()
        .map(Into::into)
        .collect(),
    }))
}
#[utoipa::path(operation_id="llm_catalog",get,path="/providers/{id}/catalog",params(("id"=String,Path)),responses((status=200,body=LlmCatalogStatus)))]
pub(crate) async fn catalog(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LlmCatalogStatus> {
    Ok(Json(LlmCatalogStatus {
        catalog: blocking(move || s.llm.repository.catalog(&id))
            .await?
            .map(Into::into),
    }))
}
