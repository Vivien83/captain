use super::{
    set_private_dir, validate_manifest, ClientProfileEntry, ClientProfileRegistry,
    ClientProfileRegistryError, PendingLegacyImport, PersistedProfileEntry,
    PersistedProfileRegistry, MAX_PROFILES,
};
use captain_types::durable_fs;
use std::{fs, path::Path};
use uuid::Uuid;

impl ClientProfileRegistry {
    /// Import the one-profile Alpha.14 layout without copying its credential.
    ///
    /// A pending record is committed before the directory rename. Repeating
    /// this method completes either side of an interrupted rename and never
    /// deletes an ambiguous source or destination.
    pub fn import_legacy_profile(
        &self,
        legacy_root: impl AsRef<Path>,
        created_at_ms: i64,
    ) -> Result<Option<ClientProfileEntry>, ClientProfileRegistryError> {
        if created_at_ms < 0 {
            return Err(ClientProfileRegistryError::InvalidInput);
        }
        let legacy_root = legacy_root.as_ref();
        if legacy_root.starts_with(&self.root) || self.root.starts_with(legacy_root) {
            return Err(ClientProfileRegistryError::LegacyImportConflict);
        }

        self.with_lock(|manifest| {
            validate_manifest(manifest)?;
            self.validate_filesystem(manifest)?;
            if let Some(pending) = manifest.pending_legacy_import.clone() {
                return self.resume_legacy_import(manifest, &pending, legacy_root);
            }
            if !legacy_profile_exists(legacy_root)? {
                return Ok(None);
            }
            if !manifest.profiles.is_empty() || manifest.profiles.len() >= MAX_PROFILES {
                return Err(ClientProfileRegistryError::LegacyImportConflict);
            }

            let pending = PendingLegacyImport {
                profile_id: Uuid::new_v4().to_string(),
                created_at_ms,
            };
            manifest.pending_legacy_import = Some(pending.clone());
            self.save_manifest(manifest)?;
            self.resume_legacy_import(manifest, &pending, legacy_root)
        })
    }

    fn resume_legacy_import(
        &self,
        manifest: &mut PersistedProfileRegistry,
        pending: &PendingLegacyImport,
        legacy_root: &Path,
    ) -> Result<Option<ClientProfileEntry>, ClientProfileRegistryError> {
        let destination = self.profiles_root.join(&pending.profile_id);
        let source_exists = legacy_profile_exists(legacy_root)?;
        let destination_exists = profile_directory_exists(&destination)?;
        match (source_exists, destination_exists) {
            (true, false) => {
                durable_fs::rename_noclobber(legacy_root, &destination)
                    .map_err(|_| ClientProfileRegistryError::StateUnavailable)?;
                set_private_dir(&destination)?;
            }
            (false, true) => {}
            _ => return Err(ClientProfileRegistryError::LegacyImportConflict),
        }

        let entry = PersistedProfileEntry {
            id: pending.profile_id.clone(),
            created_at_ms: pending.created_at_ms,
            label: None,
        };
        manifest.profiles.push(entry.clone());
        manifest.active_profile = Some(entry.id.clone());
        manifest.pending_legacy_import = None;
        self.save_manifest(manifest)?;
        Ok(Some(ClientProfileEntry {
            id: entry.id,
            created_at_ms: entry.created_at_ms,
            active: true,
            label: None,
        }))
    }
}

fn legacy_profile_exists(path: &Path) -> Result<bool, ClientProfileRegistryError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(ClientProfileRegistryError::StateUnavailable),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ClientProfileRegistryError::UnsafePath);
    }
    let config = path.join("config.toml");
    let metadata = fs::symlink_metadata(config)
        .map_err(|_| ClientProfileRegistryError::LegacyImportConflict)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ClientProfileRegistryError::UnsafePath);
    }
    Ok(true)
}

fn profile_directory_exists(path: &Path) -> Result<bool, ClientProfileRegistryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ClientProfileRegistryError::UnsafePath)
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ClientProfileRegistryError::StateUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(root: &Path) -> std::path::PathBuf {
        let path = root.join("client");
        durable_fs::create_dir_all(&path).unwrap();
        durable_fs::atomic_write(
            &path.join("config.toml"),
            b"schema_version = 1\ndisplay_name = 'Legacy'\n",
        )
        .unwrap();
        durable_fs::atomic_write(&path.join("pairing.json"), b"secret-state").unwrap();
        path
    }

    #[test]
    fn legacy_import_moves_one_profile_without_copying_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = legacy(dir.path());
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();

        let imported = registry
            .import_legacy_profile(&legacy, 42)
            .unwrap()
            .unwrap();
        assert!(!legacy.exists());
        let root = registry.profile_root(&imported.id).unwrap();
        assert_eq!(
            fs::read(root.join("pairing.json")).unwrap(),
            b"secret-state"
        );
        assert_eq!(registry.list().unwrap(), vec![imported]);
    }

    #[test]
    fn migration_resumes_after_the_directory_move() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = legacy(dir.path());
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        let pending = PendingLegacyImport {
            profile_id: Uuid::new_v4().to_string(),
            created_at_ms: 77,
        };
        registry
            .with_lock(|manifest| {
                manifest.pending_legacy_import = Some(pending.clone());
                registry.save_manifest(manifest)
            })
            .unwrap();
        let destination = registry.profiles_root.join(&pending.profile_id);
        durable_fs::rename_noclobber(&legacy, &destination).unwrap();

        let resumed = registry
            .import_legacy_profile(&legacy, 999)
            .unwrap()
            .unwrap();
        assert_eq!(resumed.id, pending.profile_id);
        assert_eq!(resumed.created_at_ms, 77);
        assert_eq!(registry.active_profile().unwrap(), Some(resumed));
    }

    #[test]
    fn ambiguous_migration_never_deletes_either_copy() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = legacy(dir.path());
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        let pending = PendingLegacyImport {
            profile_id: Uuid::new_v4().to_string(),
            created_at_ms: 77,
        };
        registry
            .with_lock(|manifest| {
                manifest.pending_legacy_import = Some(pending.clone());
                registry.save_manifest(manifest)
            })
            .unwrap();
        let destination = registry.profiles_root.join(&pending.profile_id);
        durable_fs::create_dir_all(&destination).unwrap();
        durable_fs::atomic_write(&destination.join("config.toml"), b"other").unwrap();

        assert_eq!(
            registry.import_legacy_profile(&legacy, 999),
            Err(ClientProfileRegistryError::LegacyImportConflict)
        );
        assert!(legacy.join("pairing.json").exists());
        assert!(destination.join("config.toml").exists());
    }
}
