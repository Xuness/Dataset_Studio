use rusqlite::{Connection, OpenFlags};
use studio_application::{ProjectRepository, QueryRepository, ScopeRepository};
use studio_domain::*;
use studio_storage::SqliteStore;

fn source(id: &str) -> Source {
    Source {
        id: id.into(),
        name: "夹具".into(),
        kind: "demo".into(),
        index_root: None,
        media_root: None,
    }
}
fn spec(sid: &str, order: QueryOrder) -> QuerySpec {
    QuerySpec {
        version: 1,
        source_ids: vec![sid.into()],
        conditions: vec![],
        observation_rule: ObservationRule::AnyObservation,
        order,
    }
}
fn versions(sid: &str) -> Vec<QuerySourceVersion> {
    vec![QuerySourceVersion {
        source_id: sid.into(),
        catalog_revision: "fixture-v1".into(),
        analysis_sequence: None,
        consistency: "fixture".into(),
    }]
}
fn keys(sid: &str, start: usize, count: usize) -> Vec<AssetKey> {
    (start..start + count)
        .map(|i| AssetKey {
            source_id: sid.into(),
            asset_id: format!("{i:08}"),
        })
        .collect()
}
fn build(
    store: &SqliteStore,
    pid: &str,
    sid: &str,
    start: usize,
    count: usize,
    order: QueryOrder,
) -> QueryResult {
    let query = store
        .save_query(pid, "范围", spec(sid, order), None)
        .unwrap();
    let result = store
        .create_result(
            pid,
            Some((&query.id, query.revision)),
            query.spec,
            versions(sid),
        )
        .unwrap();
    assert!(result.count.is_none());
    assert!(store.start_result(pid, &result.id).unwrap());
    for batch in keys(sid, start, count).chunks(512) {
        store
            .append_result(pid, &result.id, batch, batch.len() as u64)
            .unwrap();
    }
    // An observation duplicate must not add a second stored object.
    if count > 0 {
        store
            .append_result(pid, &result.id, &keys(sid, start, 1), 1)
            .unwrap();
    }
    assert!(store.query_result(pid, &result.id).unwrap().count.is_none());
    let ready = store.finish_result(pid, &result.id, None).unwrap();
    assert_eq!(ready.count, Some(count as u64));
    ready
}
fn result_scope(pid: &str, rid: &str) -> ScopeRef {
    ScopeRef {
        project_id: pid.into(),
        target: ScopeTarget::QueryResult {
            result_id: rid.into(),
        },
    }
}
#[test]
fn backend_all_selection_is_sparse_and_fixed_inputs_survive_set_changes_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let store = SqliteStore::new(runtime.clone()).unwrap();
    let p = store.create("范围项目", None).unwrap();
    let other = store.create("隔离项目", None).unwrap();
    let sid = new_id();
    store.attach(&p.id, source(&sid)).unwrap();
    let a = build(&store, &p.id, &sid, 0, 10000, QueryOrder::AssetKeyAsc);
    let b = build(&store, &p.id, &sid, 5000, 10000, QueryOrder::AssetKeyDesc);
    let scope = result_scope(&p.id, &a.id);
    assert!(serde_json::to_vec(&scope).unwrap().len() < 200);
    let selected = store
        .change_selection_scope(&p.id, 0, &scope, ScopeOperation::Replace)
        .unwrap();
    assert_eq!(selected.count, 10000);
    assert_eq!(selected.base_result.as_deref(), Some(a.id.as_str()));
    let excluded = store
        .change_selection(&p.id, selected.revision, &[], &keys(&sid, 4, 1), false)
        .unwrap();
    assert_eq!(excluded.count, 9999);
    assert_eq!(excluded.excluded_count, 1);
    let check = Connection::open_with_flags(
        p.directory.join("project.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        check
            .query_row("SELECT COUNT(*) FROM selection", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        check
            .query_row("SELECT COUNT(*) FROM selection_exclusions", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    drop(check);
    let fixed_scope = ScopeRef {
        project_id: p.id.clone(),
        target: ScopeTarget::Selection {
            revision: excluded.revision,
        },
    };
    let collection = store
        .save_scope_collection(&p.id, "固定集合", &fixed_scope)
        .unwrap();
    let key = new_id();
    let job = store
        .submit_scope_job(&p.id, &key, &fixed_scope, 0, None)
        .unwrap();
    assert_eq!(job.total, 9999);
    let selected = store
        .change_selection_scope(
            &p.id,
            excluded.revision,
            &result_scope(&p.id, &b.id),
            ScopeOperation::Add,
        )
        .unwrap();
    assert_eq!(selected.count, 14999);
    let selected = store
        .change_selection_scope(
            &p.id,
            selected.revision,
            &result_scope(&p.id, &b.id),
            ScopeOperation::Remove,
        )
        .unwrap();
    assert_eq!(selected.count, 4999);
    let selected = store
        .change_selection_scope(
            &p.id,
            selected.revision,
            &result_scope(&p.id, &b.id),
            ScopeOperation::Intersect,
        )
        .unwrap();
    assert_eq!(selected.count, 0);
    assert_eq!(
        store
            .submit_scope_job(&p.id, &key, &fixed_scope, 0, None)
            .unwrap()
            .id,
        job.id
    );
    assert_eq!(
        store
            .change_selection_scope(&p.id, excluded.revision, &scope, ScopeOperation::Replace)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        store
            .change_selection_scope(&other.id, 0, &scope, ScopeOperation::Replace)
            .unwrap_err()
            .code,
        "SCOPE_PROJECT_MISMATCH"
    );
    assert_eq!(
        store.query_result(&other.id, &a.id).unwrap_err().code,
        "NOT_FOUND"
    );
    assert_eq!(
        store.release_result(&p.id, &a.id).unwrap_err().code,
        "RESULT_IN_USE"
    );
    drop(store);
    let store = SqliteStore::new(runtime).unwrap();
    assert!(store.recover_jobs().unwrap().is_empty());
    store.open_recent(&p.id).unwrap();
    assert_eq!(store.job(&p.id, &job.id).unwrap().total, 9999);
    assert_eq!(store.collections(&p.id).unwrap()[0].count, 9999);
    let mut actual = Vec::new();
    let mut after = None;
    loop {
        let batch = store.job_inputs(&p.id, &job.id, after.as_ref()).unwrap();
        if batch.is_empty() {
            break;
        }
        after = batch.last().cloned();
        actual.extend(batch);
    }
    let expected = keys(&sid, 0, 10000)
        .into_iter()
        .filter(|k| k.asset_id != "00000004")
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(
        store
            .collection_keys(&p.id, &collection.id, None, 1000)
            .unwrap(),
        expected[..1000]
    );
    assert_eq!(
        store.job(&p.id, &job.id).unwrap().input_scope,
        Some(fixed_scope)
    );
}
#[test]
fn result_paging_is_stable_and_incomplete_material_never_becomes_a_scope() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().into()).unwrap();
    let p = store.create("分页", None).unwrap();
    let sid = new_id();
    store.attach(&p.id, source(&sid)).unwrap();
    let result = build(&store, &p.id, &sid, 0, 1111, QueryOrder::AssetKeyDesc);
    let mut after = None;
    let mut actual = Vec::new();
    loop {
        let page = store
            .result_page(&p.id, &result.id, after.as_ref(), 73)
            .unwrap();
        actual.extend(page.keys);
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    let mut expected = keys(&sid, 0, 1111);
    expected.reverse();
    assert_eq!(actual, expected);
    let queued = store
        .create_result(
            &p.id,
            None,
            spec(&sid, QueryOrder::AssetKeyAsc),
            versions(&sid),
        )
        .unwrap();
    assert_eq!(
        store
            .result_page(&p.id, &queued.id, None, 10)
            .unwrap_err()
            .code,
        "RESULT_NOT_READY"
    );
    store.start_result(&p.id, &queued.id).unwrap();
    store
        .append_result(&p.id, &queued.id, &keys(&sid, 0, 3), 3)
        .unwrap();
    store.cancel_result(&p.id, &queued.id).unwrap();
    assert_eq!(
        store
            .append_result(&p.id, &queued.id, &keys(&sid, 3, 1), 1)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(
        store.finish_result(&p.id, &queued.id, None).unwrap().state,
        ResultState::Cancelled
    );
    assert_eq!(
        store
            .change_selection_scope(
                &p.id,
                0,
                &result_scope(&p.id, &queued.id),
                ScopeOperation::Replace
            )
            .unwrap_err()
            .code,
        "RESULT_NOT_READY"
    );
    assert_eq!(
        store.release_result(&p.id, &queued.id).unwrap().state,
        ResultState::Released
    );
    let db = Connection::open_with_flags(
        p.directory.join("project.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM result_members WHERE result_id=?1",
            [queued.id],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn query_edits_keep_old_members_and_source_jobs_wait_for_complete_capture() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("state");
    let store = SqliteStore::new(runtime.clone()).unwrap();
    let p = store.create("来源输入", None).unwrap();
    let sid = new_id();
    store.attach(&p.id, source(&sid)).unwrap();
    let result = build(&store, &p.id, &sid, 0, 20, QueryOrder::AssetKeyAsc);
    let q = store
        .query_definition(&p.id, result.definition_id.as_deref().unwrap())
        .unwrap();
    let revised = store
        .save_query(
            &p.id,
            "新的顺序",
            spec(&sid, QueryOrder::AssetKeyDesc),
            Some((&q.id, q.revision)),
        )
        .unwrap();
    assert_eq!(revised.revision, 2);
    assert_eq!(
        store.query_result(&p.id, &result.id).unwrap().spec.order,
        QueryOrder::AssetKeyAsc
    );
    assert_eq!(
        store
            .create_result(&p.id, Some((&q.id, 1)), q.spec, versions(&sid))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let scope = ScopeRef {
        project_id: p.id.clone(),
        target: ScopeTarget::Source {
            source_id: sid.clone(),
            revision: "fixture-v1".into(),
        },
    };
    let job = store
        .submit_scope_job(
            &p.id,
            &new_id(),
            &scope,
            0,
            Some((spec(&sid, QueryOrder::AssetKeyAsc), versions(&sid))),
        )
        .unwrap();
    assert_eq!(job.status, "waiting_input");
    assert_eq!(job.total, 0);
    let rid = store.job_owned_result(&p.id, &job.id).unwrap().unwrap();
    store.start_result(&p.id, &rid).unwrap();
    store
        .append_result(&p.id, &rid, &keys(&sid, 50, 4), 4)
        .unwrap();
    store.resolve_job_scopes(&p.id).unwrap();
    assert_eq!(store.job(&p.id, &job.id).unwrap().status, "waiting_input");
    store.finish_result(&p.id, &rid, None).unwrap();
    store.resolve_job_scopes(&p.id).unwrap();
    assert_eq!(store.job(&p.id, &job.id).unwrap().total, 4);
    assert_eq!(
        store.job_inputs(&p.id, &job.id, None).unwrap(),
        keys(&sid, 50, 4)
    );
    let interrupted = store
        .create_result(
            &p.id,
            None,
            spec(&sid, QueryOrder::AssetKeyAsc),
            versions(&sid),
        )
        .unwrap();
    store.start_result(&p.id, &interrupted.id).unwrap();
    store
        .append_result(&p.id, &interrupted.id, &keys(&sid, 0, 2), 2)
        .unwrap();
    drop(store);
    let store = SqliteStore::new(runtime).unwrap();
    store.recover_jobs().unwrap();
    store.open_recent(&p.id).unwrap();
    assert_eq!(
        store.query_result(&p.id, &interrupted.id).unwrap().state,
        ResultState::Interrupted
    );
    assert!(
        store
            .query_result(&p.id, &interrupted.id)
            .unwrap()
            .count
            .is_none()
    );
}
