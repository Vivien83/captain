//! Native, profile-scoped storage for lightweight Client credentials.

use super::NodePairingError;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use zeroize::Zeroizing;

const CLIENT_KEYRING_SERVICE: &str = "captain-console-client-v1";
const MAX_CREDENTIAL_REFERENCE_BYTES: usize = 128;

pub(crate) trait ClientCredentialStore: Send + Sync {
    fn load(&self, reference: &str) -> Result<Option<Zeroizing<String>>, ()>;
    fn store(&self, reference: &str, credential: &str) -> Result<(), ()>;
    fn delete(&self, reference: &str) -> Result<(), ()>;
}

pub(super) struct NativeClientCredentialStore;

impl ClientCredentialStore for NativeClientCredentialStore {
    fn load(&self, reference: &str) -> Result<Option<Zeroizing<String>>, ()> {
        let entry = native_entry(reference)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    }

    fn store(&self, reference: &str, credential: &str) -> Result<(), ()> {
        native_entry(reference)?
            .set_password(credential)
            .map_err(|_| ())
    }

    fn delete(&self, reference: &str) -> Result<(), ()> {
        match native_entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }
}

fn native_entry(reference: &str) -> Result<keyring::Entry, ()> {
    validate_credential_reference(reference)?;
    keyring::Entry::new(CLIENT_KEYRING_SERVICE, reference).map_err(|_| ())
}

pub(super) enum CredentialPersistence {
    Inline,
    Native {
        reference: String,
        store: Arc<dyn ClientCredentialStore>,
    },
}

impl CredentialPersistence {
    pub(super) fn native(reference: String) -> Result<Self, NodePairingError> {
        validate_credential_reference(&reference)
            .map_err(|_| NodePairingError::InvalidCredentialReference)?;
        Ok(Self::Native {
            reference,
            store: Arc::new(NativeClientCredentialStore),
        })
    }

    #[cfg(test)]
    pub(super) fn test_native(
        reference: String,
        store: Arc<dyn ClientCredentialStore>,
    ) -> Result<Self, NodePairingError> {
        validate_credential_reference(&reference)
            .map_err(|_| NodePairingError::InvalidCredentialReference)?;
        Ok(Self::Native { reference, store })
    }

    pub(super) fn is_native(&self) -> bool {
        matches!(self, Self::Native { .. })
    }

    pub(super) fn resolve(
        &self,
        credential: &PersistedCredential,
    ) -> Result<Zeroizing<String>, NodePairingError> {
        match (self, credential) {
            (Self::Inline, PersistedCredential::Inline(value)) => {
                Ok(Zeroizing::new(value.to_string()))
            }
            (Self::Native { reference, store }, PersistedCredential::Native { credential_ref })
                if credential_ref == reference =>
            {
                store
                    .load(reference)
                    .map_err(|_| NodePairingError::CredentialStoreUnavailable)?
                    .ok_or(NodePairingError::CredentialUnavailable)
            }
            (Self::Native { .. }, PersistedCredential::Inline(value)) => {
                Ok(Zeroizing::new(value.to_string()))
            }
            _ => Err(NodePairingError::CredentialReferenceMismatch),
        }
    }

    pub(super) fn externalize(
        &self,
        credential: &mut PersistedCredential,
    ) -> Result<bool, NodePairingError> {
        let Self::Native { reference, store } = self else {
            return Ok(false);
        };
        match credential {
            PersistedCredential::Native { credential_ref } if credential_ref == reference => {
                let stored = store
                    .load(reference)
                    .map_err(|_| NodePairingError::CredentialStoreUnavailable)?
                    .ok_or(NodePairingError::CredentialUnavailable)?;
                validate_raw_credential(&stored)?;
                Ok(false)
            }
            PersistedCredential::Native { .. } => {
                Err(NodePairingError::CredentialReferenceMismatch)
            }
            PersistedCredential::Inline(value) => {
                let existing = store
                    .load(reference)
                    .map_err(|_| NodePairingError::CredentialStoreUnavailable)?;
                match existing {
                    Some(existing) if existing.as_bytes() != value.as_bytes() => {
                        return Err(NodePairingError::LocalCredentialConflict)
                    }
                    Some(_) => {}
                    None => store
                        .store(reference, value)
                        .map_err(|_| NodePairingError::CredentialStoreUnavailable)?,
                }
                let verified = store
                    .load(reference)
                    .map_err(|_| NodePairingError::CredentialStoreUnavailable)?
                    .ok_or(NodePairingError::CredentialUnavailable)?;
                if verified.as_bytes() != value.as_bytes() {
                    return Err(NodePairingError::CredentialStoreVerificationFailed);
                }
                *credential = PersistedCredential::Native {
                    credential_ref: reference.clone(),
                };
                Ok(true)
            }
        }
    }

    pub(super) fn clear(&self) -> Result<(), NodePairingError> {
        let Self::Native { reference, store } = self else {
            return Ok(());
        };
        store
            .delete(reference)
            .map_err(|_| NodePairingError::CredentialStoreUnavailable)
    }
}

impl fmt::Debug for CredentialPersistence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialPersistence")
            .field("mode", &if self.is_native() { "native" } else { "inline" })
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum PersistedCredential {
    Inline(Zeroizing<String>),
    Native { credential_ref: String },
}

impl PersistedCredential {
    pub(super) fn inline(value: Zeroizing<String>) -> Self {
        Self::Inline(value)
    }

    pub(super) fn validate_shape(&self) -> Result<(), NodePairingError> {
        match self {
            Self::Inline(value) => validate_raw_credential(value),
            Self::Native { credential_ref } => validate_credential_reference(credential_ref)
                .map_err(|_| NodePairingError::InvalidCredentialReference),
        }
    }

    pub(super) fn is_native(&self) -> bool {
        matches!(self, Self::Native { .. })
    }
}

impl fmt::Debug for PersistedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

fn validate_raw_credential(value: &str) -> Result<(), NodePairingError> {
    if value.len() < 32
        || value.len() > 512
        || value.chars().any(char::is_control)
        || !value.is_ascii()
    {
        return Err(NodePairingError::StateCorrupt);
    }
    Ok(())
}

fn validate_credential_reference(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_REFERENCE_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(());
    }
    Ok(())
}
