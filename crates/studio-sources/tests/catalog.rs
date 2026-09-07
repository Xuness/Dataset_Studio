use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::fs;
use studio_application::SourceAdapter;
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
