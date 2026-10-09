use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};
use studio_domain::{Error, Result};
use studio_storage::SqliteStore;
use tempfile::NamedTempFile;

fn stage(folder: &Path, bytes: &[u8]) -> Result<NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix(".studio-export-")
        .tempfile_in(folder)
        .map_err(Error::io)?;
    file.write_all(bytes).map_err(Error::io)?;
    file.as_file().sync_all().map_err(Error::io)?;
    Ok(file)
}

/// Save-as may replace the chosen file; export publication must never clobber
/// a file created after collision checks. Both paths use private sibling temps.
fn write_atomic(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| Error::invalid("保存位置缺少文件夹"))?;
    let file = stage(folder, bytes)?;
    let result = if overwrite {
        file.persist(path)
    } else {
        file.persist_noclobber(path)
    };
    result.map(|_| ()).map_err(|error| {
        Error::new(
            "EXPORT_WRITE_FAILED",
            format!("写入 {} 失败：{}", path.display(), error.error),
        )
    })
}

pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes, true)
}

pub(super) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes, false)
}

fn inside_lake(store: &SqliteStore, path: &Path) -> Result<bool> {
    for root in store.source_location_roots()? {
        if root.canonicalize().is_ok_and(|root| path.starts_with(root)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// An absolute save-as target outside every lake in the shared source registry.
pub fn validate_save_path(store: &SqliteStore, path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(Error::invalid("保存位置必须是绝对路径的文件"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::invalid("保存位置缺少文件夹"))?
        .canonicalize()
        .map_err(|e| Error::new("DESTINATION_UNAVAILABLE", format!("保存文件夹不可用：{e}")))?;
    if inside_lake(store, &parent)? {
        return Err(Error::invalid("数据湖目录只读，请保存到其他位置"));
    }
    Ok(parent.join(path.file_name().expect("checked file name")))
}

pub fn validate_destination(store: &SqliteStore, destination: &str) -> Result<PathBuf> {
    let path = Path::new(destination);
    let unavailable =
        |e: std::io::Error| Error::new("DESTINATION_UNAVAILABLE", format!("导出文件夹不可用：{e}"));
    if !path.is_absolute() || !fs::metadata(path).map_err(unavailable)?.is_dir() {
        return Err(Error::invalid("导出目标必须是已存在的绝对路径文件夹"));
    }
    let path = path.canonicalize().map_err(unavailable)?;
    if inside_lake(store, &path)? {
        return Err(Error::invalid("数据湖目录只读，请选择其他导出文件夹"));
    }
    // Drop removes this private probe, even if later validation fails.
    tempfile::NamedTempFile::new_in(&path).map_err(unavailable)?;
    Ok(path)
}

pub(super) struct Target {
    pub name: String,
    pub existing: bool,
}

/// None bytes means this image has no tags: an unrelated .txt must not be
/// silently paired with it. The content lives for only one planning iteration.
pub(super) struct Sidecar {
    pub extension: &'static str,
    pub bytes: Option<Vec<u8>>,
}
impl Sidecar {
    fn name(&self, image: &str) -> String {
        let stem = image.rsplit_once('.').map_or(image, |(stem, _)| stem);
        format!("{stem}.{}", self.extension)
    }
}

fn same_image(path: &Path, bytes: u64, asset_id: &str) -> Result<bool> {
    let meta = fs::symlink_metadata(path).map_err(Error::io)?;
    if !meta.is_file() || meta.len() != bytes {
        return Ok(false);
    }
    if asset_id.len() != 64 || !asset_id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(true);
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut File::open(path).map_err(Error::io)?, &mut hash).map_err(Error::io)?;
    Ok(hex::encode(hash.finalize()).eq_ignore_ascii_case(asset_id))
}

fn same_bytes(path: &Path, expected: &[u8]) -> Result<bool> {
    let meta = fs::symlink_metadata(path).map_err(Error::io)?;
    if !meta.is_file() || meta.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut file = File::open(path).map_err(Error::io)?;
    let mut buffer = [0; 8192];
    for chunk in expected.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..chunk.len()])
            .map_err(Error::io)?;
        if &buffer[..chunk.len()] != chunk {
            return Ok(false);
        }
    }
    Ok(true)
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::io(e)),
    }
}

