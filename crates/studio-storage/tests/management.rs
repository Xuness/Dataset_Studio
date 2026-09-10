use rusqlite::{Connection, params};
use std::collections::BTreeSet;
use studio_application::{
    ManagementRepository, ProjectRepository, QueryRepository, ScopeRepository,
};
use studio_domain::*;
use studio_storage::SqliteStore;

struct Fixture {
    store: SqliteStore,
    project: Project,
    source: Source,
    _root: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().join("runtime")).unwrap();
    let project = store.create("对象管理验收", None).unwrap();
    let source = Source {
        id: new_id(),
        name: "只读参考资料".into(),
        kind: "demo".into(),
        index_root: None,
        media_root: None,
    };
    store.attach(&project.id, source.clone()).unwrap();
    Fixture {
        store,
        project,
        source,
        _root: root,
    }
}
fn keys(f: &Fixture, range: std::ops::Range<usize>) -> Vec<AssetKey> {
    range
        .map(|n| AssetKey {
            source_id: f.source.id.clone(),
            asset_id: format!("{n:08}"),
        })
        .collect()
}
fn members(f: &Fixture) -> BTreeSet<String> {
    let mut cursor = None;
    let mut members = BTreeSet::new();
    loop {
        let page = f
            .store
            .selection_keys(&f.project.id, cursor.as_ref(), 128)
            .unwrap();
        if page.is_empty() {
            break;
        }
        members.extend(page.iter().map(|key| key.asset_id.clone()));
        cursor = page.last().cloned();
    }
    members
}
fn choose(
    f: &Fixture,
    add: std::ops::Range<usize>,
    remove: std::ops::Range<usize>,
    clear: bool,
) -> Selection {
    let previous = f.store.selection(&f.project.id).unwrap();
    f.store
        .change_selection(
            &f.project.id,
            previous.revision,
            &keys(f, add),
            &keys(f, remove),
            clear,
        )
        .unwrap()
}
fn object(f: &Fixture, kind: ObjectKind, id: &str) -> ObjectDetails {
    f.store.object_details(&f.project.id, kind, id).unwrap()
}
fn remove(f: &Fixture, kind: ObjectKind, id: &str) -> Result<()> {
    f.store
        .remove_object(&f.project.id, kind, id, object(f, kind, id).object.revision)
}
fn spec(f: &Fixture) -> QuerySpec {
    QuerySpec {
        version: 1,
        source_ids: vec![f.source.id.clone()],
        conditions: vec![],
        observation_rule: ObservationRule::CurrentPost,
        order: QueryOrder::AssetKeyAsc,
        input_scope: None,
    }
}
fn result(f: &Fixture, count: usize) -> QueryResult {
    let query = f
        .store
        .save_query(&f.project.id, "全部参考对象", spec(f), None)
        .unwrap();
    let result = f
        .store
        .create_result(
            &f.project.id,
            Some((&query.id, query.revision)),
            query.spec,
            vec![QuerySourceVersion {
                source_id: f.source.id.clone(),
                catalog_revision: "fixed-v1".into(),
                analysis_sequence: None,
                consistency: "fixture".into(),
            }],
        )
        .unwrap();
    f.store.start_result(&f.project.id, &result.id).unwrap();
    for batch in keys(f, 0..count).chunks(512) {
        f.store
            .append_result(&f.project.id, &result.id, batch, batch.len() as u64)
            .unwrap();
    }
    f.store
        .finish_result(&f.project.id, &result.id, None)
        .unwrap()
}

