use super::{CapabilityRegistry, CapabilitySlot};
use crate::store::{now, CapabilityStore, RevisionDecision, StoredSlot};
use crate::{
    compile_named, CapabilityScope, CapabilityStatus, CompiledCapability, RegistryError,
    ReloadIssue, ReloadReport, MAX_SOURCE_BYTES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

impl CapabilityRegistry {
    pub(super) fn reload_scope(
        &self,
        scope: &CapabilityScope,
    ) -> Result<ReloadReport, RegistryError> {
        let root = self.root(scope)?;
        let discovered = discover_sources(&root.path)?;
        let mut report = ReloadReport::default();
        let mut seen = BTreeSet::new();
        let mut state = self.state.write().map_err(|_| RegistryError::Poisoned)?;
        let mut store = self.store.lock().map_err(|_| RegistryError::Poisoned)?;

        for source in discovered {
            report.discovered += 1;
            seen.insert(source.name.clone());
            let key = (scope.key(), source.name.clone());
            let prior = state.slots.get(&key).cloned();
            match source.contents {
                Ok(contents) => match crate::parse(&contents)
                    .and_then(|raw| compile_named(&contents, raw, Some(&source.name)))
                {
                    Ok(compiled) => {
                        let compiled_for_store = compiled.clone();
                        let (slot, outcome) = activate_valid(
                            scope,
                            source.path.clone(),
                            prior.as_ref(),
                            compiled,
                            store.decision(&scope.key(), &source.name, &source.hash)?,
                        );
                        store.save_revision_and_slot(
                            &scope.key(),
                            &contents,
                            &compiled_for_store,
                            &slot.stored(&source.name),
                        )?;
                        apply_outcome(&mut report, outcome);
                        state.slots.insert(key, slot);
                    }
                    Err(error) => {
                        retain_invalid(
                            &mut state,
                            &mut store,
                            &mut report,
                            scope,
                            source.name,
                            source.path,
                            prior.as_ref(),
                            error.to_string(),
                        )?;
                    }
                },
                Err(message) => {
                    retain_invalid(
                        &mut state,
                        &mut store,
                        &mut report,
                        scope,
                        source.name,
                        source.path,
                        prior.as_ref(),
                        message,
                    )?;
                }
            }
        }

        disable_missing(scope, &seen, &mut state, &mut store, &mut report)?;
        Ok(report)
    }
}

fn retain_invalid(
    state: &mut super::RegistryState,
    store: &mut CapabilityStore,
    report: &mut ReloadReport,
    scope: &CapabilityScope,
    name: String,
    source_path: PathBuf,
    prior: Option<&CapabilitySlot>,
    message: String,
) -> Result<(), RegistryError> {
    let key = (scope.key(), name.clone());
    let slot = invalid_slot(scope, source_path.clone(), prior, &message);
    store.save_slot(&slot.stored(&name))?;
    report.issues.push(ReloadIssue {
        source_path,
        message,
        retained_active_revision: slot.active.is_some(),
    });
    if slot.active.is_some() {
        report.retained += 1;
    }
    state.slots.insert(key, slot);
    Ok(())
}

fn disable_missing(
    scope: &CapabilityScope,
    seen: &BTreeSet<String>,
    state: &mut super::RegistryState,
    store: &mut CapabilityStore,
    report: &mut ReloadReport,
) -> Result<(), RegistryError> {
    let missing: Vec<(String, String)> = state
        .slots
        .keys()
        .filter(|(scope_key, name)| scope_key == &scope.key() && !seen.contains(name))
        .cloned()
        .collect();
    for key in missing {
        let name = key.1.clone();
        let prior = state.slots.get(&key).expect("slot key exists");
        if prior.status == CapabilityStatus::Disabled {
            continue;
        }
        let slot = CapabilitySlot {
            scope: scope.clone(),
            source_path: prior.source_path.clone(),
            status: CapabilityStatus::Disabled,
            active: None,
            pending: None,
            last_error: None,
            updated_at: now(),
        };
        store.save_slot(&slot.stored(&name))?;
        state.slots.insert(key, slot);
        report.disabled += 1;
    }
    Ok(())
}

#[derive(Debug)]
struct DiscoveredSource {
    name: String,
    path: PathBuf,
    hash: String,
    contents: Result<String, String>,
}

#[derive(Debug, Clone, Copy)]
enum ActivationOutcome {
    Activated,
    Pending,
    Retained,
}

fn discover_sources(root: &Path) -> Result<Vec<DiscoveredSource>, RegistryError> {
    let metadata = std::fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RegistryError::InvalidSourcePath(root.display().to_string()));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("captain") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let contents = read_source_no_follow(&path).map_err(|error| error.to_string());
        let hash = contents
            .as_ref()
            .map(|contents| blake3::hash(contents.as_bytes()).to_hex().to_string())
            .unwrap_or_default();
        entries.push(DiscoveredSource {
            name: name.to_string(),
            path,
            hash,
            contents,
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

fn read_source_no_follow(path: &Path) -> Result<String, std::io::Error> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CapSpec source must be a regular file, not a symlink",
        ));
    }
    let file = open_no_follow(path)?;
    let mut contents = String::new();
    file.take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_string(&mut contents)?;
    Ok(contents)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<File, std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> Result<File, std::io::Error> {
    OpenOptions::new().read(true).open(path)
}

