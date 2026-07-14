//! File-backed validation helpers for exact skill views.

use std::collections::BTreeSet;
use std::path::Path;

use captain_skills::{InstalledSkill, SkillRuntime};

use super::skill_linked_files::path_has_traversal;

const SUPPORT_DIRS: [&str; 4] = ["references", "templates", "scripts", "assets"];

pub(crate) fn skill_validation(
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
    linked_files: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let mut warnings = Vec::new();
    let linked_file_count = linked_file_count(linked_files);
    let prompt_context = skill.manifest.prompt_context.as_deref().unwrap_or_default();
    let referenced_files = referenced_support_files(prompt_context);
    let mut missing_referenced_files = Vec::new();
    let mut blocked_referenced_files = Vec::new();
    let is_executable = is_executable_skill(skill);
    let has_shell_blocks = has_shell_blocks(prompt_context);

    let skill_file = skill_file_status(skill, skill_root, &mut warnings);
    let runtime_entry = runtime_entry_status(skill, skill_root, &mut warnings);

    validate_referenced_file_set(
        skill_root,
        &referenced_files,
        &mut missing_referenced_files,
        &mut blocked_referenced_files,
        &mut warnings,
    );
    let (suggested_checks, preflight_recommended) = suggested_validation_checks(
        skill,
        linked_files,
        has_shell_blocks,
        is_executable,
        !warnings.is_empty(),
    );
    let status = validation_status(&warnings, skill_root);

    serde_json::json!({
        "status": status,
        "skill_file": skill_file,
        "runtime_entry": runtime_entry,
        "linked_files_checked": linked_file_count,
        "referenced_files_checked": referenced_files.len(),
        "missing_referenced_files": missing_referenced_files,
        "blocked_referenced_files": blocked_referenced_files,
        "required_env_vars": skill.manifest.requirements.env_inject.values().cloned().collect::<Vec<_>>(),
        "preflight_recommended": preflight_recommended,
        "preflight_tool_call": if preflight_recommended {
            serde_json::json!({"name": "skill_check", "input": {"name": skill.manifest.skill.name}})
        } else {
            serde_json::Value::Null
        },
        "warnings": warnings,
        "suggested_checks": suggested_checks,
    })
}

fn linked_file_count(linked_files: &serde_json::Map<String, serde_json::Value>) -> usize {
    linked_files
        .values()
        .filter_map(|value| value.as_array())
        .map(Vec::len)
        .sum()
}

fn is_executable_skill(skill: &InstalledSkill) -> bool {
    !matches!(
        &skill.manifest.runtime.runtime_type,
        SkillRuntime::PromptOnly | SkillRuntime::Builtin
    )
}

fn has_shell_blocks(prompt_context: &str) -> bool {
    ["```bash", "```sh", "```shell"]
        .iter()
        .any(|fence| prompt_context.contains(fence))
}

fn validate_referenced_file_set(
    skill_root: Option<&Path>,
    referenced_files: &[String],
    missing_referenced_files: &mut Vec<String>,
    blocked_referenced_files: &mut Vec<String>,
    warnings: &mut Vec<serde_json::Value>,
) {
    if referenced_files.is_empty() {
        return;
    }
    let Some(root) = skill_root else {
        push_warning(
            warnings,
            "unvalidated_referenced_files",
            "The skill references support files, but no file-backed directory is available.",
            None,
        );
        return;
    };
    validate_referenced_support_files(
        root,
        referenced_files,
        missing_referenced_files,
        blocked_referenced_files,
        warnings,
    );
}

fn suggested_validation_checks(
    skill: &InstalledSkill,
    linked_files: &serde_json::Map<String, serde_json::Value>,
    has_shell_blocks: bool,
    is_executable: bool,
    has_warnings: bool,
) -> (Vec<String>, bool) {
    let mut suggested_checks = Vec::new();
    let has_scripts = linked_files
        .get("scripts")
        .and_then(|value| value.as_array())
        .is_some_and(|items| !items.is_empty());

    push_base_suggested_checks(skill, has_scripts, is_executable, &mut suggested_checks);
    let preflight_recommended = has_scripts || has_shell_blocks || is_executable || has_warnings;
    if preflight_recommended {
        suggested_checks.push(format!(
            "Run skill_check with name '{}' before skill_execute or before relying on linked scripts.",
            skill.manifest.skill.name
        ));
    }
    if suggested_checks.is_empty() {
        suggested_checks.push(
            "No extra file-backed checks detected; follow the loaded SKILL.md workflow."
                .to_string(),
        );
    }
    (suggested_checks, preflight_recommended)
}

