use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};

fn collect(root: &Path, path: &Path, files: &mut Vec<String>) {
    let mut entries = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        if entry.is_dir() && entry.file_name().unwrap() != "__pycache__" {
            collect(root, &entry, files);
        } else if entry.extension().is_some_and(|e| e == "py") {
            files.push(
                entry
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/"),
            );
        }
    }
}
fn main() {
    let root =
        Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../services/lake-worker");
    // Build outputs can be reused after a checkout moves or shares a target directory.
    // Absolute watched/include paths would keep that output pinned to the old checkout.
    println!("cargo:rerun-if-changed=../../services/lake-worker/src");
    let mut files = vec!["worker.py".to_owned(), "pyproject.toml".to_owned()];
    collect(&root, &root.join("src"), &mut files);
    let mut hash = Sha256::new();
    let mut generated = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for relative in files {
        let path = root.join(&relative);
        println!("cargo:rerun-if-changed=../../services/lake-worker/{relative}");
        let content = fs::read(&path).unwrap();
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update((content.len() as u64).to_le_bytes());
        hash.update(&content);
        let include = format!("/../../services/lake-worker/{relative}");
        generated.push_str(&format!(
            "({relative:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {include:?}))),\n"
        ));
    }
    generated.push_str(&format!(
        "];\npub const REVISION: &str = \"{:x}\";\n",
        hash.finalize()
    ));
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("lake_worker_bundle.rs"),
        generated,
    )
    .unwrap();
}
