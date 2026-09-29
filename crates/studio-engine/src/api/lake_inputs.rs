//! Persisted preparation advances bounded member pages independently of the UI.
use super::*;
use domain::lake_updates::{LakeInputPreparation, LakePreparedInput, LakeUpdateOperation as Op};
use serde_json::json;
fn capture_versions(s: &AppState, row: &LakeInputPreparation, retain: bool) -> domain::Result<()> {
    let read = s.sources.inspect()?;
    let query = read.query(domain::METADATA_MEMORY_BYTES, false);
    for version in &row.versions {
        let source = s.store.source(&row.project_id, &version.source_id)?;
        let identity = format!("lake-input-capture/{}", row.id);
        if retain {
            query.retain_version(&source, version, &identity, &row.project_id, true)?;
        } else {
            query.release_version(&source, &identity)?;
        }
    }
    Ok(())
}
static CREATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
type CaptureSlot = Option<(String, Arc<std::sync::atomic::AtomicBool>)>;
static CAPTURE: std::sync::Mutex<CaptureSlot> = std::sync::Mutex::new(None);
struct CaptureAttempt;
impl Drop for CaptureAttempt {
    fn drop(&mut self) {
        if let Ok(mut slot) = CAPTURE.lock() {
            *slot = None;
        }
    }
}
pub(crate) fn shutdown() {
    if let Ok(slot) = CAPTURE.lock()
        && let Some((_, flag)) = &*slot
    {
        flag.store(true, std::sync::atomic::Ordering::Release);
    }
}

fn dto(row: LakeInputPreparation) -> domain::Result<LakeUpdatePreparation> {
    serde_json::from_value(json!(row)).map_err(domain::Error::io)
}
#[utoipa::path(post,path="/v1/lake-updates/preparations",request_body=PrepareLakeUpdateInputs,responses((status=200,body=LakeUpdatePreparation)),operation_id="lake_updates_prepare_scope")]
pub(super) async fn create(
    State(s): State<AppState>,
    Body(body): Body<PrepareLakeUpdateInputs>,
) -> ApiResult<LakeUpdatePreparation> {
    Ok(Json(
        blocking(move || {
            let _creation = CREATION
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "准备入口锁不可用"))?;
            let scope: domain::ScopeRef = body.scope.into();
            let pid = scope.project_id.clone();
            if !s.lake_updates.configured() {
                return Err(domain::Error::new(
                    "UPDATE_NOT_CONFIGURED",
                    "请先配置数据湖更新服务",
                ));
            }
            domain::validate_id(&body.request_key)?;
            if let Ok(old) = s.store.lake_input_preparation(&body.request_key) {
                if json!(old.scope) != json!(scope) {
                    return Err(domain::Error::new(
                        "IDEMPOTENCY_CONFLICT",
                        "请求标识已用于不同范围",
                    ));
                }
                return dto(old);
            }
            let _lease = s.store.request_lease(&pid)?;
            let label = body
                .label
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| match &scope.target {
                    domain::ScopeTarget::Selection { .. } => "当前选择".into(),
                    domain::ScopeTarget::Workset { .. } => "工作集图片范围".into(),
                    domain::ScopeTarget::QueryResult { .. } => "查询结果图片范围".into(),
                    domain::ScopeTarget::Source { .. } => "数据湖图片范围".into(),
                });
            if label.chars().count() > 512 {
                return Err(domain::Error::invalid("范围名称过长"));
            }
            {
                let read = s.sources.inspect()?;
                query::validate_scope(&s, &pid, &scope, &read)?;
            }
            let source_ids = s.store.scope_source_ids(&pid, &scope)?;
            let registered = s.lake_updates.execute(Op::Lakes, json!({}))?;
            for id in &source_ids {
                if !registered["items"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|r| r["id"].as_str() == Some(id)))
                {
                    return Err(domain::Error::new(
                        "UPDATE_NOT_REGISTERED",
                        "范围内存在尚未登记为可更新湖的来源",
                    ));
                }
            }
            if source_ids.is_empty() {
                return Err(domain::Error::invalid("范围没有可更新来源"));
            }
            // Capture on submission so a later selection edit cannot change membership.
            let (spec, versions) = if query::requires_capture(&s, &pid, &scope)? {
                let read = s.sources.inspect()?;
                query::source_capture(&s, &pid, &scope, &read)?
            } else {
                let spec = domain::QuerySpec {
                    version: 3,
                    source_ids,
                    conditions: vec![],
                    observation_rule: domain::ObservationRule::CurrentPost,
                    order: domain::QueryOrder::AssetKeyAsc,
                    input_scope: Some(scope.clone()),
                };
                let versions = s.queries.versions(&s.store, &pid, &spec)?;
                (spec, versions)
            };
            if versions
                .iter()
                .any(|v| v.consistency != "retained_online_snapshot")
            {
                return Err(domain::Error::new(
                    "UPDATE_UNSUPPORTED",
                    "更新范围准备仅支持在线数据湖",
                ));
            }
            let mut row = LakeInputPreparation {
                id: body.request_key,
                project_id: pid.clone(),
                project_name: s.store.project(&pid)?.name,
                label,
                scope,
                state: "preparing".into(),
                result_id: None,
                after: None,
                processed: 0,
                total: None,
                inputs: vec![],
                error: None,
                versions: versions.clone(),
            };
            s.store.lake_input_create(&row)?;
            let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
            *CAPTURE
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "准备状态锁不可用"))? =
                Some((row.id.clone(), cancelled.clone()));
            let _capture = CaptureAttempt;
            capture_versions(&s, &row, true)?;
            let result = if query::requires_capture(&s, &pid, &row.scope)? {
                let read = s.sources.inspect()?;
                query_views::create_fixed(&s, &pid, spec, versions, &read)?
            } else {
                let result = s
                    .store
                    .capture_lake_members(&pid, &row.scope, spec, versions, &row.id, &cancelled)?;
                let read = s.sources.inspect()?;
                query_views::retain_created(&s, &pid, &result, true, &read)?;
                result
            };
            s.store.lake_input_pin(&pid, &row.id, &result.id, true)?;
            row.result_id = Some(result.id);
            s.store.lake_input_save(&row)?;
            capture_versions(&s, &row, false)?;
            dto(row)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/lake-updates/preparations",responses((status=200,body=LakeUpdatePreparations)),operation_id="lake_updates_preparations")]
