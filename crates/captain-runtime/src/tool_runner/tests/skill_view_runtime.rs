use std::path::Path;

use captain_skills::registry::SkillRegistry;

use crate::tools::{check_skill, search_skills, view_skill};

fn write_markdown_skill(root: &Path) {
    let skill_dir = root.join("generated");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("debug-helper.md"),
        r#"
---
name: debug-helper
description: Systematic debugging workflow for failing tests
family: software-development
tags:
  - generated
  - quarantine
---
Reproduce the failure, isolate the minimal case, patch the root cause, then rerun the focused test.
"#,
    )
    .unwrap();
}

fn write_directory_skill(root: &Path) {
    let skill_dir = root.join("api-helper");
    std::fs::create_dir_all(skill_dir.join("references")).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        r#"
prompt_context = "Use the API helper workflow."

[skill]
name = "api-helper"
version = "0.1.0"
description = "API helper workflow"

[runtime]
type = "promptonly"
"#,
    )
    .unwrap();
    std::fs::write(skill_dir.join("references/api.md"), "GET /v1/items").unwrap();
}

fn write_validation_skill(root: &Path) {
    let skill_dir = root.join("validation-helper");
    std::fs::create_dir_all(skill_dir.join("references")).unwrap();
    std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        r#"
prompt_context = "Read references/api.md, run scripts/check.sh, then compare templates/missing.md."

[skill]
name = "validation-helper"
version = "0.1.0"
description = "Validation helper workflow"

[runtime]
type = "shell"
entry = "scripts/run.sh"
"#,
    )
    .unwrap();
    std::fs::write(skill_dir.join("references/api.md"), "GET /v1/items").unwrap();
    std::fs::write(skill_dir.join("scripts/check.sh"), "echo check").unwrap();
}

fn write_shell_block_skill(root: &Path, script: &str) {
    let skill_dir = root.join("shell-helper");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("shell-helper.md"),
        format!(
            r#"
---
name: shell-helper
description: Shell helper workflow
---

### run
```bash
{script}
```
"#
        ),
    )
    .unwrap();
}

