//! A query view is a recipe plus retained source versions. It does not contain a
//! copied member set; capture is the explicit boundary for worksets and tools.
use super::*;
use std::{
    collections::{BTreeSet, VecDeque},
    time::{Duration, Instant},
};
use studio_application::QueryAdapter;
#[cfg(test)]
mod tests;

pub(super) fn release_versions(
    s: &AppState,
    pid: &str,
    result: &domain::QueryResult,
    read: &SourceRead,
) -> domain::Result<()> {
    // Releasing ownership must still work after the request was cancelled. This
    // borrows the existing admission and only exposes bounded lease cleanup.
    let query = read.lease_cleanup();
    for version in &result.source_versions {
        if version.consistency == "retained_online_snapshot" {
            query.release_version(
                &s.store.source(pid, &version.source_id)?,
                &format!("result/{}", result.id),
            )?;
        }
    }
    Ok(())
}

pub(super) fn retain_job(
    s: &AppState,
    pid: &str,
    job: &domain::Job,
    read: &SourceRead,
) -> domain::Result<()> {
    if let Some(id) = s.store.job_owned_result(pid, &job.id)? {
        retain(s, pid, &s.store.query_result(pid, &id)?, true, read)?;
    }
    let frozen = s.store.job_run(pid, &job.id)?;
    let query = read.query(domain::METADATA_MEMORY_BYTES, false);
    for version in frozen.source_versions {
        if version.consistency == "retained_online_snapshot" {
            query.retain_version(
                &s.store.source(pid, &version.source_id)?,
                &version,
                &format!("job/{}", job.id),
                pid,
                true,
            )?;
        }
    }
    Ok(())
}
pub(super) fn release_job(s: &AppState, pid: &str, id: &str) -> domain::Result<()> {
    let frozen = s.store.job_run(pid, id)?;
    let read = s.sources.inspect()?;
    let query = read.query(domain::METADATA_MEMORY_BYTES, false);
    for version in frozen.source_versions {
        if version.consistency == "retained_online_snapshot" {
            query.release_version(
                &s.store.source(pid, &version.source_id)?,
                &format!("job/{id}"),
            )?;
        }
    }
    if let Some(rid) = s.store.job_owned_result(pid, id)? {
        match s.store.release_result(pid, &rid) {
            Ok(result) => release_versions(s, pid, &result, &read)?,
            Err(error) if error.code == "RESULT_IN_USE" => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) fn retain(
    s: &AppState,
    pid: &str,
    result: &domain::QueryResult,
    permanent: bool,
    read: &SourceRead,
) -> domain::Result<()> {
    let query = read.query(domain::METADATA_MEMORY_BYTES, false);
    for version in &result.source_versions {
        let source = s.store.source(pid, &version.source_id)?;
        if version.consistency == "retained_online_snapshot" || source.kind == "demo" {
            query.retain_version(
                &source,
                version,
                &format!("result/{}", result.id),
                pid,
                permanent,
            )?;
        }
    }
    Ok(())
}

pub(super) fn create_fixed(
    s: &AppState,
    pid: &str,
    spec: domain::QuerySpec,
    versions: Vec<domain::QuerySourceVersion>,
    read: &SourceRead,
) -> domain::Result<domain::QueryResult> {
    let result = s.store.create_snapshot_result(pid, spec, versions, false)?;
    retain_created(s, pid, &result, true, read)?;
    s.queries.cache.recent(pid, &result.id);
    Ok(result)
}
pub(super) fn retain_created(
    s: &AppState,
    pid: &str,
    result: &domain::QueryResult,
    permanent: bool,
    read: &SourceRead,
) -> domain::Result<()> {
    if let Err(error) = retain(s, pid, result, permanent, read) {
        if result.state == domain::ResultState::Ready {
            s.store.release_result(pid, &result.id)?;
        } else {
            s.store.cancel_result(pid, &result.id)?;
        }
        if let Err(cleanup) = release_versions(s, pid, result, read) {
            tracing::warn!(result_id=%result.id,error=%cleanup,"failed to release partial view leases");
        }
        return Err(error);
    }
    Ok(())
}

#[utoipa::path(post,path="/v1/projects/{project_id}/query-views",operation_id="browse_query",params(("project_id"=String,Path)),request_body=RunQuery,responses((status=200,body=QueryResult)))]
pub(super) async fn create(
    State(s): State<AppState>,
    Extension(context): Extension<RequestReadContext>,
    Path(pid): Path<String>,
    Body(body): Body<RunQuery>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let spec = domain::QuerySpec::from(body.spec).normalize()?;
            if spec
                .conditions
                .iter()
                .any(|c| c.field.starts_with("project."))
                || spec
                    .input_scope
                    .as_ref()
                    .is_some_and(|v| !matches!(v.target, domain::ScopeTarget::Source { .. }))
            {
                return Err(domain::Error::new(
                    "QUERY_VIEW_UNSUPPORTED",
                    "项目成果或固定子范围筛选需要生成固定结果",
                ));
            }
            let read = read_permit(&s, domain::ReadClass::Index, &context)?;
            let query = read.query(domain::METADATA_MEMORY_BYTES, false);
            let mut versions = Vec::new();
            for id in &spec.source_ids {
                let source = s.store.source(&pid, id)?;
                let fields = query.fields(&source)?;
                fields.validate(&spec)?;
                if !fields.direct_query {
                    return Err(domain::Error::new(
                        "QUERY_VIEW_UNSUPPORTED",
                        "该来源尚未支持直接分页筛选",
                    ));
                }
                let revision = match spec.input_scope.as_ref().map(|s| &s.target) {
                    Some(domain::ScopeTarget::Source { revision, .. }) => Some(revision.as_str()),
                    _ => None,
                };
                versions.push(query.read_version_at(&source, revision, spec.uses_metadata())?);
            }
            let result = s.store.create_snapshot_result(&pid, spec, versions, true)?;
            retain_created(&s, &pid, &result, false, &read)?;
            s.queries.cache.recent(&pid, &result.id);
            Ok(result.into())
        })
        .await?,
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    project_id: String,
    result_id: String,
    order: domain::QueryOrder,
    afters: BTreeMap<String, String>,
    exhausted: BTreeSet<String>,
    totals: BTreeMap<String, u64>,
    scanned: u64,
}
struct Stream {
    source: domain::Source,
    version: domain::QuerySourceVersion,
    after: Option<String>,
    buffer: VecDeque<domain::QueryHit>,
    next: Option<String>,
    done: bool,
    remaining_scan: u64,
}

