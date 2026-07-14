//! Static preflight checks for installed skills.

use std::path::{Path, PathBuf};
use std::process::Command;

use captain_skills::registry::SkillRegistry;
use captain_skills::{InstalledSkill, SkillRuntime};

use super::skill_linked_files::linked_files;
use super::skill_view_validation::skill_validation;

const SKILL_CHECK_NO_REGISTRY_HINT: &str =
    "SkillRegistry is unavailable in this execution context. Retry skill_check from the main Captain runtime.";

const SKILL_CHECK_NOT_FOUND_HINT: &str =
    "No installed skill has this exact name. Call skill_search before checking a skill.";

struct SkillCheckRequest {
    name: String,
    run_static_tests: bool,
    max_shell_blocks: usize,
}

pub fn check_skill(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let request = parse_skill_check_request(input)?;

    let Some(registry) = skill_registry else {
        return Ok(skill_check_status_response(
            &request.name,
            "unavailable",
            SKILL_CHECK_NO_REGISTRY_HINT,
        ));
    };
    let Some(skill) = registry.get(&request.name) else {
        return Ok(skill_check_status_response(
            &request.name,
            "not_found",
            SKILL_CHECK_NOT_FOUND_HINT,
        ));
    };

    Ok(checked_skill_response(&request, skill).to_string())
}

fn parse_skill_check_request(input: &serde_json::Value) -> Result<SkillCheckRequest, String> {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return Err("missing required field: name".to_string());
    }

    Ok(SkillCheckRequest {
        name: name.to_string(),
        run_static_tests: input
            .get("run_static_tests")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        max_shell_blocks: input
            .get("max_shell_blocks")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 50) as usize,
    })
}

fn skill_check_status_response(name: &str, status: &str, hint: &str) -> String {
    serde_json::json!({
        "name": name,
        "status": status,
        "hint": hint,
    })
    .to_string()
}

fn checked_skill_response(
    request: &SkillCheckRequest,
    skill: &InstalledSkill,
) -> serde_json::Value {
    let root = skill_root(skill.path.as_path());
    let linked = root.as_deref().map(linked_files).unwrap_or_default();
    let validation = skill_validation(skill, root.as_deref(), &linked);
    let mut checks = Vec::new();
    let mut failures = 0usize;
    let mut warnings = 0usize;

    add_enabled_check(skill, &mut checks, &mut warnings);
    add_validation_check(&validation, &mut checks, &mut failures, &mut warnings);
    let static_tests = add_static_shell_checks(
        skill,
        root.as_deref(),
        request,
        &mut checks,
        &mut failures,
        &mut warnings,
    );
    let status = skill_check_status(failures, warnings);

    serde_json::json!({
        "name": request.name.as_str(),
        "status": status,
        "summary": {
            "failures": failures,
            "warnings": warnings,
            "static_tests": static_tests.len(),
        },
        "checks": checks,
        "validation": validation,
        "static_tests": static_tests,
        "next_action": skill_check_next_action(status),
    })
}

fn add_enabled_check(
    skill: &InstalledSkill,
    checks: &mut Vec<serde_json::Value>,
    warnings: &mut usize,
) {
    push_check(
        checks,
        "enabled",
        if skill.enabled { "pass" } else { "warn" },
        if skill.enabled {
            "Skill is enabled."
        } else {
            "Skill is present but disabled."
        },
    );
    if !skill.enabled {
        *warnings += 1;
    }
}

fn add_validation_check(
    validation: &serde_json::Value,
    checks: &mut Vec<serde_json::Value>,
    failures: &mut usize,
    warnings: &mut usize,
) {
    let validation_blocking = has_blocking_validation_warning(validation);
    let validation_status = if validation_blocking {
        *failures += 1;
        "fail"
    } else if validation["status"] != "ok" {
        *warnings += 1;
        "warn"
    } else {
        "pass"
    };
    push_check(
        checks,
        "file_backed_validation",
        validation_status,
        "Skill file, runtime entry, linked files, and support-file references were checked.",
    );
}

fn add_static_shell_checks(
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
    request: &SkillCheckRequest,
    checks: &mut Vec<serde_json::Value>,
    failures: &mut usize,
    warnings: &mut usize,
) -> Vec<serde_json::Value> {
    if request.run_static_tests {
        let result = run_static_skill_tests(skill, skill_root, request.max_shell_blocks);
        *failures += result.failures;
        *warnings += result.warnings;
        push_check(
            checks,
            "static_shell_syntax",
            result.check_status,
            result.check_message,
        );
        return result.tests;
    }
    *warnings += 1;
    push_check(
        checks,
        "static_shell_syntax",
        "skipped",
        "Static tests disabled by input.",
    );
    Vec::new()
}

fn skill_check_status(failures: usize, warnings: usize) -> &'static str {
    if failures > 0 {
        "fail"
    } else if warnings > 0 {
        "warn"
    } else {
        "pass"
    }
}

fn skill_check_next_action(status: &str) -> &'static str {
    match status {
        "pass" => "Skill preflight passed. Follow skill_view guidance before executing.",
        "warn" => "Inspect warnings before relying on this skill; use skill_view file_path for linked scripts or references.",
        _ => "Do not execute this skill yet. Fix failed checks or choose/refine another skill.",
    }
}

struct StaticTestResult {
    tests: Vec<serde_json::Value>,
    failures: usize,
    warnings: usize,
    check_status: &'static str,
    check_message: &'static str,
}