fn warning_codes(response: &serde_json::Value) -> Vec<String> {
    response["validation"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|warning| warning["code"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn view_skill_returns_actionable_context() {
    let dir = tempfile::tempdir().unwrap();
    write_markdown_skill(dir.path());
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = view_skill(
        &serde_json::json!({"name": "debug-helper"}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(response["status"], "enabled");
    assert_eq!(response["family"]["id"], "software-development");
    assert_eq!(response["governance"]["generated"], true);
    assert_eq!(response["governance"]["quarantined"], true);
    assert_eq!(response["governance"]["locked"], true);
    assert_eq!(
        response["governance"]["promotion_required"],
        "schema_diff_tests_human"
    );
    assert_eq!(response["file_backed"], true);
    assert!(response.get("path").is_none());
    assert!(response["validation"]["skill_file"].get("path").is_none());
    assert!(
        !raw.contains(dir.path().to_string_lossy().as_ref()),
        "skill_view must not expose local skill roots"
    );
    assert_eq!(response["validation"]["status"], "ok");
    assert!(response["prompt_context"]
        .as_str()
        .is_some_and(|ctx| ctx.contains("Reproduce the failure")));
}

#[test]
fn search_skill_exposes_generated_quarantine_governance() {
    let dir = tempfile::tempdir().unwrap();
    write_markdown_skill(dir.path());
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = search_skills(
        &serde_json::json!({"query": "debug helper", "include_families": false}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let item = &response["results"][0];

    assert_eq!(item["name"], "debug-helper");
    assert_eq!(item["governance"]["generated"], true);
    assert_eq!(item["governance"]["quarantined"], true);
    assert_eq!(item["governance"]["locked"], true);
}

#[test]
fn view_skill_guides_missing_registry_and_missing_name() {
    let no_registry = view_skill(&serde_json::json!({"name": "debug-helper"}), None).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&no_registry).unwrap()["status"],
        "unavailable"
    );

    let dir = tempfile::tempdir().unwrap();
    let registry = SkillRegistry::new(dir.path().to_path_buf());
    let not_found = view_skill(&serde_json::json!({"name": "missing"}), Some(&registry)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&not_found).unwrap()["status"],
        "not_found"
    );
}

#[test]
fn view_skill_lists_and_loads_linked_files() {
    let dir = tempfile::tempdir().unwrap();
    write_directory_skill(dir.path());
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = view_skill(&serde_json::json!({"name": "api-helper"}), Some(&registry)).unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(response["status"], "enabled");
    assert_eq!(response["file_backed"], true);
    assert!(response.get("path").is_none());
    assert!(
        !raw.contains(dir.path().to_string_lossy().as_ref()),
        "skill_view linked-file index must not expose local skill roots"
    );
    assert_eq!(
        response["linked_files"]["references"][0],
        "references/api.md"
    );
    assert_eq!(response["validation"]["status"], "ok");
    assert_eq!(response["validation"]["linked_files_checked"], 1);

    let raw_file = view_skill(
        &serde_json::json!({"name": "api-helper", "file_path": "references/api.md"}),
        Some(&registry),
    )
    .unwrap();
    let file_response: serde_json::Value = serde_json::from_str(&raw_file).unwrap();
    assert_eq!(file_response["status"], "ok");
    assert_eq!(file_response["content"], "GET /v1/items");
}

#[test]
fn view_skill_can_omit_prompt_context() {
    let dir = tempfile::tempdir().unwrap();
    write_directory_skill(dir.path());
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = view_skill(
        &serde_json::json!({"name": "api-helper", "include_context": false}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(response["status"], "enabled");
    assert!(response.get("content").is_none());
    assert!(response.get("prompt_context").is_none());
    assert_eq!(response["validation"]["status"], "ok");
}

#[test]
fn view_skill_blocks_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    write_directory_skill(dir.path());
    std::fs::write(dir.path().join("secret.env"), "TOKEN=do-not-read").unwrap();
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = view_skill(
        &serde_json::json!({"name": "api-helper", "file_path": "../secret.env"}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(response["status"], "blocked");
    assert!(!raw.contains("do-not-read"));
}

#[test]
fn view_skill_reports_validation_warnings() {
    let dir = tempfile::tempdir().unwrap();
    write_validation_skill(dir.path());
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = view_skill(
        &serde_json::json!({"name": "validation-helper"}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let warnings = warning_codes(&response);

    assert_eq!(response["status"], "enabled");
    assert_eq!(response["validation"]["status"], "warn");
    assert_eq!(response["validation"]["runtime_entry"]["status"], "missing");
    assert_eq!(
        response["validation"]["missing_referenced_files"][0],
        "templates/missing.md"
    );
    assert!(warnings.contains(&"missing_runtime_entry".to_string()));
    assert!(warnings.contains(&"missing_referenced_file".to_string()));
    assert!(response["validation"]["suggested_checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check
            .as_str()
            .is_some_and(|text| text.contains("Inspect linked scripts"))));
    assert_eq!(response["validation"]["preflight_recommended"], true);
    assert_eq!(
        response["validation"]["preflight_tool_call"]["name"],
        "skill_check"
    );
}

#[test]
fn check_skill_passes_valid_shell_blocks() {
    let dir = tempfile::tempdir().unwrap();
    write_shell_block_skill(dir.path(), "echo ok");
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = check_skill(
        &serde_json::json!({"name": "shell-helper"}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(response["status"], "pass");
    assert_eq!(response["summary"]["failures"], 0);
    assert!(
        !raw.contains(dir.path().to_string_lossy().as_ref()),
        "skill_check must not expose local skill roots through validation"
    );
    assert_eq!(response["static_tests"][0]["status"], "pass");
    assert!(response["next_action"]
        .as_str()
        .is_some_and(|text| text.contains("preflight passed")));

    let raw_view = view_skill(
        &serde_json::json!({"name": "shell-helper"}),
        Some(&registry),
    )
    .unwrap();
    let view_response: serde_json::Value = serde_json::from_str(&raw_view).unwrap();
    assert_eq!(view_response["validation"]["preflight_recommended"], true);
    assert_eq!(
        view_response["validation"]["preflight_tool_call"]["input"]["name"],
        "shell-helper"
    );
}

#[test]
fn check_skill_fails_invalid_shell_blocks() {
    let dir = tempfile::tempdir().unwrap();
    write_shell_block_skill(dir.path(), "if then");
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();

    let raw = check_skill(
        &serde_json::json!({"name": "shell-helper"}),
        Some(&registry),
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(response["status"], "fail");
    assert_eq!(response["summary"]["failures"], 1);
    assert_eq!(response["static_tests"][0]["status"], "fail");
    assert!(response["next_action"]
        .as_str()
        .is_some_and(|text| text.contains("Do not execute")));
}
