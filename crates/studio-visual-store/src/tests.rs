use super::*;
use studio_domain::{QuerySourceVersion, Source};

fn fixture(root: &Path) -> VisualStore {
    VisualStore::open(root).unwrap()
}
fn temporary() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("visual-store-")
        .tempdir_in(root)
        .unwrap()
}
fn recipe() -> VisualRecipe {
    VisualRecipe {
        id: "a".repeat(64),
        model: "kaloscope3".into(),
        dimension: 256,
        spec: serde_json::json!({"v":1}),
        path: "unused".into(),
        weights: "unused".into(),
    }
}
fn snapshot(id: &str) -> VisualSourceSnapshot {
    VisualSourceSnapshot {
        source: Source {
            id: id.into(),
            name: id.into(),
            kind: "danbooru".into(),
            index_root: None,
            media_root: Some("/lake".into()),
        },
        version: QuerySourceVersion {
            source_id: id.into(),
            catalog_revision: "fixture".into(),
            analysis_sequence: None,
            consistency: "fixed".into(),
            semantics_version: None,
        },
        total_objects: 1000,
        after_row: 0,
        discovered: 0,
        complete: false,
    }
}
fn new_run(store: &VisualStore, sources: &[&str], with_model: bool) -> VisualRun {
    store.register_recipes(&[recipe()]).unwrap();
    store
        .create_run(
            &VisualRunRequest {
                source_ids: sources.iter().map(|s| (*s).into()).collect(),
                recipe_ids: if with_model {
                    vec![recipe().id]
                } else {
                    vec![]
                },
                max_objects_per_source: None,
                batch_size: 8,
            },
            &sources.iter().map(|s| snapshot(s)).collect::<Vec<_>>(),
        )
        .unwrap()
}
fn media(n: u64, offset: u64) -> VisualMediaRef {
    VisualMediaRef {
        row_id: n,
        sha256: format!("{n:064x}"),
        pack: "packs/a.tar".into(),
        offset,
        length: 100,
        extension: "png".into(),
    }
}
fn hash_result(id: i64, value: u64) -> VisualOutcome {
    VisualOutcome {
        object_id: id,
        recipe_id: HASH_RECIPE.into(),
        status: "success".into(),
        hash: Some(format!("{value:016x}")),
        error: None,
    }
}
fn receipt(batch: &VisualBatchRequest, results: Vec<VisualOutcome>) -> VisualBatchReceipt {
    VisualBatchReceipt {
        batch_id: batch.batch_id.clone(),
        object_ids: batch.items.iter().map(|i| i.object_id).collect(),
        results,
        versions: BTreeMap::new(),
        bytes_read: 100,
        elapsed_ms: 1,
        model_images: BTreeMap::new(),
    }
}

#[test]
fn shared_identity_physical_order_and_partial_model_publication() {
    let temp = temporary();
    let store = fixture(temp.path());
    let run = new_run(&store, &["one", "two"], true);
    store
        .discover(
            &run,
            0,
            &VisualMediaPage {
                items: vec![media(1, 900), media(2, 0)],
                complete: true,
            },
        )
        .unwrap();
    store
        .discover(
            &store.run(&run.id).unwrap(),
            1,
            &VisualMediaPage {
                items: vec![media(1, 500)],
                complete: true,
            },
        )
        .unwrap();
    let batch = store
        .next_batch(&store.run(&run.id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(batch.items.len(), 2);
    assert_eq!(batch.items[0].sha256, format!("{:064x}", 2));
    assert_eq!(batch.items[1].source_ids.len(), 2);
    let mut results = batch
        .items
        .iter()
        .map(|r| hash_result(r.object_id, 0))
        .collect::<Vec<_>>();
    for (i, item) in batch.items.iter().enumerate() {
        results.push(VisualOutcome {
            object_id: item.object_id,
            recipe_id: recipe().id,
            status: if i == 0 { "success" } else { "failed" }.into(),
            hash: None,
            error: (i == 1).then(|| "model failure".into()),
        });
    }
    let mut receipt = receipt(&batch, results);
    receipt.versions.insert(recipe().id, 7);
    store.publish(&receipt).unwrap();
    store.publish(&receipt).unwrap(); // lost ACK must not double-count
    let result = store.run(&run.id).unwrap();
    assert_eq!(
        (
            result.planned_objects,
            result.completed_objects,
            result.failed_features,
            result.bytes_read
        ),
        (2, 2, 1, 100)
    );
    assert_eq!(store.version(&recipe().id).unwrap(), 7);
    let groups = store.duplicates(None, 0, 10, 0).unwrap().0;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].assets.len(), 2);
    let repeated = run_fn(&store);
    let batch = store
        .next_batch(&store.run(&repeated.id).unwrap())
        .unwrap()
        .unwrap();
    assert!(batch.items[0].hash_done);
    fn run_fn(store: &VisualStore) -> VisualRun {
        let next = new_run(store, &["two"], true);
        store
            .discover(
                &next,
                0,
                &VisualMediaPage {
                    items: vec![media(1, 1)],
                    complete: true,
                },
            )
            .unwrap();
        next
    }
}