fn run_static_skill_tests(
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
    max_shell_blocks: usize,
) -> StaticTestResult {
    let mut tests = Vec::new();
    let mut failures = 0usize;
    let mut warnings = 0usize;

    if let Some(entry_test) = shell_runtime_entry_test(skill, skill_root) {
        if entry_test["status"] == "fail" {
            failures += 1;
        } else if entry_test["status"] == "skipped" {
            warnings += 1;
        }
        tests.push(entry_test);
    }

    let blocks = extract_shell_blocks(
        skill.manifest.prompt_context.as_deref().unwrap_or_default(),
        max_shell_blocks,
    );
    for block in blocks {
        let test = bash_syntax_test(&block.label, &block.script);
        if test["status"] == "fail" {
            failures += 1;
        } else if test["status"] == "skipped" {
            warnings += 1;
        }
        tests.push(test);
    }

    if tests.is_empty() {
        warnings += 1;
        return StaticTestResult {
            tests,
            failures,
            warnings,
            check_status: "skipped",
            check_message: "No shell runtime entry or bash/sh code blocks were available for static syntax checks.",
        };
    }

    StaticTestResult {
        tests,
        failures,
        warnings,
        check_status: if failures > 0 {
            "fail"
        } else if warnings > 0 {
            "warn"
        } else {
            "pass"
        },
        check_message: "Shell syntax was checked with bash -n without executing commands.",
    }
}

fn shell_runtime_entry_test(
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
) -> Option<serde_json::Value> {
    if !matches!(&skill.manifest.runtime.runtime_type, SkillRuntime::Shell) {
        return None;
    }
    let entry = skill.manifest.runtime.entry.trim();
    if entry.is_empty() {
        return Some(serde_json::json!({
            "kind": "runtime_entry",
            "label": "runtime.entry",
            "status": "fail",
            "message": "Shell runtime skill has no entry file.",
        }));
    }
    let Some(root) = skill_root else {
        return Some(serde_json::json!({
            "kind": "runtime_entry",
            "label": entry,
            "status": "skipped",
            "message": "No file-backed skill directory is available.",
        }));
    };
    let path = root.join(entry);
    let Ok(script) = std::fs::read_to_string(&path) else {
        return Some(serde_json::json!({
            "kind": "runtime_entry",
            "label": entry,
            "status": "fail",
            "message": "Runtime entry could not be read.",
        }));
    };
    Some(bash_syntax_test(entry, &script))
}

struct ShellBlock {
    label: String,
    script: String,
}

fn extract_shell_blocks(content: &str, limit: usize) -> Vec<ShellBlock> {
    let mut blocks = Vec::new();
    let mut heading = "unnamed".to_string();
    let mut in_shell_block = false;
    let mut current = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if !in_shell_block {
            if let Some(rest) = trimmed.strip_prefix("### ") {
                heading = rest.trim().to_string();
                continue;
            }
            if is_shell_fence(trimmed) {
                in_shell_block = true;
                current.clear();
            }
            continue;
        }

        if trimmed == "```" {
            blocks.push(ShellBlock {
                label: heading.clone(),
                script: current.join("\n"),
            });
            if blocks.len() >= limit {
                return blocks;
            }
            in_shell_block = false;
            continue;
        }
        current.push(line.to_string());
    }
    blocks
}

fn is_shell_fence(line: &str) -> bool {
    matches!(
        line,
        "```bash" | "```sh" | "```shell" | "``` bash" | "``` sh" | "``` shell"
    )
}

fn bash_syntax_test(label: &str, script: &str) -> serde_json::Value {
    if script.trim().is_empty() {
        return serde_json::json!({
            "kind": "shell_syntax",
            "label": label,
            "status": "skipped",
            "message": "Empty shell block.",
        });
    }
    if script.len() > 65_536 {
        return serde_json::json!({
            "kind": "shell_syntax",
            "label": label,
            "status": "skipped",
            "message": "Shell block is too large for inline syntax checking.",
        });
    }

    let output = Command::new("bash")
        .arg("-n")
        .arg("-c")
        .arg(script)
        .output();
    let Ok(output) = output else {
        return serde_json::json!({
            "kind": "shell_syntax",
            "label": label,
            "status": "skipped",
            "message": "bash is unavailable on this host.",
        });
    };
    if output.status.success() {
        return serde_json::json!({
            "kind": "shell_syntax",
            "label": label,
            "status": "pass",
            "message": "bash -n passed.",
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    serde_json::json!({
        "kind": "shell_syntax",
        "label": label,
        "status": "fail",
        "message": if stderr.is_empty() { "bash -n failed.".to_string() } else { stderr },
    })
}

fn skill_root(path: &Path) -> Option<PathBuf> {
    if path == Path::new("<bundled>") {
        return None;
    }
    if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(Path::to_path_buf)
    }
}

fn has_blocking_validation_warning(validation: &serde_json::Value) -> bool {
    validation["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|warning| warning["code"].as_str())
        .any(|code| {
            matches!(
                code,
                "missing_skill_file"
                    | "missing_runtime_entry"
                    | "blocked_runtime_entry"
                    | "runtime_entry_not_file"
                    | "missing_referenced_file"
                    | "blocked_referenced_file"
                    | "unreadable_runtime_entry"
                    | "unreadable_referenced_file"
            )
        })
}

fn push_check(checks: &mut Vec<serde_json::Value>, id: &str, status: &str, message: &str) {
    checks.push(serde_json::json!({
        "id": id,
        "status": status,
        "message": message,
    }));
}