#[test]
fn undo_and_redo_preserve_bulk_members_exclusions_and_project_restart() {
    let f = fixture();
    let base = result(&f, 1000);
    let scope = ScopeRef {
        project_id: f.project.id.clone(),
        target: ScopeTarget::QueryResult {
            result_id: base.id.clone(),
        },
    };
    let first = f
        .store
        .change_selection_scope(&f.project.id, 0, &scope, ScopeOperation::Replace)
        .unwrap();
    assert_eq!(first.base_result.as_deref(), Some(base.id.as_str()));
    let original = members(&f);
    choose(&f, 2000..2002, 8..11, false);
    let edited = members(&f);
    assert_eq!(edited.len(), 999);
    let workset = f.store.save_collection(&f.project.id, "固定候选").unwrap();
    choose(&f, 0..0, 0..0, true);
    assert!(members(&f).is_empty());
    let selected = f.store.selection(&f.project.id).unwrap();
    let restored = f
        .store
        .restore_selection(&f.project.id, selected.revision, false)
        .unwrap();
    assert_eq!(members(&f), edited);
    assert_eq!(
        restored.selection.base_result.as_deref(),
        Some(base.id.as_str())
    );
    f.store.close(&f.project.id).unwrap();
    f.store.open_recent(&f.project.id).unwrap();
    let restored = f
        .store
        .restore_selection(&f.project.id, restored.selection.revision, false)
        .unwrap();
    assert_eq!(members(&f), original);
    f.store
        .restore_selection(&f.project.id, restored.selection.revision, true)
        .unwrap();
    assert_eq!(members(&f), edited);
    assert_eq!(f.store.collections(&f.project.id).unwrap()[0].count, 999);
    assert_eq!(workset.count, 999);
}