fn push_base_suggested_checks(
    skill: &InstalledSkill,
    has_scripts: bool,
    is_executable: bool,
    suggested_checks: &mut Vec<String>,
) {
    if has_scripts {
        suggested_checks.push(
            "Inspect linked scripts with skill_view file_path before executing a script-backed workflow."
                .to_string(),
        );
    }
    if !skill.manifest.requirements.tools.is_empty() {
        suggested_checks.push(
            "Verify required_tools with capability_search or tool_search before applying the workflow."
                .to_string(),
        );
    }
    if !skill.manifest.requirements.env_inject.is_empty() {
        suggested_checks.push(
            "Verify required environment secrets are configured before running executable skill code."
                .to_string(),
        );
    }
    if is_executable {
        suggested_checks.push(
            "Validate the runtime entry and command behavior before relying on this executable skill."
                .to_string(),
        );
    }
}

fn validation_status(warnings: &[serde_json::Value], skill_root: Option<&Path>) -> &'static str {
    if warnings.is_empty() {
        "ok"
    } else if skill_root.is_none() {
        "limited"
    } else {
        "warn"
    }
}

fn skill_file_status(
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
    warnings: &mut Vec<serde_json::Value>,
) -> serde_json::Value {
    if skill.path == Path::new("<bundled>") {
        return serde_json::json!({
            "file_backed": false,
            "present": null,
        });
    }

    let present = skill.path.exists()
        || skill_root.is_some_and(|root| {
            root.join("skill.toml").exists()
                || root.join("SKILL.md").exists()
                || root.join("prompt_context.md").exists()
        });
    if !present {
        push_warning(
            warnings,
            "missing_skill_file",
            "The registry entry points to a skill file or directory that is not present on disk.",
            None,
        );
    }

    serde_json::json!({
        "file_backed": true,
        "present": present,
    })
}

fn runtime_entry_status(
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
    warnings: &mut Vec<serde_json::Value>,
) -> serde_json::Value {
    let entry = skill.manifest.runtime.entry.trim();
    let runtime_type = runtime_type_json(skill);

    if entry.is_empty() {
        return empty_runtime_entry_status(skill, &runtime_type, warnings);
    }

    let Some(root) = skill_root else {
        return warned_runtime_entry_status(
            warnings,
            "unvalidated_runtime_entry",
            "Runtime entry is declared, but no file-backed directory is available.",
            Some(entry.to_string()),
            &runtime_type,
            entry,
            "unvalidated",
        );
    };
    if path_has_traversal(entry) {
        return warned_runtime_entry_status(
            warnings,
            "blocked_runtime_entry",
            "Runtime entry escapes the skill directory.",
            Some(entry.to_string()),
            &runtime_type,
            entry,
            "blocked",
        );
    }

    let target = root.join(entry);
    if !target.exists() {
        return warned_runtime_entry_status(
            warnings,
            "missing_runtime_entry",
            "Declared runtime entry file is missing.",
            Some(entry.to_string()),
            &runtime_type,
            entry,
            "missing",
        );
    }

    existing_runtime_entry_status(root, &target, &runtime_type, entry, warnings)
}

fn runtime_type_json(skill: &InstalledSkill) -> serde_json::Value {
    serde_json::to_value(&skill.manifest.runtime.runtime_type)
        .unwrap_or_else(|_| serde_json::json!("unknown"))
}

fn empty_runtime_entry_status(
    skill: &InstalledSkill,
    runtime_type: &serde_json::Value,
    warnings: &mut Vec<serde_json::Value>,
) -> serde_json::Value {
    let required = is_executable_skill(skill);
    if required {
        push_warning(
            warnings,
            "missing_runtime_entry",
            "Executable skills should declare a runtime entry file.",
            None,
        );
    }
    runtime_entry_json(
        runtime_type,
        "",
        if required { "missing" } else { "not_required" },
    )
}

fn existing_runtime_entry_status(
    root: &Path,
    target: &Path,
    runtime_type: &serde_json::Value,
    entry: &str,
    warnings: &mut Vec<serde_json::Value>,
) -> serde_json::Value {
    let Ok(root) = root.canonicalize() else {
        return warned_runtime_entry_status(
            warnings,
            "unreadable_skill_root",
            "Could not canonicalize the skill directory during validation.",
            None,
            runtime_type,
            entry,
            "unvalidated",
        );
    };
    let Ok(resolved) = target.canonicalize() else {
        return warned_runtime_entry_status(
            warnings,
            "unreadable_runtime_entry",
            "Could not canonicalize the runtime entry during validation.",
            Some(entry.to_string()),
            runtime_type,
            entry,
            "unvalidated",
        );
    };
    if !resolved.starts_with(&root) {
        return warned_runtime_entry_status(
            warnings,
            "blocked_runtime_entry",
            "Runtime entry resolves outside the skill directory.",
            Some(entry.to_string()),
            runtime_type,
            entry,
            "blocked",
        );
    }
    if !resolved.is_file() {
        return warned_runtime_entry_status(
            warnings,
            "runtime_entry_not_file",
            "Declared runtime entry is not a file.",
            Some(entry.to_string()),
            runtime_type,
            entry,
            "not_file",
        );
    }

    runtime_entry_json(runtime_type, entry, "ok")
}