fn compare(
    order: domain::QueryOrder,
    a: &domain::QueryHit,
    b: &domain::QueryHit,
) -> std::cmp::Ordering {
    if order.by_post() {
        scoped_browse::compare(order, &a.key, a.post_id, &b.key, b.post_id)
    } else if order.descending() {
        b.key
            .source_id
            .cmp(&a.key.source_id)
            .then_with(|| b.key.asset_id.cmp(&a.key.asset_id))
    } else {
        a.key
            .source_id
            .cmp(&b.key.source_id)
            .then_with(|| a.key.asset_id.cmp(&b.key.asset_id))
    }
}

// The only persisted progress is consumed hits or completed empty scans. Buffers
// may be reread on retry, but a budget yield never loses all useful work.
fn merge_page(
    cursor: &mut Cursor,
    streams: &mut [Stream],
    spec: &domain::QuerySpec,
    limit: usize,
    context: &studio_application::SourceReadContext,
    query: &dyn QueryAdapter,
) -> domain::Result<Vec<domain::AssetKey>> {
    let order = spec.order;
    let started = Instant::now();
    let mut keys = Vec::new();
    let mut work = 0_u64;
    loop {
        context.check()?;
        // A sweep must reach every missing head before yielding. Stopping in
        // the middle can discard every fetched hit and repeat the same cursor
        // forever. Each sweep either emits a hit or advances an empty scan;
        // the soft budgets apply only at that resumable boundary. Source pages
        // remain bounded (at most eight sources, 129 hits / 2048 scanned each).
        let needs_read = streams.iter().any(|s| s.buffer.is_empty() && !s.done);
        if needs_read
            && (work >= 8192 || (work > 0 && started.elapsed() > Duration::from_millis(200)))
        {
            break;
        }
        let mut incomplete = false;
        for stream in streams.iter_mut() {
            if !stream.buffer.is_empty() || stream.done {
                continue;
            }
            let page = query.query_page(
                &stream.source,
                spec,
                &stream.version,
                stream.after.as_deref(),
                limit + 1,
                context.cancelled.clone(),
            )?;
            work += page.scanned;
            stream.remaining_scan = page.scanned;
            cursor
                .totals
                .insert(stream.source.id.clone(), page.total_objects);
            stream.next = page.next;
            stream.done = stream.next.is_none();
            stream.buffer = page.hits.into();
            if stream.buffer.is_empty() {
                cursor.scanned = cursor.scanned.saturating_add(stream.remaining_scan);
                stream.remaining_scan = 0;
                stream.after = stream.next.clone();
                if !stream.done {
                    incomplete = true;
                }
            }
        }
        // An unsearched stream may contain an earlier key than every known head.
        // Continue later instead of emitting globally out-of-order results.
        if incomplete {
            break;
        }
        let winner = streams
            .iter()
            .enumerate()
            .filter_map(|(i, stream)| stream.buffer.front().map(|hit| (i, hit)))
            .min_by(|(_, a), (_, b)| compare(order, a, b))
            .map(|(i, _)| i);
        let Some(index) = winner else {
            break;
        };
        let stream = &mut streams[index];
        let hit = stream.buffer.pop_front().expect("known head");
        cursor.scanned = cursor.scanned.saturating_add(1);
        stream.remaining_scan = stream.remaining_scan.saturating_sub(1);
        stream.after = Some(hit.key.asset_id.clone());
        keys.push(hit.key);
        if stream.buffer.is_empty() {
            cursor.scanned = cursor.scanned.saturating_add(stream.remaining_scan);
            stream.remaining_scan = 0;
            if stream.next.is_some() {
                stream.after = stream.next.clone();
            }
        }
        if keys.len() == limit {
            break;
        }
    }
    for stream in streams.iter() {
        if let Some(after) = &stream.after {
            cursor
                .afters
                .insert(stream.source.id.clone(), after.clone());
        }
        if stream.done && stream.buffer.is_empty() {
            cursor.exhausted.insert(stream.source.id.clone());
        }
    }
    Ok(keys)
}