pub(super) async fn list(State(s): State<AppState>) -> ApiResult<LakeUpdatePreparations> {
    Ok(Json(
        blocking(move || {
            Ok(LakeUpdatePreparations {
                items: s
                    .store
                    .lake_input_preparations(false)?
                    .into_iter()
                    .map(dto)
                    .collect::<domain::Result<_>>()?,
            })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/lake-updates/preparations/{id}/actions",params(("id"=String,Path)),request_body=LakeUpdateActionRequest,responses((status=200,body=LakeUpdatePreparation)),operation_id="lake_updates_preparation_action")]
pub(super) async fn action(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<LakeUpdateActionRequest>,
) -> ApiResult<LakeUpdatePreparation> {
    Ok(Json(
        blocking(move || {
            let action = match body.action {
                LakeUpdateAction::Resume => "resume",
                LakeUpdateAction::Cancel => "cancel",
                _ => return Err(domain::Error::invalid("范围准备仅支持继续或取消")),
            };
            if action == "cancel"
                && let Ok(slot) = CAPTURE.lock()
                && let Some((active, flag)) = &*slot
                && active == &id
            {
                flag.store(true, std::sync::atomic::Ordering::Release);
            }
            dto(s.store.lake_input_action(&id, action)?)
        })
        .await?,
    ))
}
pub(crate) async fn supervise(s: AppState) {
    let mut shutdown = s.shutdown.subscribe();
    loop {
        if *shutdown.borrow() {
            break;
        }
        let worker = s.clone();
        let outcome = tokio::task::spawn_blocking(move || -> domain::Result<bool> {
            let Some(mut row) = worker
                .store
                .lake_input_preparations(true)?
                .into_iter()
                .next()
            else {
                return Ok(false);
            };
            if !worker.lake_updates.configured() {
                return Ok(false);
            }
            match advance(&worker, &mut row) {
                Ok(()) => {}
                Err(error) => {
                    row.state = "needs_review".into();
                    row.error = Some(format!("{}: {}", error.code, error.message));
                }
            }
            worker.store.lake_input_save(&row)?;
            Ok(true)
        })
        .await;
        let delay = if matches!(outcome, Ok(Ok(true))) {
            100
        } else {
            1500
        };
        tokio::select! {_ = shutdown.changed()=>{}, _ = tokio::time::sleep(std::time::Duration::from_millis(delay))=>{}}
    }
}
fn advance(s: &AppState, row: &mut LakeInputPreparation) -> domain::Result<()> {
    {
        let _creation = CREATION
            .lock()
            .map_err(|_| domain::Error::new("INTERNAL_ERROR", "准备入口锁不可用"))?;
        *row = s.store.lake_input_preparation(&row.id)?;
    }
    let _lease = s.store.lake_input_lease(&row.project_id)?;
    if row.result_id.is_none() {
        row.result_id = s.store.lake_input_pinned_result(&row.project_id, &row.id)?;
    }
    if row.state == "cancelling" && row.result_id.is_none() {
        capture_versions(s, row, false)?;
        row.state = "cancelled".into();
        return Ok(());
    }
    let rid = row.result_id.clone().ok_or_else(|| {
        domain::Error::new(
            "UPDATE_CAPTURE_INTERRUPTED",
            "固定成员创建中断；请取消此准备后重新选择范围",
        )
    })?;
    if row.state == "cancelling" {
        capture_versions(s, row, false)?;
        let result = s.store.query_result(&row.project_id, &rid)?;
        if matches!(
            result.state,
            domain::ResultState::Queued | domain::ResultState::Running
        ) {
            s.queries.cancel(&rid);
            s.store.cancel_result(&row.project_id, &rid)?;
        }
        finish_result(s, row, &rid)?;
        row.state = "cancelled".into();
        return Ok(());
    }
    if row.total.is_some() && !row.inputs.is_empty() && row.inputs.iter().all(|i| i.sealed) {
        finish_result(s, row, &rid)?;
        row.state = "ready".into();
        return Ok(());
    }
    let result = s.store.query_result(&row.project_id, &rid)?;
    {
        let read = s.sources.inspect()?;
        query_views::retain(s, &row.project_id, &result, true, &read)?;
    }
    capture_versions(s, row, false)?;
    if matches!(
        result.state,
        domain::ResultState::Queued | domain::ResultState::Running
    ) {
        return Ok(());
    }
    if result.state != domain::ResultState::Ready {
        return Err(domain::Error::new(
            "UPDATE_INPUT_FAILED",
            "固定成员构建失败，请检查项目查询结果",
        ));
    }
    row.total = result.count;
    for version in &result.source_versions {
        if row.inputs.iter().any(|i| i.library_id == version.source_id) {
            continue;
        }
        let input_id = hex::encode(Sha256::digest(
            format!("{}:{}", row.id, version.source_id).as_bytes(),
        ))[..32]
            .to_owned();
        s.lake_updates.execute(Op::InputCreate,json!({"identity":input_id,"library_id":version.source_id,"source_version":version.catalog_revision,"provenance":{"project_id":row.project_id,"preparation_id":row.id,"result_id":rid}}))?;
        row.inputs.push(LakePreparedInput {
            library_id: version.source_id.clone(),
            input_id,
            count: 0,
            sealed: false,
        });
        s.store.lake_input_save(row)?;
    }
    let page = s
        .store
        .result_page(&row.project_id, &rid, row.after.as_ref(), 1024)?;
    if page.keys.is_empty() {
        for input in &mut row.inputs {
            if input.sealed {
                continue;
            }
            let value = s
                .lake_updates
                .execute(Op::InputSeal, json!({"id":input.input_id}))?;
            input.count = value["count"].as_u64().unwrap_or(0);
            input.sealed = true;
        }
        // Sealed IDs no longer depend on project members or source version leases.
        s.store.lake_input_save(row)?;
        finish_result(s, row, &rid)?;
        row.state = "ready".into();
        return Ok(());
    }
    for input in &mut row.inputs {
        let objects: Vec<_> = page
            .keys
            .iter()
            .filter(|k| k.source_id == input.library_id)
            .map(|k| k.asset_id.clone())
            .collect();
        for block in objects.chunks(4096) {
            let value = s.lake_updates.execute(
                Op::InputAppendBatch,
                json!({"id":input.input_id,"object_sha256s":block}),
            )?;
            input.count = value["count"].as_u64().unwrap_or(input.count);
        }
    }
    row.processed += page.keys.len() as u64;
    row.after = page.keys.last().cloned();
    Ok(())
}
fn finish_result(s: &AppState, row: &LakeInputPreparation, rid: &str) -> domain::Result<()> {
    s.store
        .lake_input_pin(&row.project_id, &row.id, rid, false)?;
    match s.store.release_result(&row.project_id, rid) {
        Ok(result) => {
            let read = s.sources.inspect()?;
            query_views::release_versions(s, &row.project_id, &result, &read)
        }
        Err(e) if e.code == "RESULT_IN_USE" => Ok(()),
        Err(e) => Err(e),
    }
}
