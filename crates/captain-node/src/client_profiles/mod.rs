//! Crash-safe local inventory for independent lightweight Client profiles.

use captain_types::durable_fs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt, fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
};
use thiserror::Error;
use uuid::Uuid;

mod migration;

const PROFILE_REGISTRY_SCHEMA_VERSION: u16 = 1;
const PROFILE_REGISTRY_FILE: &str = "profiles.toml";
const PROFILE_REGISTRY_LOCK: &str = "profiles.lock";
const PROFILE_DIRECTORY: &str = "profiles";
const MAX_PROFILE_REGISTRY_BYTES: u64 = 128 * 1024;
const MAX_PROFILES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientProfileEntry {
    pub id: String,
    pub created_at_ms: i64,
    pub active: bool,
    pub label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedProfileEntry {
    id: String,
    created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingLegacyImport {
    profile_id: String,
    created_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedProfileRegistry {
    schema_version: u16,
    active_profile: Option<String>,
    profiles: Vec<PersistedProfileEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_legacy_import: Option<PendingLegacyImport>,
}

impl Default for PersistedProfileRegistry {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_REGISTRY_SCHEMA_VERSION,
            active_profile: None,
            profiles: Vec::new(),
            pending_legacy_import: None,
        }
    }
}

pub struct ClientProfileRegistry {
    root: PathBuf,
    profiles_root: PathBuf,
    registry_path: PathBuf,
    lock_path: PathBuf,
}

impl ClientProfileRegistry {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ClientProfileRegistryError> {
        let root = root.into();
        reject_unsafe_path(&root, true)?;
        durable_fs::create_dir_all(&root)
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        set_private_dir(&root)?;
        let root =
            fs::canonicalize(root).map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        let profiles_root = root.join(PROFILE_DIRECTORY);
        reject_unsafe_path(&profiles_root, true)?;
        durable_fs::create_dir_all(&profiles_root)
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        set_private_dir(&profiles_root)?;
        let registry = Self {
            registry_path: root.join(PROFILE_REGISTRY_FILE),
            lock_path: root.join(PROFILE_REGISTRY_LOCK),
            root,
            profiles_root,
        };
        reject_unsafe_path(&registry.registry_path, false)?;
        reject_unsafe_path(&registry.lock_path, false)?;
        registry.with_lock(|manifest| registry.validate_filesystem(manifest))?;
        Ok(registry)
    }

    pub fn list(&self) -> Result<Vec<ClientProfileEntry>, ClientProfileRegistryError> {
        self.with_lock(|manifest| {
            ensure_no_pending_migration(manifest)?;
            self.validate_filesystem(manifest)?;
            Ok(manifest
                .profiles
                .iter()
                .map(|profile| ClientProfileEntry {
                    id: profile.id.clone(),
                    created_at_ms: profile.created_at_ms,
                    active: manifest.active_profile.as_deref() == Some(profile.id.as_str()),
                    label: profile.label.clone(),
                })
                .collect())
        })
    }

    pub fn create_profile(
        &self,
        created_at_ms: i64,
    ) -> Result<ClientProfileEntry, ClientProfileRegistryError> {
        if created_at_ms < 0 {
            return Err(ClientProfileRegistryError::InvalidInput);
        }
        self.with_lock(|manifest| {
            ensure_no_pending_migration(manifest)?;
            self.validate_filesystem(manifest)?;
            if manifest.profiles.len() >= MAX_PROFILES {
                return Err(ClientProfileRegistryError::ProfileLimitReached);
            }
            let id = Uuid::new_v4().to_string();
            let profile_root = self.profiles_root.join(&id);
            reject_unsafe_path(&profile_root, true)?;
            durable_fs::create_dir_all(&profile_root)
                .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
            set_private_dir(&profile_root)?;

            manifest.profiles.push(PersistedProfileEntry {
                id: id.clone(),
                created_at_ms,
                label: None,
            });
            let became_active = manifest.active_profile.is_none();
            if became_active {
                manifest.active_profile = Some(id.clone());
            }
            if let Err(error) = self.save_manifest(manifest) {
                manifest.profiles.retain(|profile| profile.id != id);
                if became_active {
                    manifest.active_profile = None;
                }
                let _ = fs::remove_dir(&profile_root);
                return Err(error);
            }
            Ok(ClientProfileEntry {
                id,
                created_at_ms,
                active: became_active,
                label: None,
            })
        })
    }

    pub fn set_label(
        &self,
        profile_id: &str,
        label: &str,
    ) -> Result<(), ClientProfileRegistryError> {
        validate_profile_id(profile_id)?;
        let label = validate_profile_label(label)?;
        self.with_lock(|manifest| {
            ensure_no_pending_migration(manifest)?;
            self.validate_filesystem(manifest)?;
            let profile = manifest
                .profiles
                .iter_mut()
                .find(|profile| profile.id == profile_id)
                .ok_or(ClientProfileRegistryError::ProfileNotFound)?;
            if profile.label.as_deref() != Some(label.as_str()) {
                profile.label = Some(label);
                self.save_manifest(manifest)?;
            }
            Ok(())
        })
    }

    pub fn set_active(&self, profile_id: &str) -> Result<(), ClientProfileRegistryError> {
        validate_profile_id(profile_id)?;
        self.with_lock(|manifest| {
            ensure_no_pending_migration(manifest)?;
            self.validate_filesystem(manifest)?;
            if !manifest
                .profiles
                .iter()
                .any(|profile| profile.id == profile_id)
            {
                return Err(ClientProfileRegistryError::ProfileNotFound);
            }
            if manifest.active_profile.as_deref() != Some(profile_id) {
                manifest.active_profile = Some(profile_id.to_string());
                self.save_manifest(manifest)?;
            }
            Ok(())
        })
    }

    pub fn clear_active(&self, profile_id: &str) -> Result<(), ClientProfileRegistryError> {
        validate_profile_id(profile_id)?;
        self.with_lock(|manifest| {
            ensure_no_pending_migration(manifest)?;
            self.validate_filesystem(manifest)?;
            if !manifest
                .profiles
                .iter()
                .any(|profile| profile.id == profile_id)
            {
                return Err(ClientProfileRegistryError::ProfileNotFound);
            }
            if manifest.active_profile.as_deref() == Some(profile_id) {
                manifest.active_profile = None;
                self.save_manifest(manifest)?;
            }
            Ok(())
        })
    }

    pub fn active_profile(&self) -> Result<Option<ClientProfileEntry>, ClientProfileRegistryError> {
        self.list()
            .map(|profiles| profiles.into_iter().find(|profile| profile.active))
    }

    pub fn profile_root(&self, profile_id: &str) -> Result<PathBuf, ClientProfileRegistryError> {
        validate_profile_id(profile_id)?;
        self.with_lock(|manifest| {
            ensure_no_pending_migration(manifest)?;
            self.validate_filesystem(manifest)?;
            if !manifest
                .profiles
                .iter()
                .any(|profile| profile.id == profile_id)
            {
                return Err(ClientProfileRegistryError::ProfileNotFound);
            }
            Ok(self.profiles_root.join(profile_id))
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn with_lock<T>(
        &self,
        operation: impl FnOnce(&mut PersistedProfileRegistry) -> Result<T, ClientProfileRegistryError>,
    ) -> Result<T, ClientProfileRegistryError> {
        reject_unsafe_path(&self.lock_path, false)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        set_private_file(&self.lock_path)?;
        lock.lock_exclusive()
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        let mut manifest = self.load_manifest()?;
        let result = operation(&mut manifest);
        let _ = FileExt::unlock(&lock);
        result
    }

    fn load_manifest(&self) -> Result<PersistedProfileRegistry, ClientProfileRegistryError> {
        reject_unsafe_path(&self.registry_path, false)?;
        let metadata = match fs::metadata(&self.registry_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PersistedProfileRegistry::default())
            }
            Err(_) => return Err(ClientProfileRegistryError::StateUnavailable),
        };
        if !metadata.is_file() || metadata.len() > MAX_PROFILE_REGISTRY_BYTES {
            return Err(ClientProfileRegistryError::StateCorrupt);
        }
        let raw = fs::read_to_string(&self.registry_path)
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        let manifest = toml::from_str::<PersistedProfileRegistry>(&raw)
            .map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
        validate_manifest(&manifest)?;
        Ok(manifest)
    }

    fn save_manifest(
        &self,
        manifest: &PersistedProfileRegistry,
    ) -> Result<(), ClientProfileRegistryError> {
        validate_manifest(manifest)?;
        let raw = toml::to_string_pretty(manifest)
            .map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
        if raw.len() as u64 > MAX_PROFILE_REGISTRY_BYTES {
            return Err(ClientProfileRegistryError::StateCorrupt);
        }
        reject_unsafe_path(&self.registry_path, false)?;
        durable_fs::atomic_write(&self.registry_path, raw.as_bytes())
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
        set_private_file(&self.registry_path)
    }

    fn validate_filesystem(
        &self,
        manifest: &PersistedProfileRegistry,
    ) -> Result<(), ClientProfileRegistryError> {
        validate_manifest(manifest)?;
        let expected = manifest
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<HashSet<_>>();
        let pending_id = manifest
            .pending_legacy_import
            .as_ref()
            .map(|pending| pending.profile_id.as_str());
        for profile in &manifest.profiles {
            let path = self.profiles_root.join(&profile.id);
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ClientProfileRegistryError::UnsafePath);
            }
            set_private_dir(&path)?;
        }
        if let Some(profile_id) = pending_id {
            let path = self.profiles_root.join(profile_id);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(ClientProfileRegistryError::UnsafePath)
                }
                Ok(_) => set_private_dir(&path)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ClientProfileRegistryError::StateUnavailable),
            }
        }
        for entry in fs::read_dir(&self.profiles_root)
            .map_err(|_| ClientProfileRegistryError::StateUnavailable)?
        {
            let entry = entry.map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
            if !expected.contains(name.as_str()) && pending_id != Some(name.as_str()) {
                return Err(ClientProfileRegistryError::OrphanedProfile);
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ClientProfileRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientProfileRegistry")
            .field("root", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

fn validate_manifest(
    manifest: &PersistedProfileRegistry,
) -> Result<(), ClientProfileRegistryError> {
    if manifest.schema_version != PROFILE_REGISTRY_SCHEMA_VERSION
        || manifest.profiles.len() > MAX_PROFILES
    {
        return Err(ClientProfileRegistryError::StateCorrupt);
    }
    let mut ids = HashSet::with_capacity(manifest.profiles.len());
    for profile in &manifest.profiles {
        validate_profile_id(&profile.id).map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
        if profile.created_at_ms < 0 || !ids.insert(profile.id.as_str()) {
            return Err(ClientProfileRegistryError::StateCorrupt);
        }
        if let Some(label) = &profile.label {
            let canonical = validate_profile_label(label)
                .map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
            if canonical != *label {
                return Err(ClientProfileRegistryError::StateCorrupt);
            }
        }
    }
    if manifest
        .active_profile
        .as_ref()
        .is_some_and(|active| !ids.contains(active.as_str()))
    {
        return Err(ClientProfileRegistryError::StateCorrupt);
    }
    if let Some(pending) = &manifest.pending_legacy_import {
        validate_profile_id(&pending.profile_id)
            .map_err(|_| ClientProfileRegistryError::StateCorrupt)?;
        if pending.created_at_ms < 0 || ids.contains(pending.profile_id.as_str()) {
            return Err(ClientProfileRegistryError::StateCorrupt);
        }
    }
    Ok(())
}

fn ensure_no_pending_migration(
    manifest: &PersistedProfileRegistry,
) -> Result<(), ClientProfileRegistryError> {
    if manifest.pending_legacy_import.is_some() {
        Err(ClientProfileRegistryError::MigrationPending)
    } else {
        Ok(())
    }
}

fn validate_profile_id(profile_id: &str) -> Result<(), ClientProfileRegistryError> {
    let parsed =
        Uuid::parse_str(profile_id).map_err(|_| ClientProfileRegistryError::InvalidInput)?;
    if parsed.to_string() != profile_id {
        return Err(ClientProfileRegistryError::InvalidInput);
    }
    Ok(())
}

fn validate_profile_label(label: &str) -> Result<String, ClientProfileRegistryError> {
    let label = label.trim();
    if label.is_empty() || label.len() > 80 || label.chars().any(char::is_control) {
        return Err(ClientProfileRegistryError::InvalidInput);
    }
    Ok(label.to_string())
}

fn reject_unsafe_path(path: &Path, directory: bool) -> Result<(), ClientProfileRegistryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ClientProfileRegistryError::UnsafePath)
        }
        Ok(metadata) if directory && !metadata.is_dir() => {
            Err(ClientProfileRegistryError::UnsafePath)
        }
        Ok(metadata) if !directory && !metadata.is_file() => {
            Err(ClientProfileRegistryError::UnsafePath)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ClientProfileRegistryError::StateUnavailable),
    }
}

