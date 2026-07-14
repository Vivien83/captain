use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct MemStore(BTreeMap<String, Zeroizing<String>>);

impl SshSecretStore for MemStore {
    fn get(&self, key: &str) -> Option<Zeroizing<String>> {
        self.0.get(key).cloned()
    }

    fn set(&mut self, key: String, value: Zeroizing<String>) -> Result<(), String> {
        self.0.insert(key, value);
        Ok(())
    }

    fn remove(&mut self, key: &str) -> Result<bool, String> {
        Ok(self.0.remove(key).is_some())
    }

    fn list_keys(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

fn sample_key(name: &str) -> SshKey {
    SshKey {
        name: name.to_string(),
        host: "server.example.com".into(),
        port: 22,
        user: "captain".into(),
        private_key: Zeroizing::new(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END OPENSSH PRIVATE KEY-----\n".into(),
        ),
        passphrase: None,
        fingerprint: "SHA256:abc123".into(),
        added_at: 1_700_000_000,
        last_used: None,
    }
}

#[test]
fn save_and_load_roundtrip() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("prod-server")).unwrap();
    let back = load_ssh_key(&store, "prod-server").expect("must round-trip");
    assert_eq!(back.host, "server.example.com");
    assert_eq!(back.port, 22);
    assert_eq!(back.user, "captain");
    assert_eq!(back.fingerprint, "SHA256:abc123");
}

#[test]
fn resolve_ssh_key_accepts_unambiguous_shorthand() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("staging-server")).unwrap();
    let resolved = resolve_ssh_key(&store, "staging").unwrap();
    assert_eq!(resolved.resolved, "staging-server");
    assert_eq!(resolved.resolution, SshAliasResolution::UniqueAlias);
}

#[test]
fn resolve_ssh_key_accepts_natural_language_alias_phrase() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("prod-vps")).unwrap();
    save_ssh_key(&mut store, sample_key("staging-server")).unwrap();

    let resolved = resolve_ssh_key(&store, "vps perso").unwrap();

    assert_eq!(resolved.resolved, "prod-vps");
    assert_eq!(resolved.resolution, SshAliasResolution::UniqueAlias);
}

#[test]
fn resolve_ssh_key_fails_closed_on_ambiguous_alias_phrase() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("prod-vps")).unwrap();
    save_ssh_key(&mut store, sample_key("vps-staging")).unwrap();

    let err = resolve_ssh_key(&store, "mon vps").unwrap_err();

    assert!(err.contains("ambiguous"), "got: {err}");
    assert!(err.contains("prod-vps"));
    assert!(err.contains("vps-staging"));
}

#[test]
fn resolve_ssh_key_uses_default_for_generic_request() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("staging-server")).unwrap();
    save_ssh_key(&mut store, sample_key("prod-server")).unwrap();
    set_default_ssh_key(&mut store, "prod-server").unwrap();
    let resolved = resolve_ssh_key(&store, "server").unwrap();
    assert_eq!(resolved.resolved, "prod-server");
    assert_eq!(resolved.resolution, SshAliasResolution::Default);
}

#[test]
fn resolve_ssh_key_rejects_ambiguous_shorthand() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("api-prod")).unwrap();
    save_ssh_key(&mut store, sample_key("web-prod")).unwrap();
    let err = resolve_ssh_key(&store, "prod").unwrap_err();
    assert!(err.contains("ambiguous"));
    assert!(err.contains("api-prod"));
    assert!(err.contains("web-prod"));
}

#[test]
fn list_strips_prefix_and_skips_internal() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("a")).unwrap();
    save_ssh_key(&mut store, sample_key("b")).unwrap();
    store
        .set(SSH_DEFAULT_KEY.to_string(), Zeroizing::new("a".into()))
        .unwrap();
    let names = list_ssh_keys(&store);
    assert_eq!(names.len(), 2, "internal `_default` must not be listed");
    assert!(names.contains(&"a".to_string()));
    assert!(names.contains(&"b".to_string()));
}

#[test]
fn delete_returns_true_when_present_false_when_absent() {
    let mut store = MemStore::default();
    save_ssh_key(&mut store, sample_key("x")).unwrap();
    assert!(delete_ssh_key(&mut store, "x").unwrap());
    assert!(!delete_ssh_key(&mut store, "x").unwrap());
    assert!(load_ssh_key(&store, "x").is_none());
}

#[test]
fn set_default_requires_existing_key() {
    let mut store = MemStore::default();
    assert!(set_default_ssh_key(&mut store, "ghost").is_err());
    save_ssh_key(&mut store, sample_key("real")).unwrap();
    assert!(set_default_ssh_key(&mut store, "real").is_ok());
    assert_eq!(get_default_ssh_key(&store).as_deref(), Some("real"));
}

#[test]
fn fingerprint_of_unencrypted_ed25519_matches_ssh_keygen_format() {
    let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\n\
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n\
        QyNTUxOQAAACC+h2XHFRvMhz24O6tMKm+B4QWriqoCGRDOYMa9suc91wAAAJjaN0w+2jdM\n\
        PgAAAAtzc2gtZWQyNTUxOQAAACC+h2XHFRvMhz24O6tMKm+B4QWriqoCGRDOYMa9suc91w\n\
        AAAEC6CAU3QqHvG1dbSzfbmLdSAVxzjYbVbfM+hPRn8M3p5b6HZccVG8yHPbg7q0wqb4Hh\n\
        BauKqgIZEM5gxr2y5z3XAAAAEXRlc3QtcTYtdGhyb3dhd2F5AQIDBA==\n\
        -----END OPENSSH PRIVATE KEY-----\n";
    let fp = fingerprint_of(pem, None).expect("valid key");
    assert_eq!(
        fp, "SHA256:cn5IEOhe/2DG5+14DcUbPM6kcab6TKj0pknTjrhyf5E",
        "fingerprint must match `ssh-keygen -lf` output"
    );
}

#[test]
fn fingerprint_of_invalid_pem_returns_clear_error() {
    let r = fingerprint_of("not a key", None);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("parse"));
}

#[test]
fn audit_log_appends_jsonl_lines_and_is_resilient_to_repeats() {
    let dir = tempfile::tempdir().unwrap();
    audit_log(dir.path(), "test", "prod-server", "tcp ok", true);
    audit_log(dir.path(), "exec", "prod-server", "ls -la", true);
    let content = std::fs::read_to_string(dir.path().join("ssh.log")).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["op"], "test");
    assert_eq!(first["key"], "prod-server");
    assert_eq!(first["ok"], true);
}