fn activate_valid(
    scope: &CapabilityScope,
    source_path: PathBuf,
    prior: Option<&CapabilitySlot>,
    compiled: CompiledCapability,
    decision: RevisionDecision,
) -> (CapabilitySlot, ActivationOutcome) {
    let compiled = Arc::new(compiled);
    let active = prior.and_then(|slot| slot.active.clone());
    let same_as_active = active
        .as_ref()
        .is_some_and(|current| current.source_hash == compiled.source_hash);
    let can_autoactivate = active
        .as_ref()
        .is_some_and(|current| compiled.permissions.is_subset_of(&current.permissions));

    let (status, active, pending, outcome) = if same_as_active
        || decision == RevisionDecision::Approved
        || (!compiled.requires_human_approval() && active.is_none())
        || can_autoactivate
    {
        (
            CapabilityStatus::Operational,
            Some(compiled),
            None,
            ActivationOutcome::Activated,
        )
    } else if decision == RevisionDecision::Rejected {
        let status = if active.is_some() {
            CapabilityStatus::UpdateRejected
        } else {
            CapabilityStatus::Rejected
        };
        (status, active, None, ActivationOutcome::Retained)
    } else {
        let status = if active.is_some() {
            CapabilityStatus::UpdatePendingApproval
        } else {
            CapabilityStatus::PendingApproval
        };
        (status, active, Some(compiled), ActivationOutcome::Pending)
    };
    (
        CapabilitySlot {
            scope: scope.clone(),
            source_path,
            status,
            active,
            pending,
            last_error: None,
            updated_at: now(),
        },
        outcome,
    )
}

fn invalid_slot(
    scope: &CapabilityScope,
    source_path: PathBuf,
    prior: Option<&CapabilitySlot>,
    message: &str,
) -> CapabilitySlot {
    let active = prior.and_then(|slot| slot.active.clone());
    CapabilitySlot {
        scope: scope.clone(),
        source_path,
        status: if active.is_some() {
            CapabilityStatus::InvalidUpdateRetained
        } else {
            CapabilityStatus::Invalid
        },
        active,
        pending: None,
        last_error: Some(message.to_string()),
        updated_at: now(),
    }
}

pub(super) fn hydrate_slots(
    store: &CapabilityStore,
) -> Result<BTreeMap<(String, String), CapabilitySlot>, RegistryError> {
    let mut slots = BTreeMap::new();
    for stored in store.load_slots()? {
        let active = load_compiled(store, &stored, stored.active_hash.as_deref())?;
        let pending = load_compiled(store, &stored, stored.pending_hash.as_deref())?;
        let scope = scope_from_key(&stored.scope_key)?;
        slots.insert(
            (stored.scope_key.clone(), stored.name.clone()),
            CapabilitySlot {
                scope,
                source_path: PathBuf::from(stored.source_path),
                status: stored.status,
                active,
                pending,
                last_error: stored.last_error,
                updated_at: stored.updated_at,
            },
        );
    }
    Ok(slots)
}

fn load_compiled(
    store: &CapabilityStore,
    slot: &StoredSlot,
    source_hash: Option<&str>,
) -> Result<Option<Arc<CompiledCapability>>, RegistryError> {
    let Some(source_hash) = source_hash else {
        return Ok(None);
    };
    store
        .load_revision(&slot.scope_key, &slot.name, source_hash)?
        .map(|revision| Arc::new(revision.compiled))
        .ok_or_else(|| RegistryError::RevisionNotFound {
            scope: slot.scope_key.clone(),
            name: slot.name.clone(),
            source_hash: source_hash.to_string(),
        })
        .map(Some)
}

fn scope_from_key(key: &str) -> Result<CapabilityScope, RegistryError> {
    if key == "global" {
        return Ok(CapabilityScope::Global);
    }
    let workspace = key.strip_prefix("project:").ok_or_else(|| {
        RegistryError::InvalidPersistedState(format!("unknown scope key '{key}'"))
    })?;
    if workspace.is_empty() {
        return Err(RegistryError::InvalidPersistedState(
            "empty project workspace".to_string(),
        ));
    }
    Ok(CapabilityScope::Project(PathBuf::from(workspace)))
}

fn apply_outcome(report: &mut ReloadReport, outcome: ActivationOutcome) {
    match outcome {
        ActivationOutcome::Activated => report.activated += 1,
        ActivationOutcome::Pending => report.pending_approval += 1,
        ActivationOutcome::Retained => report.retained += 1,
    }
}