#[cfg(unix)]
fn set_private_dir(path: &Path) -> Result<(), ClientProfileRegistryError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| ClientProfileRegistryError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_dir(_path: &Path) -> Result<(), ClientProfileRegistryError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<(), ClientProfileRegistryError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| ClientProfileRegistryError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<(), ClientProfileRegistryError> {
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientProfileRegistryError {
    #[error("the Client profile registry is unavailable")]
    StateUnavailable,
    #[error("the Client profile registry is corrupt")]
    StateCorrupt,
    #[error("the Client profile registry contains an unsafe path")]
    UnsafePath,
    #[error("the Client profile registry contains an orphaned profile")]
    OrphanedProfile,
    #[error("the Client profile identifier is invalid")]
    InvalidInput,
    #[error("the Client profile was not found")]
    ProfileNotFound,
    #[error("the Client profile limit was reached")]
    ProfileLimitReached,
    #[error("the legacy Client profile migration must be resumed")]
    MigrationPending,
    #[error("legacy Client state conflicts with the profile registry")]
    LegacyImportConflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_profiles_keep_distinct_roots_and_one_explicit_authority() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        let first = registry.create_profile(10).unwrap();
        let second = registry.create_profile(20).unwrap();

        assert!(first.active);
        assert!(!second.active);
        assert_ne!(
            registry.profile_root(&first.id).unwrap(),
            registry.profile_root(&second.id).unwrap()
        );
        registry.set_active(&second.id).unwrap();
        registry
            .set_label(&second.id, "Production Captain")
            .unwrap();
        let listed = registry.list().unwrap();
        assert_eq!(listed.iter().filter(|profile| profile.active).count(), 1);
        assert_eq!(registry.active_profile().unwrap().unwrap().id, second.id);
        assert_eq!(
            registry.active_profile().unwrap().unwrap().label.as_deref(),
            Some("Production Captain")
        );
        registry.clear_active(&second.id).unwrap();
        assert_eq!(registry.active_profile().unwrap(), None);
    }

    #[test]
    fn an_unknown_or_noncanonical_profile_never_becomes_active() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        registry.create_profile(10).unwrap();
        assert_eq!(
            registry.set_active("00000000-0000-0000-0000-000000000000"),
            Err(ClientProfileRegistryError::ProfileNotFound)
        );
        assert_eq!(
            registry.set_active("NOT-A-UUID"),
            Err(ClientProfileRegistryError::InvalidInput)
        );
    }

    #[test]
    fn invalid_labels_never_change_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        let profile = registry.create_profile(10).unwrap();

        for label in ["", "   ", "bad\nlabel", &"x".repeat(81)] {
            assert_eq!(
                registry.set_label(&profile.id, label),
                Err(ClientProfileRegistryError::InvalidInput)
            );
        }
        assert_eq!(registry.active_profile().unwrap().unwrap().label, None);
    }

    #[test]
    fn an_uncommitted_nonempty_profile_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("console");
        let registry = ClientProfileRegistry::open(&root).unwrap();
        fs::create_dir(
            root.join(PROFILE_DIRECTORY)
                .join(Uuid::new_v4().to_string()),
        )
        .unwrap();
        assert_eq!(
            registry.list(),
            Err(ClientProfileRegistryError::OrphanedProfile)
        );
    }

    #[test]
    fn debug_output_never_exposes_the_registry_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("private-console");
        let registry = ClientProfileRegistry::open(&root).unwrap();
        let rendered = format!("{registry:?}");
        assert!(!rendered.contains("private-console"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_profile_root_is_rejected() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let root = dir.path().join("console");
        fs::create_dir(&root).unwrap();
        symlink(&outside, root.join(PROFILE_DIRECTORY)).unwrap();
        assert!(matches!(
            ClientProfileRegistry::open(root),
            Err(ClientProfileRegistryError::UnsafePath)
        ));
    }
}
