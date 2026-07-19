use super::*;

const READ_CAPABILITY: &str = r#"
format = 1
name = "project-summary"
description = "Read two project files and return their contents."
version = "1.2.0"
tags = ["Project", "read"]
output = "{{steps.read_cargo.output}}"

[inputs.root]
type = "string"
description = "Project root"

[permissions]
tools = ["file_read"]
read_paths = ["{{input.root}}/**"]

[policy]
timeout_secs = 60
max_parallel = 2

[[steps]]
id = "read_readme"
tool = "file_read"
needs = []
with = { path = "{{input.root}}/README.md" }

[[steps]]
id = "read_cargo"
tool = "file_read"
needs = []
with = { path = "{{input.root}}/Cargo.toml" }
"#;

#[test]
fn compiles_parallel_read_capability_into_typed_tool() {
    let compiled = compile(READ_CAPABILITY).unwrap();
    assert_eq!(compiled.tool_name, "cap_project_summary");
    assert_eq!(compiled.steps.len(), 2);
    assert!(compiled.steps.iter().all(|step| step.needs.is_empty()));
    assert!(compiled
        .steps
        .iter()
        .all(|step| step.effect == Effect::Read));
    assert!(!compiled.requires_human_approval());
    assert_eq!(compiled.tags, vec!["project", "read"]);
    assert_eq!(compiled.input_schema["required"], json!(["root"]));
}

#[test]
fn capability_name_rejects_underscore_alias_collisions() {
    let source = READ_CAPABILITY.replace("project-summary", "project_summary");
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("must use '-' instead of '_'"), "{error}");
}

#[test]
fn omitted_needs_defaults_to_previous_step() {
    let source = READ_CAPABILITY.replacen(
        "needs = []\nwith = { path = \"{{input.root}}/Cargo.toml\" }",
        "with = { path = \"{{steps.read_readme.output}}\" }",
        1,
    );
    let compiled = compile(&source).unwrap();
    assert_eq!(compiled.steps[1].needs, vec!["read_readme"]);
}

#[test]
fn rejects_dependency_cycle() {
    let source = READ_CAPABILITY
        .replace(
            "needs = []\nwith = { path = \"{{input.root}}/README.md\" }",
            "needs = [\"read_cargo\"]\nwith = { path = \"{{input.root}}/README.md\" }",
        )
        .replace(
            "needs = []\nwith = { path = \"{{input.root}}/Cargo.toml\" }",
            "needs = [\"read_readme\"]\nwith = { path = \"{{input.root}}/Cargo.toml\" }",
        );
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("contains a cycle"), "{error}");
}

#[test]
fn rejects_reference_to_non_dependency() {
    let source = READ_CAPABILITY.replace(
        "with = { path = \"{{input.root}}/Cargo.toml\" }",
        "with = { path = \"{{steps.read_readme.output}}\" }",
    );
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("without depending"), "{error}");
}

#[test]
fn rejects_tool_missing_from_permission_manifest() {
    let source = READ_CAPABILITY.replace("tool = \"file_read\"", "tool = \"shell_exec\"");
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("absent from permissions.tools"), "{error}");
}

#[test]
fn rejects_effect_downgrade() {
    let source = READ_CAPABILITY
        .replace(
            "tools = [\"file_read\"]",
            "tools = [\"shell_exec\"]\nshell_commands = [\"cargo test *\"]",
        )
        .replace("tool = \"file_read\"", "tool = \"shell_exec\"")
        .replace("needs = []", "needs = []\neffect = \"read\"");
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("below the Destructive minimum"), "{error}");
}

#[test]
fn mutating_capability_needs_approval_and_cannot_retry_manual_step() {
    let source = write_capability_source();
    let compiled = compile(&source).unwrap();
    assert!(compiled.requires_human_approval());
    assert!(compiled
        .steps
        .iter()
        .all(|step| step.idempotency == Idempotency::Manual));

    let retrying = source.replace(
        "tool = \"file_write\"",
        "tool = \"file_write\"\nretry = { max_attempts = 2, backoff_ms = 10 }",
    );
    let error = compile(&retrying).unwrap_err().to_string();
    assert!(
        error.contains("cannot retry with manual idempotency"),
        "{error}"
    );
}

#[test]
fn keyed_write_can_retry() {
    let source = READ_CAPABILITY
        .replace(
            "tools = [\"file_read\"]",
            "tools = [\"file_write\"]\nwrite_paths = [\"{{input.root}}/**\"]",
        )
        .replace("tool = \"file_read\"", "tool = \"file_write\"")
        .replace(
            "needs = []",
            "needs = []\nidempotency = \"keyed\"\nidempotency_key = \"{{run.id}}:write\"\nretry = { max_attempts = 2, backoff_ms = 10 }",
        );
    assert!(compile(&source).is_ok());
}

