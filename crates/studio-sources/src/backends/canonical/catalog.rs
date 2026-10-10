use crate::profiles;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};
use studio_application::{Media, MediaBatch, MediaInput, SourceProbe};
use studio_domain::*;

#[derive(Deserialize)]
struct Current {
    library_id: String,
    index_version: u32,
    generation: String,
}

#[cfg(test)]
mod container_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicBool};

    fn fixture() -> (tempfile::TempDir, Catalog, Connection, Vec<String>) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        fs::create_dir_all(root.join("segments/one")).unwrap();
        fs::create_dir_all(root.join("segments/two")).unwrap();
        fs::create_dir(root.join("containers")).unwrap();
        fs::write(root.join("library.json"), serde_json::json!({
            "library_id":"test-library","format_version":3,"site":"pinterest","schema_set":"pinterest-media-v1",
            "required_features":["pinterest-pins-v1","raw-captures-v1","receipt-replay-v1","sha256-objects-v1","receipt-bundles-v1","archive-containers-v1"]
        }).to_string()).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT)").unwrap();
        let journal = Connection::open(root.join("journal.sqlite")).unwrap();
        journal.execute_batch("CREATE TABLE archive_containers(container_id TEXT PRIMARY KEY,path TEXT,state TEXT,bytes INTEGER); CREATE TABLE archive_members(path TEXT PRIMARY KEY,container_id TEXT,offset INTEGER,length INTEGER);INSERT INTO archive_containers VALUES('c','containers/c.tar','sealed',4096)").unwrap();
        let mut container = vec![0; 4096];
        let mut ids = Vec::new();
        for (index, name) in ["one", "two"].iter().enumerate() {
            let pack = format!("segments/{name}/media.tar");
            let mut original = vec![0; 1024];
            original[512..515].copy_from_slice(name.as_bytes());
            fs::write(root.join(&pack), &original).unwrap();
            let base = (index + 1) * 1024;
            container[base..base + 1024].copy_from_slice(&original);
            let sha = hex::encode(Sha256::digest(name.as_bytes()));
            db.execute(
                "INSERT INTO objects VALUES(?1,?2,512,3,'png')",
                params![sha, pack],
            )
            .unwrap();
            journal
                .execute(
                    "INSERT INTO archive_members VALUES(?1,'c',?2,1024)",
                    params![pack, base as i64],
                )
                .unwrap();
            ids.push(sha);
        }
        fs::write(root.join("containers/c.tar"), container).unwrap();
        let catalog = Catalog {
            db,
            revision: "fixed-version".into(),
            library_id: "test-library".into(),
            root: root.clone(),
            index: root.clone(),
            generation_path: root,
            generation: "unchanged".into(),
            sequence: 3,
            online: true,
            format_version: 4,
            count: Some(2),
        };
        (directory, catalog, journal, ids)
    }

    #[test]
    fn pinterest_container_switch_preserves_an_open_catalog_and_orders_physical_reads() {
        let (_directory, catalog, journal, ids) = fixture();
        assert_eq!(catalog.read(&ids[0]).unwrap().bytes, b"one");
        journal
            .execute("UPDATE archive_containers SET state='active'", [])
            .unwrap();
        for name in ["one", "two"] {
            fs::remove_file(catalog.root.join(format!("segments/{name}/media.tar"))).unwrap();
        }
        assert_eq!(catalog.read(&ids[0]).unwrap().bytes, b"one");
        let inputs = [1, 0].map(|i| MediaInput {
            asset_id: ids[i].clone(),
            byte_limit: 3,
            deadline: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
        let result = catalog.read_many(&inputs).unwrap();
        assert_eq!(result.stats.opens, 1);
        assert_eq!(
            result
                .stats
                .trace
                .iter()
                .map(|v| v.offset)
                .collect::<Vec<_>>(),
            vec![1536, 2560]
        );
        assert_eq!(
            result
                .items
                .into_iter()
                .map(|r| r.unwrap().bytes)
                .collect::<Vec<_>>(),
            vec![b"two".to_vec(), b"one".to_vec()]
        );
        assert_eq!(catalog.revision, "fixed-version");
    }

    #[test]
    fn pinterest_reclamation_between_resolve_and_open_retries_the_published_mapping() {
        let (_directory, catalog, journal, _) = fixture();
        let (mut pack, mut offset) = catalog
            .resolve_pack("segments/one/media.tar", 512, 3)
            .unwrap();
        journal
            .execute("UPDATE archive_containers SET state='cleaned'", [])
            .unwrap();
        fs::remove_file(catalog.root.join(&pack)).unwrap();
        let mut file = catalog.open_pack(&mut pack, &mut offset, 3).unwrap();
        assert_eq!(offset, 1536);
        file.seek(SeekFrom::Start(offset)).unwrap();
        let mut bytes = [0; 3];
        file.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"one");
    }

    #[test]
    fn pinterest_container_rejects_escaping_ranges_and_paths() {
        let (_directory, catalog, journal, ids) = fixture();
        journal
            .execute("UPDATE archive_containers SET state='active'", [])
            .unwrap();
        journal
            .execute("UPDATE archive_members SET length=514", [])
            .unwrap();
        assert_eq!(catalog.read(&ids[0]).err().unwrap().code, "SOURCE_CORRUPT");
        journal
            .execute("UPDATE archive_members SET length=1024", [])
            .unwrap();
        journal
            .execute("UPDATE archive_containers SET path='../outside'", [])
            .unwrap();
        assert_eq!(
            catalog.read(&ids[0]).err().unwrap().code,
            "SOURCE_PATH_INVALID"
        );
    }
}
#[derive(Deserialize)]
struct Library {
    library_id: String,
    format_version: u32,
    image_format: String,
}

