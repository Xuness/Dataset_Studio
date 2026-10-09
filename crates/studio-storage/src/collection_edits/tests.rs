use super::*;
use crate::ranking_projection::RankingProjectionReader;
use studio_application::{ProjectRepository, QueryRepository, ScopeRepository};

struct Fixture {
    store: SqliteStore,
    project: Project,
    source: Source,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
        fs::create_dir_all(&root).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("collection-edits-")
            .tempdir_in(root)
            .unwrap();
        let store = SqliteStore::new(temp.path().join("app")).unwrap();
        let project = store.create("成员版本", None).unwrap();
        let source = Source {
            id: new_id(),
            name: "fixture".into(),
            kind: "demo".into(),
            index_root: None,
            media_root: None,
        };
        store.attach(&project.id, source.clone()).unwrap();
        Self {
            _temp: temp,
            store,
            project,
            source,
        }
    }
    fn key(&self, n: u64) -> AssetKey {
        AssetKey {
            source_id: self.source.id.clone(),
            asset_id: format!("{n:064x}"),
        }
    }
    fn scope(&self, c: &Collection, revision: Option<u64>) -> ScopeRef {
        ScopeRef {
            project_id: self.project.id.clone(),
            target: ScopeTarget::Workset {
                collection_id: c.id.clone(),
                revision,
            },
        }
    }
    fn collection(&self) -> Collection {
        self.store
            .change_selection(&self.project.id, 0, &[self.key(0), self.key(1)], &[], false)
            .unwrap();
        self.store
            .save_collection(&self.project.id, "两张图片")
            .unwrap()
    }
    fn edit(&self, c: &Collection, change: CollectionChange) -> CollectionEditResult {
        self.store
            .edit_collection(
                &self.project.id,
                &c.id,
                &CollectionEdit {
                    request_id: new_id(),
                    expected_revision: c.revision,
                    change,
                },
                &mut metadata,
            )
            .unwrap()
    }
    fn ids(&self, scope: &ScopeRef) -> Vec<u64> {
        self.store
            .browse_scope_keys(&self.project.id, scope, None, 4096, false)
            .unwrap()
            .iter()
            .map(|k| u64::from_str_radix(&k.asset_id, 16).unwrap())
            .collect()
    }
    fn values(&self, sql: &str) -> u64 {
        self.store
            .handle(&self.project.id)
            .unwrap()
            .read()
            .unwrap()
            .query_row(sql, [], |r| unsigned(r, 0))
            .unwrap()
    }
    fn ranking(&self) -> Collection {
        self.ranking_sized(64, Some(10))
    }
    fn ranking_sized(&self, count: u64, top: Option<u64>) -> Collection {
        let dir = self.project.directory.join("artifacts");
        fs::create_dir_all(&dir).unwrap();
        let input_path = dir.join("fixture.ranking-input.sqlite");
        let score_path = dir.join("fixture.ranking.sqlite");
        let mut input = ranking_tables::RankingInputTable::create_v2(&input_path).unwrap();
        let mut scores = ranking_tables::RankingResultTable::create_v2(&score_path).unwrap();
        let rows = (0..count)
            .map(|n| RankingInput {
                ordinal: n,
                source_id: self.source.id.clone(),
                asset_id: self.key(n).asset_id,
                post_id: Some(n as i64 + 1),
                rating: Some("g".into()),
                stored_extension: "png".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        for rows in rows.chunks(512) {
            input.append(rows).unwrap();
        }
        let values = rows
            .iter()
            .map(|row| RankingScores {
                ordinal: row.ordinal,
                rating: row.rating.clone(),
                main_rank: (row.ordinal < count / 2).then_some(row.ordinal + 1),
                rescue_rank: (row.ordinal < count / 2).then(|| count / 2 - row.ordinal),
                v2: (row.ordinal < count / 2).then(|| RankingV2Scores {
                    direct_rank: row.ordinal + 1,
                    fused_rank: count / 2 - row.ordinal,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        for rows in values.chunks(512) {
            scores.append(rows).unwrap();
        }
        input.finalize(&["g".into()]).unwrap();
        scores
            .finish(&RankingSummary {
                input_count: count,
                ratings: vec![RankingRatingSummary {
                    rating: "g".into(),
                    eligible: count / 2,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        drop(input);
        drop(scores);
        let aid = new_id();
        let jid = new_id();
        let p = self.store.handle(&self.project.id).unwrap();
        let db = p.db.lock().unwrap();
        db.execute("INSERT INTO jobs(id,operator,status,total,created_at,idempotency_key,request_hash,delay_ms) VALUES(?1,'danbooru.metarecall','succeeded',?2,'1',?1,'fixture',0)",params![jid,count as i64]).unwrap();
        let provenance = ArtifactProvenance {
            run: None,
            input_scope: None,
            input_sha256: None,
            attempt: Some(1),
            input_artifacts: vec![],
            fields_frozen: true,
            evidence: "fixture".into(),
        };
        db.execute("INSERT INTO artifacts(id,job_id,output_id,name,kind,schema_version,status,count,created_at,files_json,provenance_json) VALUES(?1,?2,'data','fixture',?3,2,'ready',?5,'1','[]',?4)",params![aid,jid,RANKING_KIND,serde_json::to_string(&provenance).unwrap(),count as i64]).unwrap();
        drop(db);
        self.store
            .ranking_workset(
                &self.project.id,
                &aid,
                &new_id(),
                "排名前十",
                &RankingFilter {
                    top,
                    order: RankingOrder::Main,
                    ..Default::default()
                },
                (&score_path, &input_path),
            )
            .unwrap()
    }
}
fn metadata(keys: &[AssetKey]) -> Result<Vec<RankingInput>> {
    Ok(keys
        .iter()
        .map(|key| RankingInput {
            source_id: key.source_id.clone(),
            asset_id: key.asset_id.clone(),
            rating: Some(
                if key.asset_id.ends_with("270f") {
                    "s"
                } else {
                    "g"
                }
                .into(),
            ),
            post_id: u64::from_str_radix(&key.asset_id, 16)
                .ok()
                .map(|id| id as i64 + 1),
            stored_extension: "png".into(),
            ..Default::default()
        })
        .collect())
}
fn points(keys: Vec<AssetKey>) -> CollectionMemberInput {
    CollectionMemberInput::Keys { keys }
}

fn spec(f: &Fixture, scope: ScopeRef) -> QuerySpec {
    QuerySpec {
        version: 3,
        source_ids: vec![f.source.id.clone()],
        conditions: vec![],
        observation_rule: ObservationRule::CurrentPost,
        order: QueryOrder::AssetKeyAsc,
        input_scope: Some(scope),
    }
}
fn versions(f: &Fixture) -> Vec<QuerySourceVersion> {
    vec![QuerySourceVersion {
        source_id: f.source.id.clone(),
        catalog_revision: "demo-v1".into(),
        analysis_sequence: None,
        semantics_version: None,
        consistency: "immutable_demo".into(),
    }]
}

#[test]
fn changes_keep_old_versions_tasks_and_selection_stable_and_can_restore_empty_worksets() {
    let f = Fixture::new();
    let initial = f.collection();
    let old_scope = f.scope(&initial, Some(0));
    let selection = f.store.selection(&f.project.id).unwrap();
    let selection = f
        .store
        .change_selection(
            &f.project.id,
            selection.revision,
            &[f.key(1), f.key(2), f.key(3)],
            &[],
            true,
        )
        .unwrap();
    let added = f.edit(
        &initial,
        CollectionChange::Add {
            input: CollectionMemberInput::Scope {
                scope: ScopeRef {
                    project_id: f.project.id.clone(),
                    target: ScopeTarget::Selection {
                        revision: selection.revision,
                    },
                },
            },
        },
    );
    assert_eq!(
        (
            added.requested,
            added.changed,
            added.collection.count,
            added.collection.revision
        ),
        (3, 2, 4, 1)
    );
    assert_eq!(f.ids(&old_scope), vec![0, 1]);
    let current = f.scope(&added.collection, None);
    let job = f
        .store
        .submit_scope_job(&f.project.id, &new_id(), &current, 0, None)
        .unwrap();
    let pinned = f.store.pin_scope(&f.project.id, &current).unwrap();
    let selected = f
        .store
        .change_selection_scope(
            &f.project.id,
            selection.revision,
            &pinned,
            ScopeOperation::Replace,
        )
        .unwrap();
    let removed = f.edit(
        &added.collection,
        CollectionChange::Remove {
            input: points(vec![f.key(0), f.key(1), f.key(1), f.key(99)]),
        },
    );
    assert_eq!(
        (removed.requested, removed.changed, removed.collection.count),
        (3, 2, 2)
    );
    assert_eq!(f.ids(&current), vec![2, 3]);
    assert_eq!(f.ids(&pinned), vec![0, 1, 2, 3]);
    assert_eq!(
        f.store.selection(&f.project.id).unwrap().count,
        selected.count
    );
    let db = f.store.handle(&f.project.id).unwrap();
    let db = db.read().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM job_inputs WHERE job_id=?1",
            [job.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
    drop(db);
    let empty = f.edit(
        &removed.collection,
        CollectionChange::Remove {
            input: CollectionMemberInput::Scope {
                scope: current.clone(),
            },
        },
    );
    assert_eq!(empty.collection.count, 0);
    assert_eq!(f.store.collections(&f.project.id).unwrap().len(), 1);
    assert!(f.ids(&current).is_empty());
    let restored = f.edit(&empty.collection, CollectionChange::Restore { revision: 0 });
    assert_eq!(restored.collection.count, 2);
    assert_eq!(f.ids(&current), vec![0, 1]);
    assert_eq!(f.values("SELECT count(*) FROM collection_member_legacy"), 2);
    f.store.close(&f.project.id).unwrap();
    let reopened = f.store.open(f.project.directory.clone()).unwrap();
    assert_eq!(reopened.id, f.project.id);
    assert_eq!(f.ids(&pinned), vec![0, 1, 2, 3]);
}

#[test]
fn edits_are_idempotent_fenced_and_atomic_on_source_failure_or_cancellation() {
    let f = Fixture::new();
    let initial = f.collection();
    let request = CollectionEdit {
        request_id: new_id(),
        expected_revision: 0,
        change: CollectionChange::Add {
            input: points(vec![f.key(2), f.key(2)]),
        },
    };
    let first = f
        .store
        .edit_collection(&f.project.id, &initial.id, &request, &mut metadata)
        .unwrap();
    assert_eq!((first.requested, first.changed), (1, 1));
    let noop = f.edit(
        &first.collection,
        CollectionChange::Add {
            input: points(vec![f.key(1), f.key(2)]),
        },
    );
    assert_eq!((noop.changed, noop.collection.revision), (0, 1));
    assert_eq!(
        f.store
            .edit_collection(&f.project.id, &initial.id, &request, &mut |_| panic!(
                "a replay must not read metadata"
            ))
            .unwrap()
            .collection
            .revision,
        1
    );
    let mut stale = request.clone();
    stale.request_id = new_id();
    assert_eq!(
        f.store
            .edit_collection(&f.project.id, &initial.id, &stale, &mut metadata)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let mut conflict = request.clone();
    conflict.expected_revision = 1;
    assert_eq!(
        f.store
            .edit_collection(&f.project.id, &initial.id, &conflict, &mut metadata)
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    let broken = CollectionEdit {
        request_id: new_id(),
        expected_revision: 1,
        change: CollectionChange::Add {
            input: points((100..500).map(|n| f.key(n)).collect()),
        },
    };
    let mut calls = 0;
    let result = f
        .store
        .edit_collection(&f.project.id, &initial.id, &broken, &mut |keys| {
            calls += 1;
            if calls == 2 {
                Err(Error::new("SOURCE_CHANGED", "fixture failure"))
            } else {
                metadata(keys)
            }
        });
    assert_eq!(result.unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(
        f.values("SELECT count(*) FROM collection_member_changes"),
        1
    );
    let cancelled = CollectionEdit {
        request_id: new_id(),
        ..broken.clone()
    };
    let result = f
        .store
        .edit_collection(&f.project.id, &initial.id, &cancelled, &mut |keys| {
            f.store
                .cancel_member_write(&f.project.id, &cancelled.request_id)?;
            metadata(keys)
        });
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    assert_eq!(
        f.store
            .collection(&f.project.id, &initial.id)
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        f.values("SELECT count(*) FROM collection_member_changes"),
        1
    );
    assert_eq!(f.ids(&f.scope(&initial, None)), vec![0, 1, 2]);
}

#[test]
fn edited_ranking_merges_old_scores_and_unranked_additions_in_every_order_and_snapshot() {
    let f = Fixture::new();
    let initial = f.ranking();
    assert_eq!(initial.count, 10);
    let files = ["fixture.ranking.sqlite", "fixture.ranking-input.sqlite"]
        .map(|name| f.project.directory.join("artifacts").join(name));
    let before = files.each_ref().map(|p| fs::read(p).unwrap());
    let added = f.edit(
        &initial,
        CollectionChange::Add {
            input: points(vec![f.key(20), f.key(9999)]),
        },
    );
    let snapshot = f
        .store
        .pin_scope(&f.project.id, &f.scope(&initial, None))
        .unwrap();
    let frozen_job = f
        .store
        .submit_scope_job(&f.project.id, &new_id(), &snapshot, 0, None)
        .unwrap();
    let removed = f.edit(
        &added.collection,
        CollectionChange::Remove {
            input: points(vec![f.key(2)]),
        },
    );
    assert_eq!(removed.collection.count, 11);
    let scope = f.scope(&initial, None);
    assert_eq!(f.ids(&scope), vec![0, 1, 3, 4, 5, 6, 7, 8, 9, 20, 9999]);
    let recipe = f
        .store
        .ranking_projection(&f.project.id, &scope)
        .unwrap()
        .unwrap();
    let reader = RankingProjectionReader::open(
        &f.project.directory,
        &recipe,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(reader.count().unwrap(), 11);
    for order in [
        RankingOrder::Main,
        RankingOrder::Rescue,
        RankingOrder::Direct,
        RankingOrder::Fused,
        RankingOrder::Input,
    ] {
        for descending in [false, true] {
            let mut positions = Vec::new();
            loop {
                let page = reader.page(order, descending, positions.last(), 3).unwrap();
                if page.is_empty() {
                    break;
                }
                positions.extend(page);
                assert!(positions.len() <= 11);
            }
            assert_eq!(positions.len(), 11);
            let ids = positions
                .iter()
                .map(|p| {
                    if p.ordinal >= 1 << 62 {
                        9999
                    } else {
                        p.ordinal
                    }
                })
                .collect::<Vec<_>>();
            if order != RankingOrder::Input {
                assert_eq!(ids.last(), Some(&9999));
            }
            assert!(!ids.contains(&2));
            assert!(ids.contains(&20));
            for (i, p) in positions.iter().enumerate() {
                let located = reader
                    .locate_position(i as u64 + 1, order, descending)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    located.ordinal, p.ordinal,
                    "{order:?}, descending={descending}, position={i}"
                );
                assert_eq!(
                    reader
                        .locate(
                            if p.ordinal >= 1 << 62 {
                                10000
                            } else {
                                p.ordinal as i64 + 1
                            },
                            order,
                            descending
                        )
                        .unwrap()
                        .unwrap()
                        .ordinal,
                    p.ordinal
                );
            }
        }
    }
    assert!(
        reader
            .locate_rank(3, "g", RankingOrder::Main, false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reader
            .locate_rank(21, "g", RankingOrder::Main, false)
            .unwrap()
            .unwrap()
            .ordinal,
        20
    );
    let mut s_only = recipe.clone();
    s_only.restrict_ratings(&["s".into()]);
    let s_reader = RankingProjectionReader::open(
        &f.project.directory,
        &s_only,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(s_reader.count().unwrap(), 1);
    assert!(s_reader.contains_key(&f.key(9999)).unwrap());
    assert!(!s_reader.contains_key(&f.key(20)).unwrap());
    assert_eq!(
        f.ids(&snapshot),
        vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 20, 9999]
    );
    assert_eq!(
        f.values(&format!(
            "SELECT count(*) FROM job_inputs WHERE job_id='{}'",
            frozen_job.id
        )),
        12
    );
    assert_eq!(f.values("SELECT count(*) FROM collection_member_legacy"), 0);
    assert_eq!(
        f.values("SELECT count(*) FROM collection_member_changes"),
        3
    );
    for (path, old) in files.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), old);
    }
}

#[test]
fn query_cache_versions_do_not_alias_equal_counts_and_inflight_queries_keep_their_input() {
    let f = Fixture::new();
    let initial = f.collection();
    let query = spec(&f, f.scope(&initial, None));
    let cache = QueryCacheRequest {
        enabled: true,
        ..Default::default()
    };
    let first = f
        .store
        .create_result_with_cache(&f.project.id, None, query.clone(), versions(&f), &cache)
        .unwrap();
    let second = f
        .edit(
            &initial,
            CollectionChange::Remove {
                input: points(vec![f.key(0)]),
            },
        )
        .collection;
    let third = f
        .edit(
            &second,
            CollectionChange::Add {
                input: points(vec![f.key(2)]),
            },
        )
        .collection;
    assert_eq!(third.count, initial.count);
    assert_eq!(
        f.store
            .query_input_keys(&f.project.id, &first.spec, &f.source.id, None)
            .unwrap(),
        vec![f.key(0), f.key(1)]
    );
    f.store.start_result(&f.project.id, &first.id).unwrap();
    f.store
        .append_result(&f.project.id, &first.id, &[f.key(0), f.key(1)], 2)
        .unwrap();
    f.store
        .finish_result(&f.project.id, &first.id, None)
        .unwrap();
    let next = f
        .store
        .create_result_with_cache(&f.project.id, None, query, versions(&f), &cache)
        .unwrap();
    assert_ne!(
        next.state,
        ResultState::Ready,
        "an old member version must not be reused"
    );
    assert_eq!(
        f.store
            .query_input_keys(&f.project.id, &next.spec, &f.source.id, None)
            .unwrap(),
        vec![f.key(1), f.key(2)]
    );
    assert_eq!(
        f.values("SELECT count(DISTINCT fingerprint) FROM query_families"),
        2
    );
}

#[test]
fn refined_ranked_queries_and_derived_worksets_keep_fixed_members_and_allow_more_additions() {
    let f = Fixture::new();
    let initial = f.ranking();
    let current = f
        .edit(
            &initial,
            CollectionChange::Add {
                input: points(vec![f.key(20), f.key(9999)]),
            },
        )
        .collection;
    let result = f
        .store
        .create_result(
            &f.project.id,
            None,
            spec(&f, f.scope(&current, None)),
            versions(&f),
        )
        .unwrap();
    f.store.start_result(&f.project.id, &result.id).unwrap();
    f.store
        .append_result(&f.project.id, &result.id, &[f.key(0), f.key(9999)], 2)
        .unwrap();
    f.store
        .finish_result(&f.project.id, &result.id, None)
        .unwrap();
    let result_scope = ScopeRef {
        project_id: f.project.id.clone(),
        target: ScopeTarget::QueryResult {
            result_id: result.id.clone(),
        },
    };
    let derived = f
        .store
        .save_scope_collection(&f.project.id, "筛选结果", &result_scope)
        .unwrap();
    let derived = f
        .edit(
            &derived,
            CollectionChange::Add {
                input: points(vec![f.key(20)]),
            },
        )
        .collection;
    let _changed = f.edit(
        &current,
        CollectionChange::Remove {
            input: points(vec![f.key(9999), f.key(0)]),
        },
    );
    for (scope, expected) in [
        (result_scope.clone(), vec![0, 9999]),
        (f.scope(&derived, None), vec![0, 20, 9999]),
    ] {
        let recipe = f
            .store
            .ranking_projection(&f.project.id, &scope)
            .unwrap()
            .unwrap();
        let reader = RankingProjectionReader::open(
            &f.project.directory,
            &recipe,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(reader.count().unwrap(), expected.len() as u64);
        for descending in [false, true] {
            let rows = reader
                .page(RankingOrder::Main, descending, None, 128)
                .unwrap();
            assert_eq!(rows.len(), expected.len());
            assert!(rows.last().unwrap().ordinal >= 1 << 62);
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    reader
                        .locate_position(index as u64 + 1, RankingOrder::Main, descending)
                        .unwrap()
                        .unwrap()
                        .ordinal,
                    row.ordinal
                );
            }
        }
        assert_eq!(f.ids(&scope), expected);
    }
    assert!(
        f.values(&format!(
            "SELECT count(*) FROM collection_version_references WHERE result_id='{}'",
            result.id
        )) > 0
    );
    assert_eq!(
        f.store
            .release_result(&f.project.id, &result.id)
            .unwrap_err()
            .code,
        "RESULT_IN_USE"
    );
}

#[test]
fn adding_a_shared_large_base_only_visits_the_changed_keys() {
    let f = Fixture::new();
    let result = f
        .store
        .create_result(
            &f.project.id,
            None,
            QuerySpec {
                input_scope: None,
                ..spec(
                    &f,
                    ScopeRef {
                        project_id: f.project.id.clone(),
                        target: ScopeTarget::Selection { revision: 0 },
                    },
                )
            },
            versions(&f),
        )
        .unwrap();
    let p = f.store.handle(&f.project.id).unwrap();
    {
        let db = p.db.lock().unwrap();
        db.execute("WITH RECURSIVE n(x) AS(VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<99999) INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from) SELECT ?1,?2,printf('%064x',x),1 FROM n",params![result.id,f.source.id]).unwrap();
        db.execute(
            "UPDATE query_results SET status='ready',count=100000 WHERE id=?1",
            [&result.id],
        )
        .unwrap();
        db.execute(
            "UPDATE query_families SET fixed=1,stored_members=100000 WHERE id=?1",
            [&result.id],
        )
        .unwrap();
    }
    let input = ScopeRef {
        project_id: f.project.id.clone(),
        target: ScopeTarget::QueryResult {
            result_id: result.id.clone(),
        },
    };
    let collection = f
        .store
        .save_scope_collection(&f.project.id, "共享十万成员", &input)
        .unwrap();
    let removed = f
        .edit(
            &collection,
            CollectionChange::Remove {
                input: points(vec![f.key(50000)]),
            },
        )
        .collection;
    let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let db = p.db.lock().unwrap();
        db.execute_batch("CREATE TEMP TABLE collection_edit_candidates(source_id TEXT,asset_id TEXT,present INTEGER,PRIMARY KEY(source_id,asset_id)) WITHOUT ROWID;").unwrap();
        let counter = ticks.clone();
        db.progress_handler(
            100,
            Some(move || counter.fetch_add(1, Ordering::Relaxed) > 100),
        )
        .unwrap();
        let resolved = scopes::resolve(&db, &f.project.id, &input).unwrap();
        assert!(
            shared_base_add(
                &db,
                &f.project.id,
                &collection.id,
                removed.revision,
                &input,
                &resolved.sql,
                &members_sql(&collection.id, removed.revision)
            )
            .unwrap()
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM temp.collection_edit_candidates",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        db.execute_batch("DROP TABLE temp.collection_edit_candidates;")
            .unwrap();
    }
    assert!(
        ticks.load(Ordering::Relaxed) <= 100,
        "unchanged membership must not be enumerated"
    );
    let restored = f.edit(
        &removed,
        CollectionChange::Add {
            input: CollectionMemberInput::Scope { scope: input },
        },
    );
    assert_eq!(
        (
            restored.requested,
            restored.changed,
            restored.collection.count
        ),
        (100000, 1, 100000)
    );
    assert_eq!(f.values("SELECT count(*) FROM collection_member_legacy"), 0);
    assert_eq!(
        f.values("SELECT count(*) FROM collection_member_changes"),
        2
    );
}

#[test]
fn adding_a_ranked_subset_of_the_same_full_material_only_restores_exclusions() {
    let f = Fixture::new();
    let small = f.ranking();
    let recipe = f
        .store
        .ranking_projection(&f.project.id, &f.scope(&small, None))
        .unwrap()
        .unwrap();
    let (input, scores) = recipe.files(&f.project.directory).unwrap();
    let full = f
        .store
        .ranking_workset(
            &f.project.id,
            &recipe.artifact_id,
            &new_id(),
            "全部",
            &RankingFilter::default(),
            (&scores, &input),
        )
        .unwrap();
    let removed = f
        .edit(
            &full,
            CollectionChange::Remove {
                input: points(vec![f.key(2), f.key(50)]),
            },
        )
        .collection;
    let edited = f.edit(
        &removed,
        CollectionChange::Add {
            input: CollectionMemberInput::Scope {
                scope: f.scope(&small, None),
            },
        },
    );
    assert_eq!(
        (edited.requested, edited.changed, edited.collection.count),
        (10, 1, 63)
    );
    let ids = f.ids(&f.scope(&full, None));
    assert!(ids.contains(&2));
    assert!(!ids.contains(&50));
}

#[test]
fn evaluation_intent_pins_members_before_the_prepare_stage_is_materialized() {
    use studio_domain::aesthetic::{AestheticConfig, AestheticCreate};
    use studio_domain::llm::*;
    let f = Fixture::new();
    let initial = f.collection();
    let initial_token = f
        .store
        .evaluation_input(&f.project.id, &initial.id)
        .unwrap()
        .1;
    let first = f
        .edit(
            &initial,
            CollectionChange::Remove {
                input: points(vec![f.key(0)]),
            },
        )
        .collection;
    let frozen = f
        .edit(
            &first,
            CollectionChange::Add {
                input: points(vec![f.key(2)]),
            },
        )
        .collection;
    let (count, token) = f
        .store
        .evaluation_input(&f.project.id, &initial.id)
        .unwrap();
    assert_eq!(count, initial.count);
    assert_ne!(
        token, initial_token,
        "same count must not hide a changed input"
    );
    let request = AestheticCreate {
        idempotency_key: new_id(),
        name: "等待准备".into(),
        collection_id: initial.id.clone(),
        model_id: new_id(),
        system_prompt_id: new_id(),
        overrides: Default::default(),
        exposures: 1,
        max_calls: 1000,
        concurrency: 1,
        expected_input_version: None,
        max_request_mib: None,
        sampling: None,
        execution_policy: None,
        budget_mode: None,
    };
    let config = AestheticConfig {
        version: 1,
        membership_revision: 0,
        model: LlmInvocationSnapshot {
            schema_version: 1,
            invocation_id: new_id(),
            provider_id: new_id(),
            provider_revision: 1,
            provider_kind: LlmProviderKind::OpenaiCompatible,
            base_url: "http://127.0.0.1:1".into(),
            model_id: request.model_id.clone(),
            model_revision: 1,
            remote_model_id: "fixture".into(),
            protocol: LlmProtocol::OpenaiChat,
            preset_id: None,
            preset_revision: None,
            system_prompt_id: Some(request.system_prompt_id.clone()),
            system_prompt_revision: Some(1),
            parameters: Default::default(),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: vec![LlmContent::Text {
                    text: studio_application::aesthetic::OUTPUT_INSTRUCTIONS.into(),
                }],
            }],
            tools: vec![],
            warnings: vec![],
        },
        request,
        sources: versions(&f),
        image_policy: "stored_original_v1".into(),
        grouping_policy: "fixture".into(),
        observation_policy: "meaningful_indifference_v1".into(),
        max_image_bytes: 2 << 20,
        max_request_bytes: 12 << 20,
        execution: None,
    };
    let legacy = serde_json::to_value(&config).unwrap();
    assert!(legacy.get("membership_revision").is_none());
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<AestheticConfig>(legacy.clone()).unwrap())
            .unwrap(),
        legacy
    );
    let intent = f
        .store
        .register_evaluation(&f.project.id, config, &token)
        .unwrap();
    assert_eq!(intent.config.membership_revision, frozen.revision);
    let next = f
        .edit(
            &frozen,
            CollectionChange::Remove {
                input: points(vec![f.key(1)]),
            },
        )
        .collection;
    let next = f
        .edit(
            &next,
            CollectionChange::Add {
                input: points(vec![f.key(3)]),
            },
        )
        .collection;
    assert_eq!(next.count, count);
    let stage = f
        .store
        .materialize_evaluation(&f.project.id, &intent.config.request.idempotency_key)
        .unwrap();
    assert_eq!(stage.state, "preparing");
    assert_eq!(stage.total, 2);
    assert_eq!(
        f.ids(&f.scope(&initial, Some(stage.config.membership_revision))),
        vec![1, 2]
    );
    assert_eq!(f.ids(&f.scope(&initial, None)), vec![2, 3]);
}

#[test]
fn ranked_members_keep_opaque_source_keys_in_small_and_streamed_pages() {
    for count in [64, 8192] {
        let f = Fixture::new();
        let collection = f.ranking_sized(count, None);
        let opaque = ["0", "0aZx", "sample-0001", "参考图"].map(|asset_id| AssetKey {
            source_id: f.source.id.clone(),
            asset_id: asset_id.into(),
        });
        let updated = f
            .edit(
                &collection,
                CollectionChange::Add {
                    input: points(opaque.to_vec()),
                },
            )
            .collection;
        assert_eq!(updated.count, count + opaque.len() as u64);
        let scope = f.scope(&updated, None);
        let result = f
            .store
            .create_ranking_result(
                &f.project.id,
                &spec(&f, scope.clone()),
                None,
                &versions(&f),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap()
            .unwrap();
        let query = ScopeRef {
            project_id: f.project.id.clone(),
            target: ScopeTarget::QueryResult {
                result_id: result.id,
            },
        };
        let mut expected = (0..count)
            .map(|n| f.key(n).asset_id)
            .chain(opaque.iter().map(|k| k.asset_id.clone()))
            .collect::<Vec<_>>();
        expected.sort();
        for descending in [false, true] {
            let mut after = None;
            let mut seen = Vec::new();
            loop {
                let page = f
                    .store
                    .browse_scope_keys(&f.project.id, &query, after.as_ref(), 137, descending)
                    .unwrap();
                if page.is_empty() {
                    break;
                }
                after = page.last().cloned();
                seen.extend(page.into_iter().map(|key| key.asset_id));
                assert!(seen.len() <= expected.len());
            }
            let mut wanted = expected.clone();
            if descending {
                wanted.reverse();
            }
            assert_eq!(seen, wanted);
        }
        assert_eq!(
            f.store
                .filter_browse_scope(&f.project.id, &query, &opaque)
                .unwrap(),
            vec![true; opaque.len()]
        );
        let removed = f.edit(
            &updated,
            CollectionChange::Remove {
                input: points(opaque.to_vec()),
            },
        );
        assert_eq!(removed.collection.count, count);
        assert_eq!(
            f.store
                .filter_browse_scope(&f.project.id, &scope, &opaque)
                .unwrap(),
            vec![false; opaque.len()]
        );
    }
}
