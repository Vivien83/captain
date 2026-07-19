use super::*;
use crate::store::CapabilityStore;
use std::fs;
use tempfile::TempDir;

fn read_source(name: &str, version: &str) -> String {
    format!(
        r#"format = 1
name = "{name}"
description = "Read a bounded project file for revision {version}."
version = "{version}"

[permissions]
tools = ["file_read"]
read_paths = ["/tmp/**"]

[[steps]]
id = "read"
tool = "file_read"
with = {{ path = "/tmp/input.txt" }}
"#
    )
}

fn write_source(name: &str, version: &str, path: &str) -> String {
    format!(
        r#"format = 1
name = "{name}"
description = "Write one approved file for revision {version}."
version = "{version}"

[permissions]
tools = ["file_write"]
write_paths = ["{path}"]

[[steps]]
id = "write"
tool = "file_write"
with = {{ path = "{path}", content = "confirmed" }}
"#
    )
}

fn paths(temp: &TempDir) -> (PathBuf, PathBuf) {
    (
        temp.path().join("capabilities"),
        temp.path().join("state").join("capabilities.db"),
    )
}

fn write_capability(root: &Path, name: &str, source: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join(format!("{name}.captain")), source).unwrap();
}

fn only_view(registry: &CapabilityRegistry) -> CapabilityView {
    registry.list().unwrap().into_iter().next().unwrap()
}

#[test]
fn read_only_source_activates_without_approval() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(&root, "reader", &read_source("reader", "1.0.0"));

    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let view = only_view(&registry);
    assert_eq!(view.status, CapabilityStatus::Operational);
    assert!(view.active_hash.is_some());
    assert!(view.pending_hash.is_none());
    assert_eq!(
        registry
            .active_by_tool("cap_reader", None)
            .unwrap()
            .unwrap()
            .version,
        "1.0.0"
    );
}

#[test]
fn dangerous_source_waits_for_exact_human_approval() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(
        &root,
        "writer",
        &write_source("writer", "1.0.0", "/tmp/output.txt"),
    );
    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let pending = only_view(&registry);
    assert_eq!(pending.status, CapabilityStatus::PendingApproval);
    assert!(registry
        .active_by_tool("cap_writer", None)
        .unwrap()
        .is_none());

    let hash = pending.pending_hash.unwrap();
    assert!(matches!(
        registry.approve(&CapabilityScope::Global, "writer", &hash, ""),
        Err(RegistryError::EmptyActor)
    ));
    let error = registry
        .approve(&CapabilityScope::Global, "writer", "wrong", "operator")
        .unwrap_err()
        .to_string();
    assert!(error.contains("expects pending hash"), "{error}");
    let approved = registry
        .approve(&CapabilityScope::Global, "writer", &hash, "operator")
        .unwrap();
    assert_eq!(approved.status, CapabilityStatus::Operational);
    assert_eq!(approved.active_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(
        registry
            .revisions(&CapabilityScope::Global, "writer")
            .unwrap()[0]
            .approved_by
            .as_deref(),
        Some("operator")
    );

    drop(registry);
    let reopened = CapabilityRegistry::open(&root, &database).unwrap();
    let restored = only_view(&reopened);
    assert_eq!(restored.status, CapabilityStatus::Operational);
    assert_eq!(restored.active_hash.as_deref(), Some(hash.as_str()));
}

#[test]
fn revision_source_and_durable_disable_preserve_history() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    let source = read_source("reader", "1.0.0");
    write_capability(&root, "reader", &source);

    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let hash = only_view(&registry).active_hash.unwrap();
    assert_eq!(
        registry
            .revision_source(&CapabilityScope::Global, "reader", &hash)
            .unwrap(),
        source
    );

    let (removed, disabled) = registry
        .remove_source(&CapabilityScope::Global, "reader")
        .unwrap();
    assert!(removed);
    assert_eq!(disabled.status, CapabilityStatus::Disabled);
    assert!(registry
        .active_by_tool("cap_reader", None)
        .unwrap()
        .is_none());
    assert_eq!(
        registry
            .revision_source(&CapabilityScope::Global, "reader", &hash)
            .unwrap(),
        source
    );
}