/// Choose the image and requested sidecar together. Reuse only a matching
/// pair, otherwise leave both old files intact and try the next suffix.
pub(super) fn locate(
    folder: &Path,
    planned: &str,
    bytes: u64,
    asset_id: &str,
    sidecar: Option<&Sidecar>,
) -> Result<Target> {
    let (stem, extension) = match planned.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (planned, String::new()),
    };
    for n in 1..=99 {
        let name = if n == 1 {
            planned.to_owned()
        } else {
            format!("{stem}_{n}{extension}")
        };
        let image = folder.join(&name);
        let existing = exists(&image)?;
        if existing && !same_image(&image, bytes, asset_id)? {
            continue;
        }
        if let Some(sidecar) = sidecar {
            let path = folder.join(sidecar.name(&name));
            if exists(&path)?
                && !sidecar
                    .bytes
                    .as_ref()
                    .map(|bytes| same_bytes(&path, bytes))
                    .transpose()?
                    .unwrap_or(false)
            {
                continue;
            }
        }
        return Ok(Target { name, existing });
    }
    Err(Error::new(
        "EXPORT_NAME_CONFLICT",
        format!("{planned} 的图片或元数据已有过多同名文件"),
    ))
}

/// Pending sidecars are spooled to disk, not accumulated across a media batch.
/// Publish only after the corresponding original has been written or verified.
pub(super) struct StagedSidecar {
    name: String,
    temporary: Option<NamedTempFile>,
}
impl StagedSidecar {
    pub fn prepare(folder: &Path, image: &str, sidecar: &Sidecar) -> Result<Option<Self>> {
        let Some(bytes) = &sidecar.bytes else {
            return Ok(None);
        };
        let name = sidecar.name(image);
        let path = folder.join(&name);
        let temporary = if exists(&path)? {
            if !same_bytes(&path, bytes)? {
                return Err(Error::new(
                    "EXPORT_NAME_CONFLICT",
                    format!("{name} 已发生变化"),
                ));
            }
            None
        } else {
            Some(stage(folder, bytes)?)
        };
        Ok(Some(Self { name, temporary }))
    }

    pub fn publish(self, folder: &Path) -> Result<String> {
        if let Some(temporary) = self.temporary {
            temporary
                .persist_noclobber(folder.join(&self.name))
                .map_err(|e| {
                    Error::new(
                        "EXPORT_WRITE_FAILED",
                        format!("写入 {} 失败：{}", self.name, e.error),
                    )
                })?;
        }
        Ok(self.name)
    }
}

/// Errors remain readable after every batch, including cancellation. Only a
/// counter and one bounded writer buffer are retained regardless of lake size.
pub(super) struct FailureLog {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    pub count: u64,
}
impl FailureLog {
    pub fn new(folder: &Path) -> Self {
        Self {
            path: folder.join("export-errors.jsonl"),
            writer: None,
            count: 0,
        }
    }
    pub fn record(&mut self, row: &Value) -> Result<()> {
        if self.writer.is_none() {
            self.writer = Some(BufWriter::new(File::create(&self.path).map_err(Error::io)?));
        }
        let writer = self.writer.as_mut().expect("error writer created");
        serde_json::to_writer(&mut *writer, row).map_err(Error::io)?;
        writer.write_all(b"\n").map_err(Error::io)?;
        self.count += 1;
        Ok(())
    }
    pub fn flush(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.writer {
            writer.flush().map_err(Error::io)?;
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<u64> {
        self.flush()?;
        if let Some(writer) = self.writer {
            writer.get_ref().sync_all().map_err(Error::io)?;
        } else {
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(e)),
            }
        }
        Ok(self.count)
    }
}

#[cfg(test)]
mod tests;
