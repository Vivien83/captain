use captain_capspec::{compile_named, Effect, Idempotency};
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/capspec-certification")
}

fn compile_fixture(name: &str) -> captain_capspec::CompiledCapability {
    let path = fixtures_root().join(format!("{name}.captain"));
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    compile_named(
        &source,
        captain_capspec::parse(&source).unwrap(),
        Some(name),
    )
    .unwrap_or_else(|error| panic!("compile {}: {error}", path.display()))
}

#[test]
fn every_real_certification_fixture_compiles_under_the_public_contract() {
    for name in [
        "cert-cargo",
        "cert-crash",
        "cert-http-allowed",
        "cert-http-denied",
        "cert-memory",
        "cert-parallel",
        "cert-secret",
        "cert-transform",
        "cert-traversal",
        "cert-write",
    ] {
        let compiled = compile_fixture(name);
        assert_eq!(compiled.name, name);
        assert_eq!(
            compiled.tool_name,
            format!("cap_{}", name.replace('-', "_"))
        );
        assert!(!compiled.steps.is_empty());
    }
}

#[test]
fn fixtures_pin_parallel_dependency_and_operator_boundaries() {
    let parallel = compile_fixture("cert-parallel");
    assert_eq!(parallel.policy.max_parallel, 2);
    assert!(parallel.steps.iter().all(|step| step.needs.is_empty()));
    assert!(!parallel.requires_human_approval());

    let write = compile_fixture("cert-write");
    assert_eq!(write.steps[1].needs, ["write"]);
    assert!(write.requires_human_approval());

    let crash = compile_fixture("cert-crash");
    assert_eq!(crash.steps[0].effect, Effect::Destructive);
    assert_eq!(crash.steps[0].idempotency, Idempotency::Manual);
    assert!(crash.requires_human_approval());

    let denied = compile_fixture("cert-http-denied");
    assert_eq!(denied.permissions.network_hosts, ["example.com"]);
    assert!(denied.requires_human_approval());

    let traversal = compile_fixture("cert-traversal");
    assert!(!traversal.requires_human_approval());
}
