//! Linux envelopes share their format and keyring namespace with the lake worker.
//! Only an opaque key ID is kept on disk; key material lives in Secret Service.
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use studio_domain::{Error, Result};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"DSC1";
const HEADER: usize = 36; // magic + lowercase UUID hex
const NONCE: usize = 12;
static ACTIVE: OnceLock<String> = OnceLock::new();
static KEY: Mutex<Option<(String, Zeroizing<Vec<u8>>)>> = Mutex::new(None);

fn unavailable() -> Error {
    Error::new(
        "LLM_CREDENTIAL_UNAVAILABLE",
        "无法读取系统密钥环；请安装 libsecret-tools，并解锁当前桌面会话的密钥环",
    )
}
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn keyring(id: &str, secret: Option<&[u8]>) -> Result<Zeroizing<Vec<u8>>> {
    let mut command = Command::new("secret-tool");
    if secret.is_some() {
        command.args(["store", "--label=Dataset Studio credential key"]);
    } else {
        command.arg("lookup");
    }
    command
        .args(["application", "com.xuness.datasetstudio", "key", id])
        .stdin(if secret.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| unavailable())?;
    if let Some(secret) = secret {
        let encoded = Zeroizing::new(STANDARD.encode(secret));
        let write = child
            .stdin
            .take()
            .ok_or_else(unavailable)
            .and_then(|mut stdin| {
                stdin
                    .write_all(encoded.as_bytes())
                    .map_err(|_| unavailable())
            });
        if write.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unavailable());
        }
    }
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable());
            }
        }
    }
    let output = child.wait_with_output().map_err(|_| unavailable())?;
    let bytes = Zeroizing::new(output.stdout);
    if !output.status.success() {
        return Err(unavailable());
    }
    Ok(bytes)
}
fn decode_key(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| unavailable())?
        .trim();
    let key = Zeroizing::new(STANDARD.decode(text).map_err(|_| unavailable())?);
    if key.len() != 32 {
        return Err(unavailable());
    }
    Ok(key)
}
fn active_id() -> Result<String> {
    if let Some(id) = ACTIVE.get() {
        return Ok(id.clone());
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .filter(|p| p.is_absolute())
        .ok_or_else(unavailable)?;
    let root = base.join("dataset-studio");
    fs::create_dir_all(&root).map_err(|_| unavailable())?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| unavailable())?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join("credential-key.lock"))
        .map_err(|_| unavailable())?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|_| unavailable())?;
    let marker = root.join("credential-key");
    let id = match fs::read_to_string(&marker) {
        Ok(id) if valid_id(id.trim()) => id.trim().to_owned(),
        Ok(_) => return Err(unavailable()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Never replace a key after a failed lookup. Each envelope embeds
            // its key ID, so older keys remain usable if the marker is lost.
            let id = uuid::Uuid::new_v4().simple().to_string();
            let mut key = Zeroizing::new(vec![0u8; 32]);
            getrandom::fill(&mut key).map_err(|_| unavailable())?;
            keyring(&id, Some(&key))?;
            if *decode_key(&keyring(&id, None)?)? != *key {
                return Err(unavailable());
            }
            let mut file = tempfile::NamedTempFile::new_in(&root).map_err(|_| unavailable())?;
            file.write_all(id.as_bytes()).map_err(|_| unavailable())?;
            file.as_file().sync_all().map_err(|_| unavailable())?;
            file.persist(marker).map_err(|_| unavailable())?;
            id
        }
        Err(_) => return Err(unavailable()),
    };
    let _ = ACTIVE.set(id.clone());
    Ok(id)
}
pub(super) fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let id = if encrypt {
        active_id()?
    } else {
        if input.len() < HEADER + NONCE + 16 || &input[..4] != MAGIC {
            return Err(unavailable());
        }
        let id = std::str::from_utf8(&input[4..HEADER]).map_err(|_| unavailable())?;
        if !valid_id(id) {
            return Err(unavailable());
        }
        id.to_owned()
    };
    let mut slot = KEY.lock().map_err(|_| unavailable())?;
    if slot.as_ref().is_none_or(|(cached, _)| cached != &id) {
        *slot = Some((id.clone(), decode_key(&keyring(&id, None)?)?));
    }
    let key = &slot.as_ref().ok_or_else(unavailable)?.1;
    transform(input, encrypt, &id, key)
}
fn transform(input: &[u8], encrypt: bool, id: &str, key: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| unavailable())?;
    if encrypt {
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(id.as_bytes());
        let mut nonce = [0u8; NONCE];
        getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: input,
                    aad: &header,
                },
            )
            .map_err(|_| unavailable())?;
        header.extend_from_slice(&nonce);
        header.extend_from_slice(&encrypted);
        Ok(header)
    } else {
        cipher
            .decrypt(
                Nonce::from_slice(&input[HEADER..HEADER + NONCE]),
                Payload {
                    msg: &input[HEADER + NONCE..],
                    aad: &input[..HEADER],
                },
            )
            .map_err(|_| unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_envelopes_reject_corruption_and_use_fresh_nonces() {
        let id = "a".repeat(32);
        let key = [7u8; 32];
        let first = transform(b"fixture", true, &id, &key).unwrap();
        let second = transform(b"fixture", true, &id, &key).unwrap();
        assert_ne!(first, second);
        assert_eq!(transform(&first, false, &id, &key).unwrap(), b"fixture");
        for position in [4, HEADER, first.len() - 1] {
            let mut broken = first.clone();
            broken[position] ^= 1;
            assert!(transform(&broken, false, &id, &key).is_err());
        }
        assert!(protect(b"invalid", false).is_err());
    }
}
