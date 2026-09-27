use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use studio_domain::{Error, Result};

include!(concat!(env!("OUT_DIR"), "/lake_worker_bundle.rs"));

/// Each engine carries its own reviewed source. A new revision never edits a
/// running worker's files, and no path from the old Store checkout is consulted.
pub fn install(application_root: &Path) -> Result<PathBuf> {
    let root = application_root.join("lake-worker").join(REVISION);
    for (relative, bytes) in FILES {
        let path = root.join(relative);
        if fs::read(&path).is_ok_and(|old| old == *bytes) {
            continue;
        }
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(Error::io)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(Error::io)?;
        temporary.write_all(bytes).map_err(Error::io)?;
        temporary.as_file().sync_all().map_err(Error::io)?;
        temporary.persist(&path).map_err(Error::io)?;
    }
    Ok(root)
}