pub(super) fn assets(
    s: &AppState,
    pid: &str,
    rid: &str,
    params: QueryListParams,
    context: &RequestReadContext,
    read: &SourceRead,
) -> domain::Result<ResultAssets> {
    let result = s.store.touch_query_view(pid, rid)?;
    retain(s, pid, &result, false, read)?;
    let mut spec = result.spec.clone();
    let order = params.order.map(Into::into).unwrap_or(spec.order);
    spec.order = order;
    let mut cursor = if let Some(raw) = params.cursor {
        if raw.len() > 8192 {
            return Err(domain::Error::invalid("视图游标过长"));
        }
        let cursor: Cursor = URL_SAFE_NO_PAD
            .decode(raw)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| domain::Error::invalid("视图游标无效"))?;
        if cursor.project_id != pid
            || cursor.result_id != rid
            || cursor.order != order
            || cursor
                .afters
                .keys()
                .chain(cursor.exhausted.iter())
                .chain(cursor.totals.keys())
                .any(|id| !spec.source_ids.contains(id))
        {
            return Err(domain::Error::invalid("视图游标不属于该范围或排序"));
        }
        cursor
    } else {
        Cursor {
            project_id: pid.into(),
            result_id: rid.into(),
            order,
            afters: BTreeMap::new(),
            exhausted: BTreeSet::new(),
            totals: BTreeMap::new(),
            scanned: 0,
        }
    };
    let mut streams = result
        .source_versions
        .iter()
        .map(|version| {
            Ok(Stream {
                source: s.store.source(pid, &version.source_id)?,
                version: version.clone(),
                after: cursor.afters.get(&version.source_id).cloned(),
                buffer: VecDeque::new(),
                next: None,
                done: cursor.exhausted.contains(&version.source_id),
                remaining_scan: 0,
            })
        })
        .collect::<domain::Result<Vec<_>>>()?;
    let query = read.query(domain::METADATA_MEMORY_BYTES, false);
    let limit = params.limit.unwrap_or(48).clamp(1, 128);
    let keys = merge_page(
        &mut cursor,
        &mut streams,
        &spec,
        limit,
        &read.context,
        &query,
    )?;
    let more = streams.iter().any(|s| !s.done || !s.buffer.is_empty());
    let mut resolved = std::collections::HashMap::new();
    for stream in &streams {
        let selected = keys
            .iter()
            .filter(|k| k.source_id == stream.source.id)
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        for item in read.freeze_at(
            &stream.source,
            &selected,
            Some(&stream.version.catalog_revision),
        )? {
            resolved.insert(item.asset.key.clone(), item.asset);
        }
    }
    let membership = s.store.contains(pid, &keys)?;
    let mut items = keys
        .into_iter()
        .zip(membership)
        .map(|(key, selected)| {
            resolved
                .remove(&key)
                .map(|asset| Asset::from_domain(asset, selected))
                .ok_or_else(|| domain::Error::new("SOURCE_FORMAT_ERROR", "视图成员缺少对象记录"))
        })
        .collect::<domain::Result<Vec<_>>>()?;
    enrich_summaries_at(s, pid, context, read, &mut items, &result.source_versions)?;
    let scan = (more && items.len() < limit).then(|| BrowseScan {
        scanned: cursor.scanned,
        total: cursor.totals.values().copied().sum(),
    });
    let next = if more {
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(domain::Error::io)?))
    } else {
        None
    };
    Ok(ResultAssets {
        result_id: rid.into(),
        count: None,
        page: AssetPage {
            items,
            next_cursor: next,
            revision: rid.into(),
            preparing: None,
            result_id: Some(rid.into()),
            scan,
            start_cursor: None,
        },
    })
}
