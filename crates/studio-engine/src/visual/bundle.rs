use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use studio_domain::{Error, Result};
include!(concat!(env!("OUT_DIR"), "/visual_worker_bundle.rs"));

pub fn install(application_root: &Path) -> Result<PathBuf> {
    let root = application_root.join("visual-worker").join(REVISION);
    fs::create_dir_all(&root).map_err(Error::io)?;
    let root = root.canonicalize().map_err(Error::io)?;
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
