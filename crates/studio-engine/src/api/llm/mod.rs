use super::*;
use studio_protocol::llm::*;
mod configuration;
mod inference;
pub(super) use configuration::*;
pub(super) use inference::*;

#[derive(OpenApi)]
#[openapi(
    paths(
        providers,
        save_provider,
        remove_provider,
        models,
        save_model,
        remove_model,
        presets,
        save_preset,
        remove_preset,
        parameters,
        model_parameters,
        catalog,
        refresh_catalog,
        prepare,
        generate,
        stream,
        cancel
    ),
    components(schemas(LlmEvent, LlmFailure))
)]
pub struct LlmApiDoc;

pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/providers", get(providers).post(save_provider))
        .route("/providers/{id}/remove", post(remove_provider))
        .route("/providers/{id}/models", get(models))
        .route("/providers/{id}/catalog", get(catalog))
        .route("/providers/{id}/catalog/refresh", post(refresh_catalog))
        .route("/models", post(save_model))
        .route("/models/{id}/remove", post(remove_model))
        .route("/models/{id}/parameters", get(model_parameters))
        .route("/presets", get(presets).post(save_preset))
        .route("/presets/{id}/remove", post(remove_preset))
        .route("/parameters", get(parameters))
        .route("/prepare", post(prepare))
        .route("/generate", post(generate))
        .route("/stream", post(stream))
        .route("/invocations/{id}/cancel", post(cancel))
        .layer(axum::extract::DefaultBodyLimit::max(24 * 1024 * 1024))
}

pub struct RemoteFailure(pub domain::llm::LlmFailure);
impl From<domain::llm::LlmFailure> for RemoteFailure {
    fn from(v: domain::llm::LlmFailure) -> Self {
        Self(v)
    }
}
impl IntoResponse for RemoteFailure {
    fn into_response(self) -> Response {
        let status = match self.0.code.as_str() {
            "LLM_RATE_LIMITED" | "LLM_QUEUE_FULL" => StatusCode::TOO_MANY_REQUESTS,
            "LLM_CANCELLED" => StatusCode::CONFLICT,
            "LLM_TIMEOUT" => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::BAD_GATEWAY,
        };
        (status, Json(LlmFailure::from(self.0))).into_response()
    }
}
