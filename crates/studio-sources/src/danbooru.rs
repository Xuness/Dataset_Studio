use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};
use studio_application::{Media, MediaBatch, MediaInput, SourceProbe, read_cancelled};
use studio_domain::*;

#[derive(Deserialize)]
struct Current {
    library_id: String,
    index_version: u32,
    generation: String,
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
    if !resolved.starts_with(root) {
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
}
impl Catalog {
    pub(crate) fn connection(&self) -> &Connection {
        &self.db
    }
    pub fn open(source: &Source) -> Result<Self> {
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
        })
    }
    pub fn analysis_path(&self) -> Result<PathBuf> {
        child(&self.generation_path, "analysis.duckdb")
    }
    pub fn verify_unchanged(&self, source: &Source) -> Result<()> {
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
            count: None,
            index_version: 1,
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
        let path = child(&self.root, &pack)?;
        let mut file = File::open(path).map_err(Error::io)?;
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
                    locations.push((i, pack, offset, length, extension));
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
        for (index, pack, offset, length, extension) in locations {
            let input = &inputs[index];
            let mut read = 0u64;
            let result = (|| -> Result<Media> {
                read_cancelled(&input.cancelled)?;
                if current.as_ref().is_none_or(|(name, _)| name != &pack) {
                    let path = child(&self.root, &pack)?;
                    read_cancelled(&input.cancelled)?;
                    current = Some((pack.clone(), File::open(path).map_err(Error::io)?));
                    stats.opens += 1;
                }
                let file = &mut current.as_mut().expect("opened pack").1;
                if offset
                    .checked_add(length)
                    .is_none_or(|end| end > file.metadata().map(|m| m.len()).unwrap_or(0))
                {
                    return Err(Error::new("SOURCE_CORRUPT", "图片位置超出数据包边界"));
                }
                read_cancelled(&input.cancelled)?;
                file.seek(SeekFrom::Start(offset)).map_err(Error::io)?;
                stats.seeks += 1;
                let mut bytes = vec![0; length as usize];
                for chunk in bytes.chunks_mut(64 * 1024) {
                    let mut filled = 0;
                    while filled < chunk.len() {
                        read_cancelled(&input.cancelled)?;
                        let count = file.read(&mut chunk[filled..]).map_err(Error::io)?;
                        if count == 0 {
                            return Err(Error::new("SOURCE_CORRUPT", "数据包读取提前结束"));
                        }
                        filled += count;
                        read += count as u64;
                        stats.bytes += count as u64;
                    }
                }
                read_cancelled(&input.cancelled)?;
                if hex::encode(Sha256::digest(&bytes)) != input.asset_id {
                    return Err(Error::new("SOURCE_CORRUPT", "图片内容校验失败"));
                }
                let content_type = match extension.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "gif" => "image/gif",
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