#[test]
fn invalid_edit_retains_last_active_revision_across_restart() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(&root, "reader", &read_source("reader", "1.0.0"));
    let original_hash = {
        let registry = CapabilityRegistry::open(&root, &database).unwrap();
        let hash = only_view(&registry).active_hash.unwrap();
        write_capability(&root, "reader", "format = 1\nname = [broken");
        let report = registry.reload_global().unwrap();
        assert_eq!(report.retained, 1);
        let view = only_view(&registry);
        assert_eq!(view.status, CapabilityStatus::InvalidUpdateRetained);
        assert_eq!(view.active_hash.as_deref(), Some(hash.as_str()));
        hash
    };

    let reopened = CapabilityRegistry::open(&root, &database).unwrap();
    let view = only_view(&reopened);
    assert_eq!(view.status, CapabilityStatus::InvalidUpdateRetained);
    assert_eq!(view.active_hash.as_deref(), Some(original_hash.as_str()));
}

#[test]
fn permission_expansion_retains_old_revision_until_approved() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(&root, "change", &read_source("change", "1.0.0"));
    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let old_hash = only_view(&registry).active_hash.unwrap();

    write_capability(
        &root,
        "change",
        &write_source("change", "2.0.0", "/tmp/output.txt"),
    );
    registry.reload_global().unwrap();
    let pending = only_view(&registry);
    assert_eq!(pending.status, CapabilityStatus::UpdatePendingApproval);
    assert_eq!(pending.active_hash.as_deref(), Some(old_hash.as_str()));
    let new_hash = pending.pending_hash.unwrap();
    registry
        .approve(&CapabilityScope::Global, "change", &new_hash, "operator")
        .unwrap();
    assert_eq!(
        registry
            .active_by_tool("cap_change", None)
            .unwrap()
            .unwrap()
            .version,
        "2.0.0"
    );
}

#[test]
fn update_inside_approved_authority_activates_without_second_prompt() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(
        &root,
        "writer",
        &write_source("writer", "1.0.0", "/tmp/output.txt"),
    );
    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let hash = only_view(&registry).pending_hash.unwrap();
    registry
        .approve(&CapabilityScope::Global, "writer", &hash, "operator")
        .unwrap();

    write_capability(
        &root,
        "writer",
        &write_source("writer", "1.1.0", "/tmp/output.txt"),
    );
    let report = registry.reload_global().unwrap();
    assert_eq!(report.activated, 1);
    let view = only_view(&registry);
    assert_eq!(view.status, CapabilityStatus::Operational);
    assert!(view.pending_hash.is_none());
}

#[test]
fn delete_disables_but_reinstall_restores_known_revision() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    let source = read_source("reader", "1.0.0");
    write_capability(&root, "reader", &source);
    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let hash = only_view(&registry).active_hash.unwrap();

    fs::remove_file(root.join("reader.captain")).unwrap();
    let report = registry.reload_global().unwrap();
    assert_eq!(report.disabled, 1);
    assert_eq!(only_view(&registry).status, CapabilityStatus::Disabled);
    assert!(registry.active_capabilities(None).unwrap().is_empty());
    assert_eq!(
        registry
            .revisions(&CapabilityScope::Global, "reader")
            .unwrap()
            .len(),
        1
    );

    write_capability(&root, "reader", &source);
    registry.reload_global().unwrap();
    assert_eq!(
        only_view(&registry).active_hash.as_deref(),
        Some(hash.as_str())
    );
}

#[test]
fn rollback_restores_historical_source_and_revision() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    let first = read_source("reader", "1.0.0");
    write_capability(&root, "reader", &first);
    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let first_hash = only_view(&registry).active_hash.unwrap();
    write_capability(&root, "reader", &read_source("reader", "2.0.0"));
    registry.reload_global().unwrap();

    let rolled_back = registry
        .rollback(&CapabilityScope::Global, "reader", &first_hash, "operator")
        .unwrap();
    assert_eq!(
        rolled_back.active_hash.as_deref(),
        Some(first_hash.as_str())
    );
    assert_eq!(
        fs::read_to_string(root.join("reader.captain")).unwrap(),
        first
    );
}

