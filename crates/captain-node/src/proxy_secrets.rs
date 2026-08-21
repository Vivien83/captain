//! Native secret storage used by the standalone Node's authenticated proxy.

#[cfg(feature = "node-runtime")]
use crate::operator::NodeProxyPasswordResolver;
use crate::{NodeNetworkConfig, NodeProxyMode, ResolvedProxyPassword};
use std::{fmt, sync::Arc};
use thiserror::Error;
use zeroize::Zeroizing;

const NODE_PROXY_KEYRING_SERVICE: &str = "captain-node-proxy-v1";
const MAX_SECRET_NAME_BYTES: usize = 128;
const MAX_PROXY_PASSWORD_BYTES: usize = 4096;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NodeProxySecretError {
    #[error("The proxy secret name is invalid")]
    InvalidName,
    #[error("The proxy password is invalid")]
    InvalidPassword,
    #[error("The native proxy secret store is unavailable")]
    StoreUnavailable,
    #[error("The proxy secret could not be verified after storage")]
    VerificationFailed,
    #[error("The configured proxy password secret is unavailable")]
    Missing,
    #[error("Proxy secret deletion requires explicit confirmation")]
    ConfirmationRequired,
}

trait ProxySecretStore: Send + Sync {
    fn load(&self, name: &str) -> Result<Option<Zeroizing<String>>, ()>;
    fn store(&self, name: &str, password: &str) -> Result<(), ()>;
    fn delete(&self, name: &str) -> Result<(), ()>;
}

struct NativeProxySecretStore;

impl ProxySecretStore for NativeProxySecretStore {
    fn load(&self, name: &str) -> Result<Option<Zeroizing<String>>, ()> {
        let entry = native_entry(name)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(Zeroizing::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    }

    fn store(&self, name: &str, password: &str) -> Result<(), ()> {
        native_entry(name)?.set_password(password).map_err(|_| ())
    }

    fn delete(&self, name: &str) -> Result<(), ()> {
        match native_entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }
}

fn native_entry(name: &str) -> Result<keyring::Entry, ()> {
    validate_name(name).map_err(|_| ())?;
    keyring::Entry::new(NODE_PROXY_KEYRING_SERVICE, name).map_err(|_| ())
}

#[derive(Clone)]
pub struct NativeNodeProxySecrets {
    store: Arc<dyn ProxySecretStore>,
}

impl Default for NativeNodeProxySecrets {
    fn default() -> Self {
        Self {
            store: Arc::new(NativeProxySecretStore),
        }
    }
}

impl NativeNodeProxySecrets {
    pub fn set(&self, name: &str, password: &str) -> Result<(), NodeProxySecretError> {
        validate_name(name)?;
        validate_password(password)?;
        self.store
            .store(name, password)
            .map_err(|_| NodeProxySecretError::StoreUnavailable)?;
        let stored = self
            .store
            .load(name)
            .map_err(|_| NodeProxySecretError::StoreUnavailable)?
            .ok_or(NodeProxySecretError::VerificationFailed)?;
        if stored.as_bytes() != password.as_bytes() {
            return Err(NodeProxySecretError::VerificationFailed);
        }
        Ok(())
    }

    pub fn delete(&self, name: &str, confirmed: bool) -> Result<(), NodeProxySecretError> {
        validate_name(name)?;
        if !confirmed {
            return Err(NodeProxySecretError::ConfirmationRequired);
        }
        self.store
            .delete(name)
            .map_err(|_| NodeProxySecretError::StoreUnavailable)
    }

    fn load(&self, name: &str) -> Result<Zeroizing<String>, NodeProxySecretError> {
        validate_name(name)?;
        self.store
            .load(name)
            .map_err(|_| NodeProxySecretError::StoreUnavailable)?
            .ok_or(NodeProxySecretError::Missing)
    }

    pub fn resolve_network(
        &self,
        network: &NodeNetworkConfig,
    ) -> Result<Option<ResolvedProxyPassword>, NodeProxySecretError> {
        self.resolve_proxy(&network.proxy)
    }

    pub fn resolve_proxy(
        &self,
        proxy: &NodeProxyMode,
    ) -> Result<Option<ResolvedProxyPassword>, NodeProxySecretError> {
        let NodeProxyMode::Explicit {
            password_secret: Some(name),
            ..
        } = proxy
        else {
            return Ok(None);
        };
        let password = self.load(name)?;
        Ok(Some(ResolvedProxyPassword::new(name, password.as_str())))
    }

    #[cfg(test)]
    fn test_with_store(store: Arc<dyn ProxySecretStore>) -> Self {
        Self { store }
    }
}

#[cfg(feature = "node-runtime")]
impl NodeProxyPasswordResolver for NativeNodeProxySecrets {
    fn resolve(
        &self,
        network: &NodeNetworkConfig,
    ) -> Result<Option<ResolvedProxyPassword>, String> {
        self.resolve_network(network)
            .map_err(|error| error.to_string())
    }
}

impl fmt::Debug for NativeNodeProxySecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeNodeProxySecrets")
    }
}

fn validate_name(name: &str) -> Result<(), NodeProxySecretError> {
    if name.is_empty()
        || name.len() > MAX_SECRET_NAME_BYTES
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(NodeProxySecretError::InvalidName);
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), NodeProxySecretError> {
    if password.is_empty() || password.len() > MAX_PROXY_PASSWORD_BYTES || password.contains('\0') {
        return Err(NodeProxySecretError::InvalidPassword);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Mutex};

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<BTreeMap<String, String>>,
    }

    impl ProxySecretStore for MemoryStore {
        fn load(&self, name: &str) -> Result<Option<Zeroizing<String>>, ()> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .map(Zeroizing::new))
        }

        fn store(&self, name: &str, password: &str) -> Result<(), ()> {
            self.values
                .lock()
                .unwrap()
                .insert(name.to_string(), password.to_string());
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), ()> {
            self.values.lock().unwrap().remove(name);
            Ok(())
        }
    }

    #[test]
    fn native_proxy_secret_round_trip_is_named_verified_and_confirmed() {
        let secrets = NativeNodeProxySecrets::test_with_store(Arc::new(MemoryStore::default()));
        secrets.set("office-proxy", "not-printed").unwrap();
        let network = NodeNetworkConfig {
            proxy: NodeProxyMode::Explicit {
                url: "https://proxy.example".to_string(),
                username: Some("operator".to_string()),
                password_secret: Some("office-proxy".to_string()),
            },
            ..NodeNetworkConfig::new("https://hub.example")
        };
        let resolved = secrets.resolve_network(&network).unwrap().unwrap();
        let debug = format!("{resolved:?} {secrets:?}");
        assert!(!debug.contains("not-printed"));
        assert!(matches!(
            secrets.delete("office-proxy", false),
            Err(NodeProxySecretError::ConfirmationRequired)
        ));
        secrets.delete("office-proxy", true).unwrap();
        assert!(secrets.resolve_network(&network).is_err());
    }

    #[test]
    fn invalid_names_and_passwords_never_reach_the_store() {
        let secrets = NativeNodeProxySecrets::test_with_store(Arc::new(MemoryStore::default()));
        assert!(matches!(
            secrets.set("../proxy", "password"),
            Err(NodeProxySecretError::InvalidName)
        ));
        assert!(matches!(
            secrets.set("proxy", ""),
            Err(NodeProxySecretError::InvalidPassword)
        ));
    }
}
