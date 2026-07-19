use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use crate::ExecutorError;

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const FORMAT_VERSION: u8 = 1;
const MAX_CIPHERTEXT_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct StateCipher {
    key: Zeroizing<[u8; KEY_BYTES]>,
    key_path: PathBuf,
}

impl StateCipher {
    pub(crate) fn open(key_path: &Path) -> Result<Self, ExecutorError> {
        ensure_regular_private_key_path(key_path)?;
        if !key_path.exists() {
            let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
            rand::rngs::OsRng.fill_bytes(key.as_mut());
            if !captain_types::durable_fs::create_new(key_path, key.as_ref())? {
                key.zeroize();
            }
        }
        ensure_regular_private_key_path(key_path)?;
        make_private(key_path)?;
        let bytes = Zeroizing::new(std::fs::read(key_path)?);
        if bytes.len() != KEY_BYTES {
            return Err(ExecutorError::Crypto(format!(
                "CapSpec state key {} must contain exactly {KEY_BYTES} bytes",
                key_path.display()
            )));
        }
        let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
        key.copy_from_slice(&bytes);
        Ok(Self {
            key,
            key_path: key_path.to_path_buf(),
        })
    }

    pub(crate) fn seal(&self, context: &str, plaintext: &[u8]) -> Result<Vec<u8>, ExecutorError> {
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| ExecutorError::Crypto("invalid CapSpec state key".to_string()))?;
        let mut nonce = [0_u8; NONCE_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| ExecutorError::Crypto("cannot encrypt CapSpec run state".to_string()))?;
        let mut output = Vec::with_capacity(1 + NONCE_BYTES + encrypted.len());
        output.push(FORMAT_VERSION);
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&encrypted);
        Ok(output)
    }

    pub(crate) fn open_blob(
        &self,
        context: &str,
        ciphertext: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, ExecutorError> {
        if ciphertext.len() <= 1 + NONCE_BYTES
            || ciphertext.len() > MAX_CIPHERTEXT_BYTES
            || ciphertext[0] != FORMAT_VERSION
        {
            return Err(ExecutorError::Crypto(format!(
                "invalid encrypted CapSpec state in {}",
                self.key_path.display()
            )));
        }
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| ExecutorError::Crypto("invalid CapSpec state key".to_string()))?;
        cipher
            .decrypt(
                Nonce::from_slice(&ciphertext[1..1 + NONCE_BYTES]),
                Payload {
                    msg: &ciphertext[1 + NONCE_BYTES..],
                    aad: context.as_bytes(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| {
                ExecutorError::Crypto("CapSpec run state authentication failed".to_string())
            })
    }
}

fn ensure_regular_private_key_path(path: &Path) -> Result<(), ExecutorError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(ExecutorError::Crypto(format!(
                "CapSpec state key {} must be a regular file, never a symlink",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn make_private(path: &Path) -> Result<(), ExecutorError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_private(_path: &Path) -> Result<(), ExecutorError> {
    Ok(())
}

trait ZeroizeArray {
    fn zeroize(&mut self);
}

impl ZeroizeArray for Zeroizing<[u8; KEY_BYTES]> {
    fn zeroize(&mut self) {
        self.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciphertext_is_context_bound_and_not_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let cipher = StateCipher::open(&temp.path().join("state.key")).unwrap();
        let sealed = cipher.seal("run:a", b"private-value").unwrap();
        assert!(!sealed
            .windows(b"private-value".len())
            .any(|window| window == b"private-value"));
        assert_eq!(
            cipher.open_blob("run:a", &sealed).unwrap().as_slice(),
            b"private-value"
        );
        assert!(cipher.open_blob("run:b", &sealed).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn state_key_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.key");
        StateCipher::open(&path).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_state_key_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.key");
        std::fs::write(&target, [0_u8; KEY_BYTES]).unwrap();
        let link = temp.path().join("state.key");
        symlink(target, &link).unwrap();
        assert!(StateCipher::open(&link).is_err());
    }
}