#[test]
fn project_source_overrides_global_only_inside_registered_workspace() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(&root, "shared", &read_source("shared", "global"));
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let project_root = workspace.join(".captain").join("capabilities");
    write_capability(&project_root, "shared", &read_source("shared", "project"));

    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    assert_eq!(
        registry.active_capabilities(None).unwrap()[0].version,
        "global"
    );
    registry.register_project(&workspace).unwrap();
    assert_eq!(
        registry.active_capabilities(Some(&workspace)).unwrap()[0].version,
        "project"
    );
    assert_eq!(
        registry.active_capabilities(None).unwrap()[0].version,
        "global"
    );
}

#[test]
fn rejected_revision_stays_rejected_until_source_changes() {
    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    write_capability(
        &root,
        "writer",
        &write_source("writer", "1.0.0", "/tmp/output.txt"),
    );
    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let hash = only_view(&registry).pending_hash.unwrap();
    registry
        .reject(&CapabilityScope::Global, "writer", &hash, "operator")
        .unwrap();
    registry.reload_global().unwrap();
    let rejected = only_view(&registry);
    assert_eq!(rejected.status, CapabilityStatus::Rejected);
    assert!(rejected.pending_hash.is_none());

    write_capability(
        &root,
        "writer",
        &write_source("writer", "2.0.0", "/tmp/output.txt"),
    );
    registry.reload_global().unwrap();
    assert_eq!(
        only_view(&registry).status,
        CapabilityStatus::PendingApproval
    );
}

#[cfg(unix)]
#[test]
fn symlink_source_fails_closed() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let (root, database) = paths(&temp);
    fs::create_dir_all(&root).unwrap();
    let target = temp.path().join("target.captain");
    fs::write(&target, read_source("linked", "1.0.0")).unwrap();
    symlink(&target, root.join("linked.captain")).unwrap();

    let registry = CapabilityRegistry::open(&root, &database).unwrap();
    let view = only_view(&registry);
    assert_eq!(view.status, CapabilityStatus::Invalid);
    assert!(view.active_hash.is_none());
    assert!(view.last_error.unwrap().contains("must be a regular file"));
}

#[cfg(unix)]
#[test]
fn project_parent_symlink_fails_before_creating_an_external_root() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let external = temp.path().join("external");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&external).unwrap();
    symlink(&external, workspace.join(".captain")).unwrap();
    let (root, database) = paths(&temp);
    let registry = CapabilityRegistry::open(&root, &database).unwrap();

    let error = registry
        .register_project(&workspace)
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid CapSpec source path"), "{error}");
    assert!(!external.join("capabilities").exists());
}

#[cfg(unix)]
#[test]
fn replaced_project_root_cannot_redirect_a_rollback_write() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let project_root = workspace.join(".captain/capabilities");
    let external = temp.path().join("external");
    fs::create_dir_all(&project_root).unwrap();
    fs::create_dir_all(&external).unwrap();
    write_capability(&project_root, "reader", &read_source("reader", "1.0.0"));
    let (global_root, database) = paths(&temp);
    let registry = CapabilityRegistry::open(&global_root, &database).unwrap();
    let (scope, _) = registry.register_project(&workspace).unwrap();
    let hash = registry
        .capability(&scope, "reader")
        .unwrap()
        .active_hash
        .unwrap();

    fs::remove_file(project_root.join("reader.captain")).unwrap();
    fs::remove_dir(&project_root).unwrap();
    symlink(&external, &project_root).unwrap();

    let error = registry
        .rollback(&scope, "reader", &hash, "operator")
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid CapSpec source path"), "{error}");
    assert!(!external.join("reader.captain").exists());
}

#[test]
fn registry_database_uses_production_durability_pragmas() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("state").join("capabilities.db");
    let store = CapabilityStore::open(&database).unwrap();
    let (journal, synchronous, fullfsync, checkpoint_fullfsync) =
        store.durability_settings().unwrap();
    assert_eq!(journal.to_lowercase(), "wal");
    assert_eq!(synchronous, 2);
    assert_eq!(fullfsync, 1);
    assert_eq!(checkpoint_fullfsync, 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(database).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
