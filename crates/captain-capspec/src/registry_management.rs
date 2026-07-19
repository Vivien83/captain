use super::{ensure_direct_child, CapabilityRegistry, CapabilitySlot};
use crate::store::now;
use crate::{CapabilityScope, CapabilityStatus, CapabilityView, RegistryError};
use std::path::PathBuf;

impl CapabilityRegistry {
    pub fn revision_source(
        &self,
        scope: &CapabilityScope,
        name: &str,
        source_hash: &str,
    ) -> Result<String, RegistryError> {
        self.store
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .load_revision(&scope.key(), name, source_hash)?
            .map(|revision| revision.source_text)
            .ok_or_else(|| RegistryError::RevisionNotFound {
                scope: scope.key(),
                name: name.to_string(),
                source_hash: source_hash.to_string(),
            })
    }

    pub fn capability(
        &self,
        scope: &CapabilityScope,
        name: &str,
    ) -> Result<CapabilityView, RegistryError> {
        self.view(scope, name)
    }

    /// Resolve a direct source path only after revalidating the registered root.
    pub fn source_path_for_mutation(
        &self,
        scope: &CapabilityScope,
        name: &str,
    ) -> Result<PathBuf, RegistryError> {
        let root = self.root(scope)?;
        let metadata = std::fs::symlink_metadata(&root.path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RegistryError::InvalidSourcePath(
                root.path.display().to_string(),
            ));
        }
        let canonical_root = root.path.canonicalize()?;
        if canonical_root != root.path
            || matches!(scope, CapabilityScope::Project(workspace) if !canonical_root.starts_with(workspace))
        {
            return Err(RegistryError::InvalidSourcePath(
                root.path.display().to_string(),
            ));
        }
        let source_path = root.path.join(format!("{name}.captain"));
        ensure_direct_child(&root.path, &source_path)?;
        Ok(source_path)
    }

    pub fn approve(
        &self,
        scope: &CapabilityScope,
        name: &str,
        expected_hash: &str,
        actor: &str,
    ) -> Result<CapabilityView, RegistryError> {
        self.decide(scope, name, expected_hash, actor, true)
    }

    pub fn reject(
        &self,
        scope: &CapabilityScope,
        name: &str,
        expected_hash: &str,
        actor: &str,
    ) -> Result<CapabilityView, RegistryError> {
        self.decide(scope, name, expected_hash, actor, false)
    }

    pub fn rollback(
        &self,
        scope: &CapabilityScope,
        name: &str,
        target_hash: &str,
        actor: &str,
    ) -> Result<CapabilityView, RegistryError> {
        if actor.trim().is_empty() {
            return Err(RegistryError::EmptyActor);
        }
        let revision = self
            .store
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .load_revision(&scope.key(), name, target_hash)?
            .ok_or_else(|| RegistryError::RevisionNotFound {
                scope: scope.key(),
                name: name.to_string(),
                source_hash: target_hash.to_string(),
            })?;
        let source_path = self.source_path_for_mutation(scope, name)?;
        captain_types::durable_fs::atomic_write(&source_path, revision.source_text.as_bytes())?;
        self.store
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .mark_approved(&scope.key(), name, target_hash, actor)?;
        self.reload_scope(scope)?;
        self.view(scope, name)
    }

    pub fn remove_source(
        &self,
        scope: &CapabilityScope,
        name: &str,
    ) -> Result<(bool, CapabilityView), RegistryError> {
        self.view(scope, name)?;
        let source_path = self.source_path_for_mutation(scope, name)?;
        let removed = captain_types::durable_fs::remove_file(&source_path)?;
        self.reload_scope(scope)?;
        Ok((removed, self.view(scope, name)?))
    }

    fn decide(
        &self,
        scope: &CapabilityScope,
        name: &str,
        expected_hash: &str,
        actor: &str,
        approve: bool,
    ) -> Result<CapabilityView, RegistryError> {
        if actor.trim().is_empty() {
            return Err(RegistryError::EmptyActor);
        }
        let key = (scope.key(), name.to_string());
        let mut state = self.state.write().map_err(|_| RegistryError::Poisoned)?;
        let prior =
            state
                .slots
                .get(&key)
                .cloned()
                .ok_or_else(|| RegistryError::CapabilityNotFound {
                    scope: scope.key(),
                    name: name.to_string(),
                })?;
        let pending = prior
            .pending
            .as_ref()
            .ok_or_else(|| RegistryError::CapabilityNotFound {
                scope: scope.key(),
                name: name.to_string(),
            })?;
        if pending.source_hash != expected_hash {
            return Err(RegistryError::PendingHashMismatch {
                name: name.to_string(),
                expected: pending.source_hash.clone(),
                actual: expected_hash.to_string(),
            });
        }

        let slot = if approve {
            CapabilitySlot {
                scope: scope.clone(),
                source_path: prior.source_path,
                status: CapabilityStatus::Operational,
                active: prior.pending,
                pending: None,
                last_error: None,
                updated_at: now(),
            }
        } else {
            CapabilitySlot {
                scope: scope.clone(),
                source_path: prior.source_path,
                status: if prior.active.is_some() {
                    CapabilityStatus::UpdateRejected
                } else {
                    CapabilityStatus::Rejected
                },
                active: prior.active,
                pending: None,
                last_error: None,
                updated_at: now(),
            }
        };
        let stored = slot.stored(name);
        let mut store = self.store.lock().map_err(|_| RegistryError::Poisoned)?;
        if approve {
            store.approve_revision_and_slot(&scope.key(), name, expected_hash, actor, &stored)?;
        } else {
            store.reject_revision_and_slot(&scope.key(), name, expected_hash, actor, &stored)?;
        }
        let view = slot.view(name);
        state.slots.insert(key, slot);
        Ok(view)
    }
}
