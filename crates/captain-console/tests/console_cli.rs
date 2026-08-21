use captain_node::{
    ClientLocalConfig, ClientLocalConfigStore, ClientProfileRegistry, NodeNetworkConfig,
};
use std::{path::Path, process::Command};

fn console(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_captain-console"))
        .args(args)
        .env("CAPTAIN_HOME", home)
        .output()
        .unwrap()
}

#[test]
fn version_uses_the_canonical_release_build_version() {
    let home = tempfile::tempdir().unwrap();
    let output = console(home.path(), &["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!(
            "captain-console {}",
            captain_types::version::captain_version()
        )
    );
    assert!(!home.path().join("console").exists());
}

fn configured_profile(home: &Path) -> String {
    let registry = ClientProfileRegistry::open(home.join("console")).unwrap();
    let profile = registry.create_profile(10).unwrap();
    registry.set_label(&profile.id, "Production").unwrap();
    let config = ClientLocalConfig::new(
        "Private Device Name",
        "test-platform",
        NodeNetworkConfig::new("https://private.example"),
    )
    .unwrap();
    ClientLocalConfigStore::open(registry.profile_root(&profile.id).unwrap())
        .unwrap()
        .save(&config)
        .unwrap();
    profile.id
}

#[test]
fn fresh_console_inventory_never_creates_full_runtime_state() {
    let home = tempfile::tempdir().unwrap();
    let output = console(home.path(), &["list", "--local", "--json"]);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["profiles"],
        serde_json::json!([])
    );
    for forbidden in [
        "agents",
        "memory",
        "memories.db",
        "python",
        "channels",
        "config.toml",
    ] {
        assert!(!home.path().join(forbidden).exists(), "created {forbidden}");
    }
}

#[test]
fn list_use_and_rename_operate_without_exposing_remote_identity() {
    let home = tempfile::tempdir().unwrap();
    let profile_id = configured_profile(home.path());

    let listed = console(home.path(), &["list", "--local", "--json"]);
    assert!(listed.status.success());
    let rendered = String::from_utf8(listed.stdout).unwrap();
    assert!(rendered.contains("Production"));
    assert!(!rendered.contains("private.example"));
    assert!(!rendered.contains("Private Device Name"));

    let renamed = console(home.path(), &["rename", &profile_id[..8], "Office"]);
    assert!(renamed.status.success());
    let selected = console(home.path(), &["use", "Office"]);
    assert!(selected.status.success());

    let listed = console(home.path(), &["list", "--local", "--json"]);
    let value = serde_json::from_slice::<serde_json::Value>(&listed.stdout).unwrap();
    assert_eq!(value["profiles"][0]["label"], "Office");
    assert_eq!(value["profiles"][0]["active"], true);
}

#[test]
fn pair_help_exposes_the_standalone_network_contract() {
    let home = tempfile::tempdir().unwrap();
    let output = console(home.path(), &["pair", "--help"]);
    assert!(output.status.success());
    let rendered = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "--hub",
        "--profile",
        "--ca-bundle",
        "--proxy",
        "--proxy-password-secret",
        "--no-proxy",
    ] {
        assert!(rendered.contains(expected), "missing {expected}");
    }
}

#[test]
fn root_help_exposes_the_lightweight_terminal_surface() {
    let home = tempfile::tempdir().unwrap();
    let output = console(home.path(), &["--help"]);
    assert!(output.status.success());
    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(rendered.contains("tui"));
    assert!(rendered.contains("proxy-secret"));

    let output = console(home.path(), &["tui", "--help"]);
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("--profile"));

    let output = console(home.path(), &["proxy-secret", "--help"]);
    assert!(output.status.success());
    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(rendered.contains("set"));
    assert!(rendered.contains("delete"));
}

#[test]
fn invalid_pairing_network_creates_no_console_profile() {
    let home = tempfile::tempdir().unwrap();
    let output = console(
        home.path(),
        &["pair", "--hub", "http://public.example", "--no-browser"],
    );
    assert!(!output.status.success());
    assert!(!home.path().join("console").exists());
}

#[test]
fn proxy_secret_delete_requires_confirmation_without_creating_console_state() {
    let home = tempfile::tempdir().unwrap();
    let output = console(home.path(), &["proxy-secret", "delete", "office-proxy"]);
    assert!(!output.status.success());
    assert!(!home.path().join("console").exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("office-proxy-password"));
}

#[test]
fn conflicting_pairing_authority_does_not_mutate_the_existing_profile() {
    let home = tempfile::tempdir().unwrap();
    let profile_id = configured_profile(home.path());
    let output = console(
        home.path(),
        &[
            "pair",
            "--hub",
            "https://other.example",
            "--profile",
            &profile_id[..8],
            "--label",
            "Mutated",
            "--no-browser",
        ],
    );
    assert!(!output.status.success());

    let listed = console(home.path(), &["list", "--local", "--json"]);
    assert!(listed.status.success());
    let value = serde_json::from_slice::<serde_json::Value>(&listed.stdout).unwrap();
    assert_eq!(value["profiles"][0]["label"], "Production");
    assert_eq!(value["profiles"][0]["active"], true);

    let registry = ClientProfileRegistry::open(home.path().join("console")).unwrap();
    let config = ClientLocalConfigStore::open(registry.profile_root(&profile_id).unwrap())
        .unwrap()
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(config.network.hub_url, "https://private.example");
}
