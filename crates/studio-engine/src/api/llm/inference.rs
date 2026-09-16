use super::*;
use futures::StreamExt;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoverLlmModels {
    invocation_id: String,
    expected_revision: u64,
}
#[utoipa::path(operation_id="llm_refresh_catalog",post,path="/providers/{id}/catalog/refresh",params(("id"=String,Path)),request_body=DiscoverLlmModels,responses((status=200,body=LlmCatalog),(status=502,body=LlmFailure)))]
pub(crate) async fn refresh_catalog(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(v): Body<DiscoverLlmModels>,
) -> Response {
    let result = async {
        let guard = s
            .llm_invocations
            .register(&v.invocation_id)
            .map_err(|e| Failure(e).into_response())?;
        let service = s.llm.clone();
        let provider = blocking(move || {
            let provider = service.repository.provider(&id)?;
            if provider.revision != v.expected_revision {
                return Err(domain::Error::new("REVISION_CONFLICT", "连接配置已更新"));
            }
            Ok(provider)
        })
        .await
        .map_err(IntoResponse::into_response)?;
        let catalog = s
            .llm
            .discover(provider, guard.cancel.clone())
            .await
            .map_err(|e| RemoteFailure(e).into_response())?;
        let copy = catalog.clone();
        blocking(move || s.llm.repository.save_catalog(&copy))
            .await
            .map_err(IntoResponse::into_response)?;
        Ok::<_, Response>(Json(LlmCatalog::from(catalog)).into_response())
    }
    .await;
    result.unwrap_or_else(|r| r)
}
#[utoipa::path(operation_id="llm_prepare",post,path="/prepare",request_body=LlmInvocationRequest,responses((status=200,body=LlmPrepared)))]
pub(crate) async fn prepare(
    State(s): State<AppState>,
    Body(v): Body<LlmInvocationRequest>,
) -> ApiResult<LlmPrepared> {
    Ok(Json(
        blocking(move || {
            let plan = s.llm.prepare(v.into())?;
            Ok(LlmPrepared {
                native_request: s.llm.preview(&plan)?,
                snapshot: plan.snapshot.into(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(operation_id="llm_generate",post,path="/generate",request_body=LlmInvocationRequest,responses((status=200,body=LlmResponse),(status=502,body=LlmFailure)))]
pub(crate) async fn generate(
    State(s): State<AppState>,
    Body(v): Body<LlmInvocationRequest>,
) -> Response {
    let guard = match s.llm_invocations.register(&v.invocation_id) {
        Ok(g) => g,
        Err(e) => return Failure(e).into_response(),
    };
    let service = s.llm.clone();
    let plan = match blocking(move || service.prepare(v.into())).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    match s.llm.generate(plan, guard.cancel.clone()).await {
        Ok(value) => Json(LlmResponse::from(value)).into_response(),
        Err(error) => RemoteFailure(error).into_response(),
    }
}
#[utoipa::path(operation_id="llm_stream",post,path="/stream",request_body=LlmInvocationRequest,responses((status=200,description="SSE data contains LlmEvent; completion or failure is required",content_type="text/event-stream",body=LlmEvent)))]
pub(crate) async fn stream(
    State(s): State<AppState>,
    Body(v): Body<LlmInvocationRequest>,
) -> Response {
    let guard = match s.llm_invocations.register(&v.invocation_id) {
        Ok(g) => g,
        Err(e) => return Failure(e).into_response(),
    };
    let service = s.llm.clone();
    let plan = match blocking(move || service.prepare(v.into())).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let stream = async_stream::stream! {
        let _guard = guard;
        let mut events = s.llm.stream(plan,_guard.cancel.clone());
        while let Some(value) = events.next().await {
            let value = LlmEvent::from(value);
            yield Ok::<_,Infallible>(Event::default().json_data(value).expect("serializable LLM event"));
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
#[utoipa::path(operation_id="llm_cancel",post,path="/invocations/{id}/cancel",params(("id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(crate) async fn cancel(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<OkResponse> {
    s.llm_invocations.cancel(&id)?;
    Ok(Json(OkResponse { ok: true }))
}