#[test]
fn web_download_requires_network_and_write_scopes() {
    let source = READ_CAPABILITY
        .replace(
            "tools = [\"file_read\"]",
            "tools = [\"web_download\"]\nnetwork_hosts = [\"example.com\"]",
        )
        .replace("tool = \"file_read\"", "tool = \"web_download\"")
        .replace(
            "with = { path = \"{{input.root}}/README.md\" }",
            "with = { url = \"https://example.com/a\", path = \"download/a\" }",
        )
        .replace(
            "with = { path = \"{{input.root}}/Cargo.toml\" }",
            "with = { url = \"https://example.com/b\", path = \"download/b\" }",
        );
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("permissions.write_paths"), "{error}");
    assert!(compile(&source.replace(
        "network_hosts = [\"example.com\"]",
        "network_hosts = [\"example.com\"]\nwrite_paths = [\"download/**\"]"
    ))
    .is_ok());
}

#[test]
fn input_validation_applies_defaults_and_rejects_unknown_fields() {
    let source = READ_CAPABILITY.replace(
        "[inputs.root]\ntype = \"string\"\ndescription = \"Project root\"",
        "[inputs.root]\ntype = \"string\"\ndescription = \"Project root\"\nrequired = false\ndefault = \".\"",
    );
    let compiled = compile(&source).unwrap();
    let normalized = compiled.validate_input(&json!({})).unwrap();
    assert_eq!(normalized["root"], json!("."));
    assert!(compiled.validate_input(&json!({"other": true})).is_err());
}

#[test]
fn file_stem_must_match_declared_name() {
    let raw = parse(READ_CAPABILITY).unwrap();
    let error = compile_named(READ_CAPABILITY, raw, Some("other"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("file stem 'other' must match"), "{error}");
}

#[test]
fn permission_fingerprint_is_order_independent() {
    let reordered = READ_CAPABILITY.replace(
        "tools = [\"file_read\"]\nread_paths = [\"{{input.root}}/**\"]",
        "tools = [\"file_read\", \"file_read\"]\nread_paths = [\"{{input.root}}/**\", \"{{input.root}}/**\"]",
    );
    assert_eq!(
        compile(READ_CAPABILITY).unwrap().permission_fingerprint,
        compile(&reordered).unwrap().permission_fingerprint
    );
}

#[test]
fn omitted_output_returns_the_actual_last_step() {
    let source = READ_CAPABILITY.replace("output = \"{{steps.read_cargo.output}}\"\n", "");
    let compiled = compile(&source).unwrap();
    assert_eq!(compiled.output, json!("{{steps.read_cargo.output}}"));
}

#[test]
fn permission_scope_rejects_unknown_input_reference() {
    let source = READ_CAPABILITY.replace(
        "read_paths = [\"{{input.root}}/**\"]",
        "read_paths = [\"{{input.missing}}/**\"]",
    );
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("unknown input 'missing'"), "{error}");
}

#[test]
fn strict_toml_rejects_unknown_top_level_fields() {
    let source = format!("unknown = true\n{READ_CAPABILITY}");
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("unknown field `unknown`"), "{error}");
}

#[test]
fn source_size_limit_is_fail_closed() {
    let source = "x".repeat(MAX_SOURCE_BYTES + 1);
    assert!(matches!(
        compile(&source),
        Err(CompileError::SourceTooLarge)
    ));
}

#[test]
fn capability_name_is_provider_safe() {
    let source =
        READ_CAPABILITY.replace("name = \"project-summary\"", "name = \"Project Summary\"");
    let error = compile(&source).unwrap_err().to_string();
    assert!(error.contains("must start with a lowercase"), "{error}");
}

fn write_capability_source() -> String {
    READ_CAPABILITY
        .replace(
            "tools = [\"file_read\"]",
            "tools = [\"file_write\"]\nwrite_paths = [\"{{input.root}}/**\"]",
        )
        .replace("tool = \"file_read\"", "tool = \"file_write\"")
        .replace(
            "with = { path = \"{{input.root}}/README.md\" }",
            "with = { path = \"{{input.root}}/out.txt\", content = \"ok\" }",
        )
        .replace(
            "with = { path = \"{{input.root}}/Cargo.toml\" }",
            "with = { path = \"{{input.root}}/out2.txt\", content = \"ok\" }",
        )
}