fn unsigned(row: &rusqlite::Row, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
pub(crate) fn err(e: rusqlite::Error) -> Error {
    Error::new(
        if matches!(e,rusqlite::Error::SqliteFailure(ref code,_) if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked))
        {
            "SOURCE_BUSY"
        } else {
            "SOURCE_FORMAT_ERROR"
        },
        e.to_string(),
    )
}
fn child(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(Error::io)?;
    let path = Path::new(relative);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::new(
            "SOURCE_PATH_INVALID",
            "数据包路径必须是相对路径",
        ));
    }
    let resolved = root.join(path).canonicalize().map_err(Error::io)?;
    if !resolved.starts_with(&root) {
        return Err(Error::new(
            "SOURCE_PATH_INVALID",
            "数据包超出已登记的数据湖目录",
        ));
    }
    Ok(resolved)
}
pub struct Catalog {
    db: Connection,
    pub revision: String,
    pub library_id: String,
    root: PathBuf,
    index: PathBuf,
    pub generation_path: PathBuf,
    pub generation: String,
    pub sequence: u64,
    pub online: bool,
    pub format_version: u32,
    count: Option<u64>,
}
impl Catalog {
    fn resolve_pack(&self, pack: &str, offset: u64, length: u64) -> Result<(String, u64)> {
        if self.format_version != 4 {
            return Ok((pack.into(), offset));
        }
        let info: serde_json::Value =
            serde_json::from_slice(&fs::read(self.root.join("library.json")).map_err(Error::io)?)
                .map_err(Error::io)?;
        crate::pinterest_metadata::validate_archive_format(&info)?;
        if info["library_id"].as_str() != Some(&self.library_id) || info["format_version"] != 3 {
            return Err(Error::new("SOURCE_CORRUPT", "Pinterest 归档身份已改变"));
        }
        if !info["required_features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|x| x == "archive-containers-v1"))
        {
            return Ok((pack.into(), offset));
        }
        let db = Connection::open_with_flags(
            self.root.join("journal.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        let row: Option<(String, u64, u64, u64)> = db.query_row(
            "SELECT c.path,m.offset,m.length,c.bytes FROM archive_members m JOIN archive_containers c USING(container_id) WHERE m.path=?1 AND c.state IN ('active','cleaned')",
            [pack], |r| Ok((r.get(0)?, unsigned(r,1)?, unsigned(r,2)?, unsigned(r,3)?))
        ).optional().map_err(err)?;
        let Some((container, base, member_length, container_length)) = row else {
            return Ok((pack.into(), offset));
        };
        if offset
            .checked_add(length)
            .is_none_or(|end| end > member_length)
            || base
                .checked_add(member_length)
                .is_none_or(|end| end > container_length)
            || fs::metadata(child(&self.root, &container)?)
                .map_err(Error::io)?
                .len()
                != container_length
        {
            return Err(Error::new("SOURCE_CORRUPT", "Pinterest 容器范围校验失败"));
        }
        Ok((container, base + offset))
    }

    fn open_pack(&self, pack: &mut String, offset: &mut u64, length: u64) -> Result<File> {
        let first = child(&self.root, pack).and_then(|path| File::open(path).map_err(Error::io));
        match first {
            Ok(file) => Ok(file),
            Err(original) => {
                // Activation can occur between resolving a loose path and opening it.
                let (next, next_offset) = self.resolve_pack(pack, *offset, length)?;
                if next == *pack {
                    return Err(original);
                }
                let file = File::open(child(&self.root, &next)?).map_err(Error::io)?;
                *pack = next;
                *offset = next_offset;
                Ok(file)
            }
        }
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.db
    }
    pub fn open(source: &Source) -> Result<Self> {
        Self::open_at(source, None)
    }
    pub fn open_at(source: &Source, revision: Option<&str>) -> Result<Self> {
        if crate::online::available(source) {
            let snapshot = crate::online::Snapshot::open(
                source,
                revision,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                None,
            )?;
            let root = source
                .media_root
                .as_ref()
                .ok_or_else(|| Error::invalid("缺少图片湖目录"))?
                .clone();
            snapshot.db.execute_batch("CREATE TEMP VIEW objects AS SELECT sha256,pack_path,offset,length,stored_ext FROM visible_objects").map_err(err)?;
            return Ok(Self {
                db: snapshot.db,
                revision: snapshot.revision,
                library_id: snapshot.pointer.library_id,
                root,
                index: source.index_root.clone().expect("validated root"),
                generation_path: snapshot.path.parent().expect("database parent").into(),
                generation: snapshot.pointer.generation,
                sequence: snapshot.sequence,
                online: true,
                format_version: snapshot.pointer.schema_version,
                count: Some(snapshot.count),
            });
        }
        let index = source
            .index_root
            .as_ref()
            .ok_or_else(|| Error::invalid("缺少索引目录"))?
            .canonicalize()
            .map_err(Error::io)?;
        let root = source
            .media_root
            .as_ref()
            .ok_or_else(|| Error::invalid("缺少图片湖目录"))?
            .canonicalize()
            .map_err(Error::io)?;
        let current: Current =
            serde_json::from_slice(&fs::read(index.join("CURRENT.json")).map_err(Error::io)?)
                .map_err(Error::io)?;
        let library: Library =
            serde_json::from_slice(&fs::read(root.join("library.json")).map_err(Error::io)?)
                .map_err(Error::io)?;
        if current.library_id != library.library_id
            || (!source.id.is_empty() && source.id != library.library_id)
        {
            return Err(Error::new(
                "SOURCE_ID_MISMATCH",
                "索引和图片湖的逻辑身份不一致",
            ));
        }
        if current.index_version != 1
            || library.format_version != 1
            || library.image_format != "uncompressed-pax-tar"
        {
            return Err(Error::new(
                "SOURCE_FORMAT_UNSUPPORTED",
                "不支持该数据湖存储格式",
            ));
        }
        profiles::validate_site(&root, &source.kind)?;
        let generation = child(
            &index.join("indexes").canonicalize().map_err(Error::io)?,
            &current.generation,
        )?;
        let dbpath = child(&generation, "catalog.sqlite")?;
        let db = Connection::open_with_flags(
            dbpath,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(err)?;
        db.busy_timeout(std::time::Duration::from_millis(400))
            .map_err(err)?;
        db.execute_batch("PRAGMA query_only=ON; PRAGMA mmap_size=2147483648; BEGIN;")
            .map_err(err)?;
        let seq: u64 = db
            .query_row("SELECT value FROM state WHERE key='seq'", [], |r| {
                unsigned(r, 0)
            })
            .map_err(err)?;
        let revision = format!("catalog-v1:{}:{seq}", current.generation);
        Ok(Self {
            db,
            revision,
            library_id: library.library_id,
            root,
            index,
            generation_path: generation,
            generation: current.generation,
            sequence: seq,
            online: false,
            format_version: 1,
            count: None,
        })
    }
    pub fn validate_media(&self, source: &Source) -> Result<()> {
        if self.online {
            let root = self.root.canonicalize().map_err(Error::io)?;
            let library: Library = serde_json::from_slice(
                &fs::read(child(&root, "library.json")?).map_err(Error::io)?,
            )
            .map_err(Error::io)?;
            if library.library_id != self.library_id
                || library.format_version
                    != match self.format_version {
                        4 => 3,
                        3 => 2,
                        _ => 1,
                    }
                || library.image_format != "uncompressed-pax-tar"
            {
                return Err(Error::new(
                    "SOURCE_ID_MISMATCH",
                    "在线索引和图片湖身份不一致",
                ));
            }
            profiles::validate_site(&root, &source.kind)?;
        }
        Ok(())
    }
    pub fn analysis_path(&self) -> Result<PathBuf> {
        if self.online {
            return Err(Error::new(
                "QUERY_UNSUPPORTED",
                "在线来源的分析任务需要冻结投影输入",
            ));
        }
        child(&self.generation_path, "analysis.duckdb")
    }
    pub fn verify_unchanged(&self, source: &Source) -> Result<()> {
        if self.online {
            Self::open_at(source, Some(&self.revision))?;
            return Ok(());
        }
        let pointer: Current =
            serde_json::from_slice(&fs::read(self.index.join("CURRENT.json")).map_err(Error::io)?)
                .map_err(Error::io)?;
        if pointer.library_id != self.library_id
            || pointer.generation != self.generation
            || pointer.index_version != 1
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "读取过程中数据源切换了索引版本，请刷新元数据",
            ));
        }
        // A fresh connection supplies an end fence; the original transaction remains open.
        let current = Self::open(source)?;
        if current.revision != self.revision || current.library_id != self.library_id {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "读取过程中数据源已更新，请刷新元数据",
            ));
        }
        Ok(())
    }
    pub fn probe(&self) -> SourceProbe {
        SourceProbe {
            id: self.library_id.clone(),
            revision: self.revision.clone(),
            enumeration: "stored_objects".into(),
            count: self.count,
            index_version: self.format_version,
        }
    }
    pub fn page(
        &self,
        source: &Source,
        after: Option<&str>,
        limit: usize,
        revision: Option<&str>,
    ) -> Result<AssetPage> {
        self.page_ordered(source, after, limit, revision, false)
    }
    pub fn page_ordered(
        &self,
        source: &Source,
        after: Option<&str>,
        limit: usize,
        revision: Option<&str>,
        descending: bool,
    ) -> Result<AssetPage> {
        if revision.is_some_and(|r| r != self.revision) {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "索引版本已变化，请刷新结果范围",
            ));
        }
        let op = if descending { "<" } else { ">" };
        let direction = if descending { "DESC" } else { "ASC" };
        let mut stmt=self.db.prepare(&format!("SELECT sha256,length,stored_ext FROM objects WHERE sha256{op}?1 ORDER BY sha256 {direction} LIMIT ?2")).map_err(err)?;
        let mut items = stmt
            .query_map(
                params![
                    after.unwrap_or(if descending { "z" } else { "" }),
                    (limit + 1) as i64
                ],
                |r| {
                    let id: String = r.get(0)?;
                    Ok(Asset {
                        key: AssetKey {
                            source_id: source.id.clone(),
                            asset_id: id.clone(),
                        },
                        name: id,
                        bytes: unsigned(r, 1)?,
                        extension: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        source_name: source.name.clone(),
                    })
                },
            )
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().map(|a| a.key.asset_id.clone())
        } else {
            None
        };
        Ok(AssetPage {
            items,
            next,
            revision: self.revision.clone(),
        })
    }
    pub fn asset(&self, source: &Source, id: &str) -> Result<Asset> {
        self.db
            .query_row(
                "SELECT length,stored_ext FROM objects WHERE sha256=?1",
                [id],
                |r| {
                    Ok(Asset {
                        key: AssetKey {
                            source_id: source.id.clone(),
                            asset_id: id.to_owned(),
                        },
                        name: id.to_owned(),
                        bytes: unsigned(r, 0)?,
                        extension: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        source_name: source.name.clone(),
                    })
                },
            )
            .optional()
            .map_err(err)?
            .ok_or_else(|| Error::new("NOT_FOUND", "图片对象不在此数据湖中"))
    }
    pub fn read(&self, id: &str) -> Result<Media> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::invalid(
                "图片身份需要是 64 个十六进制字符的 SHA-256 值",
            ));
        }
        let location:Option<(String,u64,u64,String)>=self.db.query_row("SELECT pack_path,offset,length,COALESCE(stored_ext,'') FROM objects WHERE sha256=?1",[id],|r|Ok((r.get(0)?,unsigned(r,1)?,unsigned(r,2)?,r.get(3)?))).optional().map_err(err)?;
        let (pack, offset, length, extension) =
            location.ok_or_else(|| Error::new("NOT_FOUND", "图片对象不存在"))?;
        if length > 64 * 1024 * 1024 {
            return Err(Error::new(
                "MEDIA_TOO_LARGE",
                "首版预览支持最大 64 MiB 的单张图片",
            ));
        }
        let (mut pack, mut offset) = self.resolve_pack(&pack, offset, length)?;
        let mut file = self.open_pack(&mut pack, &mut offset, length)?;
        if offset
            .checked_add(length)
            .is_none_or(|end| end > file.metadata().map(|m| m.len()).unwrap_or(0))
        {
            return Err(Error::new("SOURCE_CORRUPT", "图片位置超出数据包边界"));
        }
        file.seek(SeekFrom::Start(offset)).map_err(Error::io)?;
        let mut bytes = vec![0; length as usize];
        file.read_exact(&mut bytes).map_err(Error::io)?;
        if hex::encode(Sha256::digest(&bytes)) != id {
            return Err(Error::new("SOURCE_CORRUPT", "图片内容校验失败"));
        }
        let content_type = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "avif" => "image/avif",
            _ => "application/octet-stream",
        }
        .to_owned();
        Ok(Media {
            bytes,
            content_type,
        })
    }
    pub fn read_many(&self, inputs: &[MediaInput]) -> Result<MediaBatch> {
        if inputs.len() > 16 {
            return Err(Error::invalid("媒体批次最多 16 个对象"));
        }
        let mut items: Vec<Option<Result<Media>>> = (0..inputs.len()).map(|_| None).collect();
        let mut locations = Vec::<(usize, String, u64, u64, String)>::new();
        let mut total = 0u64;
        for (i, input) in inputs.iter().enumerate() {
            let location: Option<(String,u64,u64,String)> = self.db.query_row(
                "SELECT pack_path,offset,length,COALESCE(stored_ext,'') FROM objects WHERE sha256=?1",
                [&input.asset_id], |r| Ok((r.get(0)?,unsigned(r,1)?,unsigned(r,2)?,r.get(3)?))
            ).optional().map_err(err)?;
            match location {
                Some((_, _, length, _)) if length > input.byte_limit => {
                    items[i] = Some(Err(Error::new(
                        "READ_BUDGET_EXCEEDED",
                        "索引中的图片长度超过已准入的读取预算",
                    )));
                }
                Some((pack, offset, length, extension)) if length <= 64 << 20 => {
                    total += length;
                    match self.resolve_pack(&pack, offset, length) {
                        Ok((pack, offset)) => locations.push((i, pack, offset, length, extension)),
                        Err(error) => items[i] = Some(Err(error)),
                    }
                }
                Some(_) => {
                    items[i] = Some(Err(Error::new(
                        "MEDIA_TOO_LARGE",
                        "预览支持最大 64 MiB 的单张图片",
                    )))
                }
                None => items[i] = Some(Err(Error::new("NOT_FOUND", "图片对象不存在"))),
            }
        }
        if total > 64 << 20 {
            return Err(Error::new(
                "READ_BUDGET_EXCEEDED",
                "一个媒体批次最多读取 64 MiB",
            ));
        }
        locations.sort_by(|a, b| (&a.1, a.2, a.0).cmp(&(&b.1, b.2, b.0)));
        let mut current: Option<(String, File)> = None;
        let mut stats = PhysicalReadStats::default();
        for (index, mut pack, mut offset, length, extension) in locations {
            let input = &inputs[index];
            let mut read = 0u64;
            let result = (|| -> Result<Media> {
                input.check()?;
                if current.as_ref().is_none_or(|(name, _)| name != &pack) {
                    input.check()?;
                    let file = self.open_pack(&mut pack, &mut offset, length)?;
                    current = Some((pack.clone(), file));
                    stats.opens += 1;
                }
                let file = &mut current.as_mut().expect("opened pack").1;
                if offset
                    .checked_add(length)
                    .is_none_or(|end| end > file.metadata().map(|m| m.len()).unwrap_or(0))
                {
                    return Err(Error::new("SOURCE_CORRUPT", "图片位置超出数据包边界"));
                }
                input.check()?;
                file.seek(SeekFrom::Start(offset)).map_err(Error::io)?;
                stats.seeks += 1;
                let mut bytes = vec![0; length as usize];
                for chunk in bytes.chunks_mut(64 * 1024) {
                    let mut filled = 0;
                    while filled < chunk.len() {
                        input.check()?;
                        let count = file.read(&mut chunk[filled..]).map_err(Error::io)?;
                        if count == 0 {
                            return Err(Error::new("SOURCE_CORRUPT", "数据包读取提前结束"));
                        }
                        filled += count;
                        read += count as u64;
                        stats.bytes += count as u64;
                    }
                }
                input.check()?;
                if hex::encode(Sha256::digest(&bytes)) != input.asset_id {
                    return Err(Error::new("SOURCE_CORRUPT", "图片内容校验失败"));
                }
                let content_type = match extension.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "gif" => "image/gif",
                    "avif" => "image/avif",
                    _ => "application/octet-stream",
                }
                .into();
                Ok(Media {
                    bytes,
                    content_type,
                })
            })();
            if read > 0 {
                stats.trace.push(PhysicalReadTrace {
                    asset_id: input.asset_id.clone(),
                    pack,
                    offset,
                    bytes: read,
                });
            }
            if result.as_ref().is_err_and(|e| e.code == "CANCELLED") {
                stats.cancelled += 1;
                stats.cancelled_before_read += u64::from(read == 0);
            }
            items[index] = Some(result);
        }
        Ok(MediaBatch {
            items: items
                .into_iter()
                .map(|v| v.expect("each input assigned once"))
                .collect(),
            stats,
        })
    }
}