#[test]
fn restart_keeps_intent_and_rejects_incomplete_receipts() {
    let temp = temporary();
    let store = fixture(temp.path());
    assert!(VisualStore::open(temp.path()).is_err());
    let run = new_run(&store, &["one"], true);
    store
        .discover(
            &run,
            0,
            &VisualMediaPage {
                items: vec![media(1, 0)],
                complete: true,
            },
        )
        .unwrap();
    let batch = store
        .next_batch(&store.run(&run.id).unwrap())
        .unwrap()
        .unwrap();
    let invalid = receipt(&batch, vec![hash_result(batch.items[0].object_id, 0)]);
    assert_eq!(store.publish(&invalid).unwrap_err().code, "VISUAL_PROTOCOL");
    assert_eq!(
        store
            .coverage()
            .unwrap()
            .iter()
            .map(|c| c.succeeded)
            .sum::<u64>(),
        0
    );
    drop(store);
    let store = fixture(temp.path());
    assert_eq!(store.run(&run.id).unwrap().status, "paused");
    assert_eq!(
        store.pending_batch(&run.id).unwrap().unwrap().batch_id,
        batch.batch_id
    );
    let mut good = invalid;
    good.results.push(VisualOutcome {
        object_id: batch.items[0].object_id,
        recipe_id: recipe().id,
        status: "success".into(),
        hash: None,
        error: None,
    });
    good.versions.insert(recipe().id, 3);
    store.publish(&good).unwrap();
    assert!(
        store
            .next_batch(&store.run(&run.id).unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn complete_bindings_are_skipped_before_media_planning() {
    let temp = temporary();
    let store = fixture(temp.path());
    let first = new_run(&store, &["one"], false);
    assert!(store.has_unfinished().unwrap());
    assert_eq!(
        store
            .next_batch_with_cancel(&first, &std::sync::atomic::AtomicBool::new(true))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let page = VisualMediaPage {
        items: vec![media(1, 0)],
        complete: true,
    };
    store.discover(&first, 0, &page).unwrap();
    let batch = store
        .next_batch(&store.run(&first.id).unwrap())
        .unwrap()
        .unwrap();
    store
        .publish(&receipt(
            &batch,
            vec![hash_result(batch.items[0].object_id, 0)],
        ))
        .unwrap();
    let repeated = new_run(&store, &["one"], false);
    store.discover(&repeated, 0, &page).unwrap();
    let repeated = store.run(&repeated.id).unwrap();
    assert_eq!((repeated.planned_objects, repeated.reused_bindings), (0, 1));
    assert!(store.next_batch(&repeated).unwrap().is_none());
}

#[test]
fn phash_radius_seven_is_complete_across_pages() {
    let temp = temporary();
    let store = fixture(temp.path());
    let run = new_run(&store, &["one"], false);
    let items = (1..=65).map(|n| media(n, n * 100)).collect();
    store
        .discover(
            &run,
            0,
            &VisualMediaPage {
                items,
                complete: true,
            },
        )
        .unwrap();
    let mut expected = BTreeSet::new();
    loop {
        let Some(batch) = store.next_batch(&store.run(&run.id).unwrap()).unwrap() else {
            break;
        };
        let results = batch
            .items
            .iter()
            .map(|i| {
                let n = u64::from_str_radix(&i.sha256, 16).unwrap();
                let bits = if n == 1 {
                    0
                } else {
                    let count = (n % 9) as u32;
                    (0..count).fold(0, |b, k| b | (1u64 << ((n + k as u64 * 8) % 64)))
                };
                if n != 1 && bits.count_ones() <= 7 {
                    expected.insert(i.object_id);
                }
                hash_result(i.object_id, bits)
            })
            .collect();
        store.publish(&receipt(&batch, results)).unwrap();
    }
    let key = AssetKey {
        source_id: "one".into(),
        asset_id: format!("{:064x}", 1),
    };
    let mut actual = BTreeSet::new();
    let mut after = 0;
    loop {
        let (items, next) = store.duplicates(Some(&key), after, 3, 7).unwrap();
        for item in items {
            assert!(actual.insert(item.object_id));
        }
        let Some(next) = next else {
            break;
        };
        assert!(next > after);
        after = next;
    }
    assert_eq!(actual, expected);
    assert!(store.duplicates(Some(&key), 0, 3, 8).is_err());
}