fn warned_runtime_entry_status(
    warnings: &mut Vec<serde_json::Value>,
    code: &str,
    message: &str,
    file: Option<String>,
    runtime_type: &serde_json::Value,
    entry: &str,
    status: &str,
) -> serde_json::Value {
    push_warning(warnings, code, message, file);
    runtime_entry_json(runtime_type, entry, status)
}

fn runtime_entry_json(
    runtime_type: &serde_json::Value,
    entry: &str,
    status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": runtime_type,
        "entry": entry,
        "status": status,
    })
}

fn validate_referenced_support_files(
    root: &Path,
    referenced_files: &[String],
    missing: &mut Vec<String>,
    blocked: &mut Vec<String>,
    warnings: &mut Vec<serde_json::Value>,
) {
    let Some(root) = canonical_support_root(root, warnings) else {
        return;
    };

    for rel in referenced_files {
        validate_one_referenced_support_file(&root, rel, missing, blocked, warnings);
    }
}

fn canonical_support_root(
    root: &Path,
    warnings: &mut Vec<serde_json::Value>,
) -> Option<std::path::PathBuf> {
    match root.canonicalize() {
        Ok(root) => Some(root),
        Err(_) => {
            push_warning(
                warnings,
                "unreadable_skill_root",
                "Could not canonicalize the skill directory during support-file validation.",
                None,
            );
            None
        }
    }
}

fn validate_one_referenced_support_file(
    root: &Path,
    rel: &str,
    missing: &mut Vec<String>,
    blocked: &mut Vec<String>,
    warnings: &mut Vec<serde_json::Value>,
) {
    if path_has_traversal(rel) {
        push_blocked_referenced_file(
            blocked,
            warnings,
            rel,
            "Referenced support file escapes the skill directory.",
        );
        return;
    }
    let target = root.join(rel);
    if !target.exists() {
        missing.push(rel.to_string());
        push_warning(
            warnings,
            "missing_referenced_file",
            "Referenced support file is missing.",
            Some(rel.to_string()),
        );
        return;
    }
    validate_existing_referenced_support_file(root, rel, &target, blocked, warnings);
}

fn validate_existing_referenced_support_file(
    root: &Path,
    rel: &str,
    target: &Path,
    blocked: &mut Vec<String>,
    warnings: &mut Vec<serde_json::Value>,
) {
    let Ok(resolved) = target.canonicalize() else {
        blocked.push(rel.to_string());
        push_warning(
            warnings,
            "unreadable_referenced_file",
            "Referenced support file cannot be canonicalized.",
            Some(rel.to_string()),
        );
        return;
    };
    if !resolved.starts_with(root) {
        push_blocked_referenced_file(
            blocked,
            warnings,
            rel,
            "Referenced support file resolves outside the skill directory.",
        );
    } else if !resolved.is_file() {
        push_warning(
            warnings,
            "referenced_path_not_file",
            "Referenced support path is not a file.",
            Some(rel.to_string()),
        );
    }
}

fn push_blocked_referenced_file(
    blocked: &mut Vec<String>,
    warnings: &mut Vec<serde_json::Value>,
    rel: &str,
    message: &str,
) {
    blocked.push(rel.to_string());
    push_warning(
        warnings,
        "blocked_referenced_file",
        message,
        Some(rel.to_string()),
    );
}

fn referenced_support_files(context: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for raw in
        context.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '<' | '>'))
    {
        let token = raw.trim_matches(|c: char| {
            matches!(
                c,
                '`' | '"' | '\'' | ',' | '.' | ';' | ':' | '!' | '?' | '{' | '}'
            )
        });
        if token.len() > 240 {
            continue;
        }
        if SUPPORT_DIRS
            .iter()
            .any(|dir| token.starts_with(&format!("{dir}/")))
        {
            out.insert(token.to_string());
        }
    }
    out.into_iter().collect()
}

fn push_warning(
    warnings: &mut Vec<serde_json::Value>,
    code: &str,
    message: &str,
    file: Option<String>,
) {
    let mut warning = serde_json::Map::new();
    warning.insert("code".to_string(), serde_json::json!(code));
    warning.insert("message".to_string(), serde_json::json!(message));
    if let Some(file) = file {
        warning.insert("file".to_string(), serde_json::json!(file));
    }
    warnings.push(serde_json::Value::Object(warning));
}
