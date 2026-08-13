use super::*;
use crate::NodeProxyMode;

fn config(workspace: &std::path::Path) -> NodeLocalConfig {
    NodeLocalConfig::new(
        "Office Node",
        "test-platform",
        NodeNetworkConfig {
            hub_url: "https://hub.example.com".to_string(),
            proxy: NodeProxyMode::Environment,
            enterprise_ca_bundle: None,
            connect_timeout_secs: 15,
            request_timeout_secs: 45,
        },
        vec![NodeLocalWorkspace {
            workspace_id: "project-main".to_string(),
            label: "Main Project".to_string(),
            root: workspace.to_path_buf(),
            read_only: false,
        }],
        true,
    )
    .unwrap()
}

#[test]
fn local_config_round_trips_privately_and_builds_exact_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("private-workspace");
    fs::create_dir(&workspace).unwrap();
    let store = NodeLocalConfigStore::open(temp.path().join("node")).unwrap();
    let config = config(&workspace);
    store.save(&config).unwrap();
    let recovered = store.load().unwrap().unwrap();
    assert_eq!(recovered, config);

    let capabilities = recovered.capabilities("0.1.0-alpha.14");
    capabilities.validate().unwrap();
    assert_eq!(capabilities.tool_families, ["file", "shell-process"]);
    assert_eq!(capabilities.workspaces[0].workspace_id, "project-main");
    assert_eq!(recovered.requested_grants().workspace_ids, ["project-main"]);
    assert!(recovered
        .execution_policy(recovered.requested_grants())
        .is_ok());

    let secret = workspace.to_string_lossy();
    assert!(!format!("{recovered:?}").contains(secret.as_ref()));
    assert!(!format!("{store:?}").contains(temp.path().to_string_lossy().as_ref()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(store.root()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.root().join(NODE_LOCAL_CONFIG_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn local_config_accepts_mixed_workspace_authority_and_rejects_excessive_grants() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut invalid = config(&workspace);
    invalid.workspaces[0].read_only = true;
    assert!(invalid.execution_policy(invalid.requested_grants()).is_ok());

    let config = config(&workspace);
    let reduced = DeviceGrant {
        workspace_ids: Vec::new(),
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    };
    assert!(config.execution_policy(reduced).is_ok());
    let excessive = DeviceGrant {
        workspace_ids: vec!["other".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    };
    assert!(matches!(
        config.execution_policy(excessive),
        Err(NodeLocalConfigError::GrantInvalid)
    ));
}

#[test]
fn local_config_corruption_and_symlinks_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let store = NodeLocalConfigStore::open(&root).unwrap();
    fs::write(root.join(NODE_LOCAL_CONFIG_FILE), b"not = [valid").unwrap();
    assert!(matches!(
        store.load(),
        Err(NodeLocalConfigError::StateCorrupt)
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let unsafe_root = temp.path().join("unsafe-node");
        symlink(&target, &unsafe_root).unwrap();
        assert!(matches!(
            NodeLocalConfigStore::open(unsafe_root),
            Err(NodeLocalConfigError::UnsafePath)
        ));
    }
}

#[test]
fn an_ungranted_offline_workspace_does_not_block_the_approved_scope() {
    let temp = tempfile::tempdir().unwrap();
    let active = temp.path().join("active");
    let offline = temp.path().join("offline");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&offline).unwrap();
    let config = NodeLocalConfig::new(
        "Office Node",
        "test-platform",
        NodeNetworkConfig::new("https://hub.example.com"),
        vec![
            NodeLocalWorkspace {
                workspace_id: "active".to_string(),
                label: "Active".to_string(),
                root: active,
                read_only: true,
            },
            NodeLocalWorkspace {
                workspace_id: "offline".to_string(),
                label: "Offline".to_string(),
                root: offline.clone(),
                read_only: true,
            },
        ],
        false,
    )
    .unwrap();
    fs::remove_dir(offline).unwrap();
    let approved = DeviceGrant {
        workspace_ids: vec!["active".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    };
    assert!(config.execution_policy(approved).is_ok());
}
