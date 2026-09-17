use super::*;

#[utoipa::path(operation_id="llm_system_prompts",get,path="/system-prompts",responses((status=200,body=LlmSystemPrompts)))]
pub(crate) async fn system_prompts(State(s): State<AppState>) -> ApiResult<LlmSystemPrompts> {
    Ok(Json(LlmSystemPrompts {
        items: blocking(move || s.llm.repository.system_prompts())
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}

#[utoipa::path(operation_id="llm_system_prompt",get,path="/system-prompts/{id}",params(("id"=String,Path)),responses((status=200,body=LlmSystemPrompt)))]
pub(crate) async fn system_prompt(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LlmSystemPrompt> {
    Ok(Json(
        blocking(move || s.llm.repository.system_prompt(&id))
            .await?
            .into(),
    ))
}

#[utoipa::path(operation_id="llm_save_system_prompt",post,path="/system-prompts",request_body=SaveLlmSystemPrompt,responses((status=200,body=LlmSystemPrompt)))]
pub(crate) async fn save_system_prompt(
    State(s): State<AppState>,
    Body(v): Body<SaveLlmSystemPrompt>,
) -> ApiResult<LlmSystemPrompt> {
    Ok(Json(
        blocking(move || {
            s.llm
                .save_system_prompt(v.id, v.expected_revision, v.config.into())
        })
        .await?
        .into(),
    ))
}

#[utoipa::path(operation_id="llm_remove_system_prompt",post,path="/system-prompts/{id}/remove",params(("id"=String,Path)),request_body=LlmRevision,responses((status=200,body=OkResponse)))]
pub(crate) async fn remove_system_prompt(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(v): Body<LlmRevision>,
) -> ApiResult<OkResponse> {
    blocking(move || {
        s.llm
            .repository
            .remove_system_prompt(&id, v.expected_revision)
    })
    .await?;
    Ok(Json(OkResponse { ok: true }))
}