#[test]
fn history_cas_branching_and_smaller_limit_keep_a_contiguous_redo_chain() {
    let f = fixture();
    for n in 0..4 {
        choose(&f, n..n + 1, 0..0, false);
    }
    let old = f.store.selection(&f.project.id).unwrap();
    f.store
        .restore_selection(&f.project.id, old.revision, false)
        .unwrap();
    assert_eq!(
        f.store
            .restore_selection(&f.project.id, old.revision, false)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    for _ in 0..2 {
        let current = f.store.selection(&f.project.id).unwrap();
        f.store
            .restore_selection(&f.project.id, current.revision, false)
            .unwrap();
    }
    assert_eq!(members(&f).len(), 1);
    f.store.configure_editing(2, 0).unwrap();
    let history = f.store.selection_history(&f.project.id).unwrap();
    assert_eq!((history.undo_steps, history.redo_steps), (1, 1));
    let current = f
        .store
        .restore_selection(&f.project.id, history.selection.revision, true)
        .unwrap();
    assert_eq!(current.selection.count, 2);
    assert_eq!(current.redo_steps, 0);
    f.store
        .restore_selection(&f.project.id, current.selection.revision, false)
        .unwrap();
    choose(&f, 8..9, 0..0, false);
    assert_eq!(
        f.store.selection_history(&f.project.id).unwrap().redo_steps,
        0
    );
    let settings = f.store.editing_settings().unwrap();
    f.store.configure_editing(0, settings.revision).unwrap();
    assert_eq!(
        f.store.selection_history(&f.project.id).unwrap().undo_steps,
        0
    );
    let before = members(&f);
    choose(&f, 9..10, 0..0, false);
    assert_eq!(
        f.store.selection_history(&f.project.id).unwrap().undo_steps,
        0
    );
    assert_eq!(members(&f).len(), before.len() + 1);
    assert!(f.store.configure_editing(201, 2).is_err());
}

#[test]
fn selection_history_protects_result_until_clear_without_changing_selection() {
    let f = fixture();
    let base = result(&f, 10);
    let scope = ScopeRef {
        project_id: f.project.id.clone(),
        target: ScopeTarget::QueryResult {
            result_id: base.id.clone(),
        },
    };
    f.store
        .change_selection_scope(&f.project.id, 0, &scope, ScopeOperation::Replace)
        .unwrap();
    choose(&f, 20..21, 0..0, true);
    assert_eq!(
        f.store
            .release_result(&f.project.id, &base.id)
            .unwrap_err()
            .code,
        "RESULT_IN_USE"
    );
    let details = object(&f, ObjectKind::QueryResult, &base.id);
    assert!(
        details
            .incoming
            .iter()
            .any(|link| link.kind == ObjectKind::SelectionHistory && link.blocking)
    );
    let current = f.store.selection(&f.project.id).unwrap();
    f.store
        .clear_selection_history(&f.project.id, current.revision)
        .unwrap();
    assert_eq!(
        f.store.selection(&f.project.id).unwrap().revision,
        current.revision
    );
    assert_eq!(
        members(&f),
        keys(&f, 20..21)
            .into_iter()
            .map(|key| key.asset_id)
            .collect()
    );
    f.store.release_result(&f.project.id, &base.id).unwrap();
}

#[test]
fn source_detach_is_local_and_keeps_names_fixed_worksets_and_selection() {
    let f = fixture();
    let other = f.store.create("另一个项目", None).unwrap();
    f.store.attach(&other.id, f.source.clone()).unwrap();
    choose(&f, 0..3, 0..0, false);
    let workset = f
        .store
        .save_collection(&f.project.id, "保留的图片")
        .unwrap();
    let before = object(&f, ObjectKind::Source, &f.source.id);
    f.store
        .edit_object(
            &f.project.id,
            ObjectKind::Source,
            &f.source.id,
            EditObject {
                expected_revision: before.object.revision,
                name: "此项目的来源别名".into(),
                notes: "用于筛选".into(),
            },
        )
        .unwrap();
    remove(&f, ObjectKind::Source, &f.source.id).unwrap();
    assert!(f.store.sources(&f.project.id).unwrap().is_empty());
    assert_eq!(
        f.store
            .source(&f.project.id, &f.source.id)
            .unwrap_err()
            .code,
        "SOURCE_DETACHED"
    );
    assert_eq!(
        f.store.source(&other.id, &f.source.id).unwrap().name,
        f.source.name
    );
    assert_eq!(
        f.store.collections(&f.project.id).unwrap()[0].id,
        workset.id
    );
    assert_eq!(f.store.selection(&f.project.id).unwrap().count, 3);
    let detached = object(&f, ObjectKind::Source, &f.source.id);
    assert_eq!(detached.object.state, "detached");
    assert_eq!(detached.object.notes, "用于筛选");
    f.store
        .restore_source(&f.project.id, &f.source.id, detached.object.revision)
        .unwrap();
    assert_eq!(
        f.store.source(&f.project.id, &f.source.id).unwrap().name,
        "此项目的来源别名"
    );
}

#[test]
fn deletion_explains_live_dependencies_and_preserves_frozen_job_input() {
    let f = fixture();
    choose(&f, 0..4, 0..0, false);
    let workset = f
        .store
        .save_collection(&f.project.id, "需检查引用")
        .unwrap();
    let scope = ScopeRef {
        project_id: f.project.id.clone(),
        target: ScopeTarget::Workset {
            collection_id: workset.id.clone(),
        },
    };
    let mut query = spec(&f);
    query.version = 2;
    query.input_scope = Some(scope.clone());
    let query = f
        .store
        .save_query(&f.project.id, "依赖工作集的查询", query, None)
        .unwrap();
    let job = f
        .store
        .submit_scope_job(&f.project.id, &new_id(), &scope, 0, None)
        .unwrap();
    let details = object(&f, ObjectKind::Workset, &workset.id);
    assert!(!details.can_remove);
    assert!(
        details
            .incoming
            .iter()
            .any(|link| link.kind == ObjectKind::Query
                && link.name == "依赖工作集的查询"
                && link.blocking)
    );
    assert!(
        details
            .incoming
            .iter()
            .any(|link| link.id == job.id && !link.blocking)
    );
    assert_eq!(
        remove(&f, ObjectKind::Workset, &workset.id)
            .unwrap_err()
            .code,
        "OBJECT_IN_USE"
    );
    assert_eq!(
        remove(&f, ObjectKind::Source, &f.source.id)
            .unwrap_err()
            .code,
        "OBJECT_IN_USE"
    );
    remove(&f, ObjectKind::Query, &query.id).unwrap();
    assert_eq!(
        f.store
            .query_definition(&f.project.id, &query.id)
            .unwrap_err()
            .code,
        "OBJECT_REMOVED"
    );
    remove(&f, ObjectKind::Workset, &workset.id).unwrap();
    assert!(f.store.collections(&f.project.id).unwrap().is_empty());
    assert_eq!(
        f.store.job_inputs(&f.project.id, &job.id, None).unwrap(),
        keys(&f, 0..4)
    );
    let db = Connection::open(f.project.directory.join("project.sqlite")).unwrap();
    let violations: bool = db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(!violations);
}

#[test]
fn metadata_and_presets_survive_reopen_and_listing_cursors_bind_conditions() {
    let f = fixture();
    let manifest = std::fs::read(f.project.directory.join("project.json")).unwrap();
    let updated = f
        .store
        .edit_object(
            &f.project.id,
            ObjectKind::Project,
            &f.project.id,
            EditObject {
                expected_revision: 0,
                name: "新的显示名称".into(),
                notes: "项目用途".into(),
            },
        )
        .unwrap();
    assert_eq!(updated.revision, 1);
    assert_eq!(f.store.project(&f.project.id).unwrap().name, "新的显示名称");
    choose(&f, 0..1, 0..0, false);
    for name in ["候选丙", "候选甲", "候选乙"] {
        f.store.save_collection(&f.project.id, name).unwrap();
    }
    let query = ObjectListing {
        kind: ObjectKind::Workset,
        search: "候选".into(),
        order: "name_asc".into(),
        state: String::new(),
        subtype: None,
        include_archived: false,
        after: None,
        limit: 1,
    };
    let first = f
        .store
        .managed_objects(&f.project.id, query.clone())
        .unwrap();
    let second = f
        .store
        .managed_objects(
            &f.project.id,
            ObjectListing {
                after: first.next_cursor.clone(),
                ..query.clone()
            },
        )
        .unwrap();
    assert_ne!(first.items[0].id, second.items[0].id);
    assert!(
        f.store
            .managed_objects(
                &f.project.id,
                ObjectListing {
                    search: "其他".into(),
                    after: first.next_cursor,
                    ..query
                }
            )
            .is_err()
    );
    let preset = f
        .store
        .save_tool_preset(
            &f.project.id,
            None,
            "常用参数",
            "可复用",
            0,
            OperatorRun {
                operator_id: "core.manifest".into(),
                operator_version: 1,
                parameters_version: 1,
                parameters: serde_json::json!({"fields":[]}),
            },
        )
        .unwrap();
    f.store.close(&f.project.id).unwrap();
    f.store.open_recent(&f.project.id).unwrap();
    assert_eq!(f.store.project(&f.project.id).unwrap().name, "新的显示名称");
    assert_eq!(
        std::fs::read(f.project.directory.join("project.json")).unwrap(),
        manifest
    );
    assert_eq!(
        f.store
            .tool_presets(&f.project.id, "core.manifest", None, 32)
            .unwrap()
            .items[0]
            .id,
        preset.id
    );
    assert_eq!(
        f.store
            .delete_tool_preset(&f.project.id, &preset.id, 0)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    f.store
        .delete_tool_preset(&f.project.id, &preset.id, preset.revision)
        .unwrap();
}

#[test]
fn finished_task_cleanup_unprotects_inputs_but_archiving_keeps_them() {
    let f = fixture();
    choose(&f, 0..2, 0..0, false);
    let key = new_id();
    let selection = f.store.selection(&f.project.id).unwrap();
    let job = f
        .store
        .submit_job(&f.project.id, &key, selection.revision, 0)
        .unwrap();
    let db = Connection::open(f.project.directory.join("project.sqlite")).unwrap();
    db.execute("UPDATE jobs SET status='cancelled' WHERE id=?1", [&job.id])
        .unwrap();
    let details = object(&f, ObjectKind::Job, &job.id);
    f.store
        .archive_job(&f.project.id, &job.id, true, details.object.revision)
        .unwrap();
    assert_eq!(
        f.store
            .job_inputs(&f.project.id, &job.id, None)
            .unwrap()
            .len(),
        2
    );
    remove(&f, ObjectKind::Job, &job.id).unwrap();
    assert!(
        f.store
            .job_inputs(&f.project.id, &job.id, None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.store.retry_job(&f.project.id, &job.id).unwrap_err().code,
        "JOB_NOT_RETRYABLE"
    );
    assert_eq!(
        f.store
            .submit_job(&f.project.id, &key, selection.revision, 0)
            .unwrap_err()
            .code,
        "OBJECT_REMOVED"
    );
    let remaining: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM result_references WHERE owner_kind='job' AND owner_id=?1",
            params![job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
}
