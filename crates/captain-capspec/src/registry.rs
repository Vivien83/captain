use crate::store::{CapabilityStore, StoredRevision, StoredSlot};
use crate::{
    CapabilityScope, CapabilityStatus, CapabilityView, CompiledCapability, RegistryError,
    ReloadReport, RevisionInfo,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

#[path = "registry_management.rs"]
mod management;
#[path = "registry_reload.rs"]
mod reload;

#[derive(Debug, Clone)]
pub struct ResolvedCapability {
    pub scope: CapabilityScope,
    pub compiled: Arc<CompiledCapability>,
}

#[derive(Debug, Clone)]
struct SourceRoot {
    scope: CapabilityScope,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct CapabilitySlot {
    scope: CapabilityScope,
    source_path: PathBuf,
    status: CapabilityStatus,
    active: Option<Arc<CompiledCapability>>,
    pending: Option<Arc<CompiledCapability>>,
    last_error: Option<String>,
    updated_at: String,
}

impl CapabilitySlot {
    fn view(&self, name: &str) -> CapabilityView {
        CapabilityView {
            scope: self.scope.clone(),
            name: name.to_string(),
            tool_name: self
                .active
                .as_ref()
                .or(self.pending.as_ref())
                .map(|compiled| compiled.tool_name.clone()),
            source_path: self.source_path.clone(),
            status: self.status,
            active_hash: self
                .active
                .as_ref()
                .map(|compiled| compiled.source_hash.clone()),
            pending_hash: self
                .pending
                .as_ref()
                .map(|compiled| compiled.source_hash.clone()),
            last_error: self.last_error.clone(),
            updated_at: self.updated_at.clone(),
        }
    }

    fn stored(&self, name: &str) -> StoredSlot {
        let view = self.view(name);
        StoredSlot {
            scope_key: self.scope.key(),
            name: name.to_string(),
            source_path: self.source_path.to_string_lossy().into_owned(),
            status: self.status,
            active_hash: view.active_hash,
            pending_hash: view.pending_hash,
            last_error: self.last_error.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

#[derive(Default)]
struct RegistryState {
    roots: BTreeMap<String, SourceRoot>,
    slots: BTreeMap<(String, String), CapabilitySlot>,
}

pub struct CapabilityRegistry {
    state: RwLock<RegistryState>,
    store: Mutex<CapabilityStore>,
}

impl CapabilityRegistry {
    pub fn open(global_root: &Path, database_path: &Path) -> Result<Self, RegistryError> {
        captain_types::durable_fs::create_dir_all(global_root)?;
        ensure_real_directory(global_root)?;
        let global_root = global_root.canonicalize()?;
        let store = CapabilityStore::open(database_path)?;
        let slots = reload::hydrate_slots(&store)?;
        let global = SourceRoot {
            scope: CapabilityScope::Global,
            path: global_root,
        };
        let state = RegistryState {
            roots: BTreeMap::from([(global.scope.key(), global)]),
            slots,
        };
        let registry = Self {
            state: RwLock::new(state),
            store: Mutex::new(store),
        };
        registry.reload_scope(&CapabilityScope::Global)?;
        Ok(registry)
    }

    pub fn register_project(
        &self,
        workspace: &Path,
    ) -> Result<(CapabilityScope, ReloadReport), RegistryError> {
        let workspace = workspace.canonicalize()?;
        let scope = CapabilityScope::Project(workspace.clone());
        let captain_root = workspace.join(".captain");
        let path = captain_root.join("capabilities");
        ensure_real_directory(&captain_root)?;
        ensure_real_directory(&path)?;
        let canonical_root = path.canonicalize()?;
        if !canonical_root.starts_with(&workspace) {
            return Err(RegistryError::InvalidSourcePath(
                canonical_root.display().to_string(),
            ));
        }
        self.state
            .write()
            .map_err(|_| RegistryError::Poisoned)?
            .roots
            .insert(
                scope.key(),
                SourceRoot {
                    scope: scope.clone(),
                    path: canonical_root,
                },
            );
        match self.reload_scope(&scope) {
            Ok(report) => Ok((scope, report)),
            Err(error) => {
                self.state
                    .write()
                    .map_err(|_| RegistryError::Poisoned)?
                    .roots
                    .remove(&scope.key());
                Err(error)
            }
        }
    }

    pub fn reload_global(&self) -> Result<ReloadReport, RegistryError> {
        self.reload_scope(&CapabilityScope::Global)
    }

    pub fn reload_all(&self) -> Result<ReloadReport, RegistryError> {
        let scopes: Vec<CapabilityScope> = self
            .state
            .read()
            .map_err(|_| RegistryError::Poisoned)?
            .roots
            .values()
            .map(|root| root.scope.clone())
            .collect();
        let mut report = ReloadReport::default();
        for scope in scopes {
            report.merge(self.reload_scope(&scope)?);
        }
        Ok(report)
    }

    pub fn list(&self) -> Result<Vec<CapabilityView>, RegistryError> {
        Ok(self
            .state
            .read()
            .map_err(|_| RegistryError::Poisoned)?
            .slots
            .iter()
            .map(|((_, name), slot)| slot.view(name))
            .collect())
    }

    pub fn source_roots(&self) -> Result<Vec<(CapabilityScope, PathBuf)>, RegistryError> {
        Ok(self
            .state
            .read()
            .map_err(|_| RegistryError::Poisoned)?
            .roots
            .values()
            .map(|root| (root.scope.clone(), root.path.clone()))
            .collect())
    }

    pub fn scope_registered(&self, scope: &CapabilityScope) -> Result<bool, RegistryError> {
        Ok(self
            .state
            .read()
            .map_err(|_| RegistryError::Poisoned)?
            .roots
            .contains_key(&scope.key()))
    }

    pub fn revisions(
        &self,
        scope: &CapabilityScope,
        name: &str,
    ) -> Result<Vec<RevisionInfo>, RegistryError> {
        let stored = self
            .store
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .list_revisions(&scope.key(), name)?;
        Ok(stored
            .into_iter()
            .map(|revision| revision_info(scope, revision))
            .collect())
    }

    pub fn active_capabilities(
        &self,
        workspace: Option<&Path>,
    ) -> Result<Vec<Arc<CompiledCapability>>, RegistryError> {
        Ok(self
            .active_resolved(workspace)?
            .into_iter()
            .map(|resolved| resolved.compiled)
            .collect())
    }

    pub fn active_resolved(
        &self,
        workspace: Option<&Path>,
    ) -> Result<Vec<ResolvedCapability>, RegistryError> {
        let state = self.state.read().map_err(|_| RegistryError::Poisoned)?;
        let mut selected = BTreeMap::<String, ResolvedCapability>::new();
        let global_key = CapabilityScope::Global.key();
        for ((scope_key, name), slot) in &state.slots {
            if scope_key == &global_key {
                if let Some(active) = &slot.active {
                    selected.insert(
                        name.clone(),
                        ResolvedCapability {
                            scope: CapabilityScope::Global,
                            compiled: Arc::clone(active),
                        },
                    );
                }
            }
        }

        if let Some(workspace) = canonical_existing(workspace) {
            let project_key = CapabilityScope::Project(workspace).key();
            if !state.roots.contains_key(&project_key) {
                return Ok(selected.into_values().collect());
            }
            for ((scope_key, name), slot) in &state.slots {
                if scope_key != &project_key || slot.status == CapabilityStatus::Disabled {
                    continue;
                }
                match &slot.active {
                    Some(active) => {
                        selected.insert(
                            name.clone(),
                            ResolvedCapability {
                                scope: slot.scope.clone(),
                                compiled: Arc::clone(active),
                            },
                        );
                    }
                    None => {
                        selected.remove(name);
                    }
                }
            }
        }
        Ok(selected.into_values().collect())
    }

    pub fn active_by_tool(
        &self,
        tool_name: &str,
        workspace: Option<&Path>,
    ) -> Result<Option<Arc<CompiledCapability>>, RegistryError> {
        Ok(self
            .active_capabilities(workspace)?
            .into_iter()
            .find(|compiled| compiled.tool_name == tool_name))
    }

    pub fn resolved_by_tool(
        &self,
        tool_name: &str,
        workspace: Option<&Path>,
    ) -> Result<Option<ResolvedCapability>, RegistryError> {
        Ok(self
            .active_resolved(workspace)?
            .into_iter()
            .find(|resolved| resolved.compiled.tool_name == tool_name))
    }

    pub fn compiled_revision(
        &self,
        scope: &CapabilityScope,
        name: &str,
        source_hash: &str,
    ) -> Result<Option<Arc<CompiledCapability>>, RegistryError> {
        Ok(self
            .store
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .load_revision(&scope.key(), name, source_hash)?
            .map(|revision| Arc::new(revision.compiled)))
    }

    fn view(&self, scope: &CapabilityScope, name: &str) -> Result<CapabilityView, RegistryError> {
        self.state
            .read()
            .map_err(|_| RegistryError::Poisoned)?
            .slots
            .get(&(scope.key(), name.to_string()))
            .map(|slot| slot.view(name))
            .ok_or_else(|| RegistryError::CapabilityNotFound {
                scope: scope.key(),
                name: name.to_string(),
            })
    }

    fn root(&self, scope: &CapabilityScope) -> Result<SourceRoot, RegistryError> {
        self.state
            .read()
            .map_err(|_| RegistryError::Poisoned)?
            .roots
            .get(&scope.key())
            .cloned()
            .ok_or_else(|| RegistryError::UnknownScope(scope.key()))
    }
}

fn revision_info(scope: &CapabilityScope, revision: StoredRevision) -> RevisionInfo {
    debug_assert_eq!(revision.scope_key, scope.key());
    RevisionInfo {
        scope: scope.clone(),
        name: revision.name,
        source_hash: revision.source_hash,
        version: revision.compiled.version,
        permission_fingerprint: revision.permission_fingerprint,
        created_at: revision.created_at,
        approved_by: revision.approved_by,
        approved_at: revision.approved_at,
        rejected_by: revision.rejected_by,
        rejected_at: revision.rejected_at,
    }
}

fn canonical_existing(path: Option<&Path>) -> Option<PathBuf> {
    path.and_then(|path| path.canonicalize().ok())
}

fn ensure_direct_child(root: &Path, path: &Path) -> Result<(), RegistryError> {
    if path.parent() == Some(root)
        && path.extension().and_then(|value| value.to_str()) == Some("captain")
    {
        Ok(())
    } else {
        Err(RegistryError::InvalidSourcePath(path.display().to_string()))
    }
}

fn ensure_real_directory(path: &Path) -> Result<(), RegistryError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(RegistryError::InvalidSourcePath(path.display().to_string()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            captain_types::durable_fs::create_dir_all(path)?;
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                Err(RegistryError::InvalidSourcePath(path.display().to_string()))
            } else {
                Ok(())
            }
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
