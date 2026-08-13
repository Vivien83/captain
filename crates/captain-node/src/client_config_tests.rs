use super::*;
use crate::NodeProxyMode;
use captain_wire::{DeviceGrant, DeviceRole};

fn config() -> ClientLocalConfig {
    ClientLocalConfig::new(
        "Office Client",
        "macos-arm64",
        NodeNetworkConfig {
            hub_url: "https://hub.example.com".to_string(),
            proxy: NodeProxyMode::Environment,
            enterprise_ca_bundle: None,
            connect_timeout_secs: 15,
            request_timeout_secs: 45,
        },
    )
    .unwrap()
}

#[test]
fn client_config_is_private_and_never_announces_execution() {
    let temp = tempfile::tempdir().unwrap();
    let store = ClientLocalConfigStore::open(temp.path().join("client")).unwrap();
    let config = config();
    store.save(&config).unwrap();
    let restored = store.load().unwrap().unwrap();
    assert_eq!(restored, config);

    let claim = restored
        .pairing_profile("0.1.0-alpha.14")
        .claim_for_test("a".repeat(64));
    claim.validate().unwrap();
    assert_eq!(claim.role, DeviceRole::Client);
    assert!(claim.capabilities.workspaces.is_empty());
    assert!(claim.capabilities.tool_families.is_empty());
    assert_eq!(claim.requested_grants, DeviceGrant::default());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(store.root()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.root().join(CLIENT_CONFIG_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn corrupt_or_symlinked_client_config_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("client");
    let store = ClientLocalConfigStore::open(&root).unwrap();
    fs::write(root.join(CLIENT_CONFIG_FILE), b"not = [valid").unwrap();
    assert_eq!(store.load(), Err(ClientLocalConfigError::StateCorrupt));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let unsafe_root = temp.path().join("unsafe-client");
        symlink(&target, &unsafe_root).unwrap();
        assert!(matches!(
            ClientLocalConfigStore::open(unsafe_root),
            Err(ClientLocalConfigError::UnsafePath)
        ));
    }
}
