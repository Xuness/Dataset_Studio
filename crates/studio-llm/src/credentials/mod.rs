use std::{fs, io::Write, path::PathBuf};
use studio_application::llm::{LlmCredentials, LlmSecret};
use studio_domain::{Error, Result, new_id, validate_id};
#[cfg(windows)]
mod windows;

pub struct CredentialVault {
    root: PathBuf,
}
impl CredentialVault {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}
impl LlmCredentials for CredentialVault {
    fn put(&self, secret: LlmSecret) -> Result<String> {
        if secret.expose().trim().is_empty()
            || secret.expose().len() > 8192
            || secret.expose().chars().any(char::is_control)
        {
            return Err(Error::invalid("API Key 为空、过长或包含控制字符"));
        }
        let encrypted = protect(secret.expose().as_bytes(), true)?;
        fs::create_dir_all(&self.root).map_err(Error::io)?;
        let reference = new_id();
        let mut file = tempfile::NamedTempFile::new_in(&self.root).map_err(Error::io)?;
        file.write_all(&encrypted).map_err(Error::io)?;
        file.as_file().sync_all().map_err(Error::io)?;
        file.persist(self.root.join(format!("{reference}.bin")))
            .map_err(Error::io)?;
        Ok(reference)
    }
    fn get(&self, reference: &str) -> Result<LlmSecret> {
        validate_id(reference)?;
        let path = self.root.join(format!("{reference}.bin"));
        if fs::metadata(&path)
            .map_err(|_| {
                Error::new(
                    "LLM_CREDENTIAL_UNAVAILABLE",
                    "凭据不可用，请重新设置 API Key",
                )
            })?
            .len()
            > 131072
        {
            return Err(Error::new("LLM_CREDENTIAL_UNAVAILABLE", "凭据格式无效"));
        }
        let bytes =
            fs::read(path).map_err(|_| Error::new("LLM_CREDENTIAL_UNAVAILABLE", "凭据不可用"))?;
        let decrypted = zeroize::Zeroizing::new(protect(&bytes, false)?);
        let text = std::str::from_utf8(&decrypted)
            .map_err(|_| Error::new("LLM_CREDENTIAL_UNAVAILABLE", "凭据格式无效"))?;
        Ok(LlmSecret::new(text.into()))
    }
    fn remove(&self, reference: &str) -> Result<()> {
        validate_id(reference)?;
        match fs::remove_file(self.root.join(format!("{reference}.bin"))) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(e)),
        }
    }
}
#[cfg(windows)]
fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    windows::protect(input, encrypt)
}
#[cfg(not(windows))]
fn protect(_: &[u8], _: bool) -> Result<Vec<u8>> {
    Err(Error::new(
        "LLM_CREDENTIAL_UNAVAILABLE",
        "当前平台尚未配置系统凭据适配器",
    ))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn dpapi_survives_reopen_without_plaintext_and_rejects_corruption() {
        let root = tempfile::tempdir().unwrap();
        let vault = CredentialVault::new(root.path().into());
        let secret = "fixture-secret-not-a-real-api-key";
        let id = vault.put(LlmSecret::new(secret.into())).unwrap();
        let path = root.path().join(format!("{id}.bin"));
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.windows(secret.len()).any(|s| s == secret.as_bytes()));
        let reopened = CredentialVault::new(root.path().into());
        assert_eq!(reopened.get(&id).unwrap().expose(), secret);
        fs::write(&path, b"broken").unwrap();
        assert!(reopened.get(&id).is_err());
        reopened.remove(&id).unwrap();
        assert!(!path.exists());
    }
}
