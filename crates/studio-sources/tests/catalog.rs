use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::fs;
use studio_application::{MediaInput, MediaSource, SourceAdapter};
use studio_domain::{AssetKey, Source, new_id};
use studio_sources::SourceRouter;
fn fixture() -> (tempfile::TempDir, Source, String, Vec<u8>) {
    let root = tempfile::tempdir().unwrap();
    let index = root.path().join("索引");
    let media = root.path().join("图片");
    fs::create_dir_all(index.join("indexes/gen-1")).unwrap();
    fs::create_dir_all(media.join("segments/a")).unwrap();
    let id = new_id();
    fs::write(
        index.join("CURRENT.json"),
        serde_json::json!({"library_id":id,"index_version":1,"generation":"gen-1"}).to_string(),
    )
    .unwrap();
    fs::write(media.join("library.json"),serde_json::json!({"library_id":id,"format_version":1,"image_format":"uncompressed-pax-tar"}).to_string()).unwrap();
    let bytes = b"deterministic image payload".to_vec();
    let hash = hex::encode(Sha256::digest(&bytes));
    let mut pack = vec![0; 512];
    pack.extend_from_slice(&bytes);
    fs::write(media.join("segments/a/images.tar"), pack).unwrap();
    let db = Connection::open(index.join("indexes/gen-1/catalog.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID;CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER);INSERT INTO state VALUES ('seq',1);").unwrap();
    db.execute(
        "INSERT INTO objects VALUES (?1,'segments/a/images.tar',512,?2,'png')",
        params![hash, bytes.len() as i64],
    )
    .unwrap();
    drop(db);
    (
        root,
        Source {
            id,
            name: "测试归档".into(),
            kind: "danbooru".into(),
            index_root: Some(index),
            media_root: Some(media),
        },
        hash,
        bytes,
    )
}
#[test]
fn reads_only_indexed_byte_range_and_verifies_content() {
    let (_root, source, id, bytes) = fixture();
    let page = SourceRouter.page(&source, None, 1, None).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(SourceRouter.read(&source, &id).unwrap().bytes, bytes);
    let frozen = SourceRouter
        .freeze(
            &source,
            &[AssetKey {
                source_id: source.id.clone(),
                asset_id: id.clone(),
            }],
        )
        .unwrap();
    assert_eq!(frozen[0].source_revision, page.revision);
    assert!(SourceRouter.read(&source, "../outside").is_err());
}
#[test]
fn detects_changed_revision_and_rejects_pack_path_escape() {
    let (_root, source, id, _) = fixture();
    let page = SourceRouter.page(&source, None, 1, None).unwrap();
    let db = Connection::open(
        source
            .index_root
            .as_ref()
            .unwrap()
            .join("indexes/gen-1/catalog.sqlite"),
    )
    .unwrap();
    db.execute("UPDATE state SET value=2 WHERE key='seq'", [])
        .unwrap();
    assert_eq!(
        SourceRouter
            .page(&source, None, 1, Some(&page.revision))
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
    db.execute(
        "UPDATE objects SET pack_path='../outside' WHERE sha256=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(
        SourceRouter.read(&source, &id).err().unwrap().code,
        "SOURCE_PATH_INVALID"
    );
}
#[test]
fn rejects_mismatched_index_and_media_identity() {
    let (_root, mut source, _, _) = fixture();
    source.id = new_id();
    assert_eq!(
        SourceRouter.probe(&source).unwrap_err().code,
        "SOURCE_ID_MISMATCH"
    );
}
#[test]
fn batches_read_pack_offsets_once_and_restore_logical_order() {
    use std::sync::{Arc, atomic::AtomicBool};
    let (_temp, source, original, _) = fixture();
    let media = source.media_root.as_ref().unwrap();
    let catalog = source
        .index_root
        .as_ref()
        .unwrap()
        .join("indexes/gen-1/catalog.sqlite");
    let db = Connection::open(catalog).unwrap();
    db.execute("DELETE FROM objects", []).unwrap();
    let mut entries = Vec::new();
    for (pack, offset, text) in [
        ("segments/a/first.tar", 512, "one"),
        ("segments/a/first.tar", 1024, "two"),
        ("segments/a/second.tar", 512, "three"),
    ] {
        let path = media.join(pack);
        let mut bytes = fs::read(&path).unwrap_or_default();
        bytes.resize(offset + text.len(), 0);
        bytes[offset..].copy_from_slice(text.as_bytes());
        fs::write(path, bytes).unwrap();
        let id = hex::encode(Sha256::digest(text.as_bytes()));
        db.execute(
            "INSERT INTO objects VALUES(?1,?2,?3,?4,'png')",
            params![id, pack, offset as i64, text.len() as i64],
        )
        .unwrap();
        entries.push((id, text));
    }
    let inputs = [2, 1, 0].map(|i| MediaInput {
        deadline: None,
        asset_id: entries[i].0.clone(),
        cancelled: Arc::new(AtomicBool::new(false)),
        byte_limit: 64 << 20,
    });
    let result = SourceRouter.read_many(&source, &inputs).unwrap();
    assert_eq!(result.stats.opens, 2);
    assert_eq!(result.stats.seeks, 3);
    assert_eq!(result.stats.bytes, 11);
    assert_eq!(
        result
            .stats
            .trace
            .iter()
            .map(|v| (&v.asset_id, v.offset))
            .collect::<Vec<_>>(),
        vec![
            (&entries[0].0, 512),
            (&entries[1].0, 1024),
            (&entries[2].0, 512)
        ]
    );
    assert_eq!(
        result
            .items
            .into_iter()
            .map(|r| r.unwrap().bytes)
            .collect::<Vec<_>>(),
        vec![b"three".to_vec(), b"two".to_vec(), b"one".to_vec()]
    );
    let inputs = entries
        .iter()
        .map(|(id, _)| MediaInput {
            deadline: None,
            asset_id: id.clone(),
            cancelled: Arc::new(AtomicBool::new(true)),
            byte_limit: 64 << 20,
        })
        .collect::<Vec<_>>();
    let stopped = SourceRouter.read_many(&source, &inputs).unwrap();
    assert_eq!(stopped.stats.opens, 0);
    assert_eq!(stopped.stats.bytes, 0);
    assert_eq!(stopped.stats.cancelled, 3);
    assert!(
        SourceRouter
            .verify_media_identity(&source, &original)
            .is_err()
    );
}
#[test]
fn an_index_length_change_cannot_exceed_the_admitted_payload_budget() {
    use std::sync::{Arc, atomic::AtomicBool};
    let (_temp, source, id, bytes) = fixture();
    let result = SourceRouter
        .read_many(
            &source,
            &[MediaInput {
                deadline: None,
                asset_id: id,
                cancelled: Arc::new(AtomicBool::new(false)),
                byte_limit: bytes.len() as u64 - 1,
            }],
        )
        .unwrap();
    assert_eq!(
        result.items[0].as_ref().err().unwrap().code,
        "READ_BUDGET_EXCEEDED"
    );
    assert_eq!(result.stats.bytes, 0);
    assert_eq!(result.stats.opens, 0);
}
