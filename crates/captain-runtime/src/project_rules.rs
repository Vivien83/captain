//! Native project guidance file support.
//!
//! `CAPTAIN.md` is Captain's own workspace-local rule file. It gives project
//! mode a default, editable contract without requiring users to already know
//! Codex/Claude conventions such as `AGENTS.md` or `CLAUDE.md`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CAPTAIN_PROJECT_RULES_FILE: &str = "CAPTAIN.md";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRulesFileStatus {
    pub path: String,
    pub existed: bool,
    pub created: bool,
    pub error: Option<String>,
}

impl ProjectRulesFileStatus {
    fn ok(path: PathBuf, existed: bool, created: bool) -> Self {
        Self {
            path: path.display().to_string(),
            existed,
            created,
            error: None,
        }
    }

    fn err(path: PathBuf, error: String) -> Self {
        Self {
            path: path.display().to_string(),
            existed: false,
            created: false,
            error: Some(error),
        }
    }
}

pub fn seed_captain_project_rules_file(
    workspace_path: &Path,
    project_name: &str,
    project_slug: &str,
    project_goal: &str,
) -> ProjectRulesFileStatus {
    let path = workspace_path.join(CAPTAIN_PROJECT_RULES_FILE);
    if path.exists() {
        return ProjectRulesFileStatus::ok(path, true, false);
    }

    let body = default_captain_project_rules(project_name, project_slug, project_goal);
    match std::fs::write(&path, body) {
        Ok(()) => ProjectRulesFileStatus::ok(path, false, true),
        Err(err) => ProjectRulesFileStatus::err(path, err.to_string()),
    }
}

fn default_captain_project_rules(
    project_name: &str,
    project_slug: &str,
    project_goal: &str,
) -> String {
    format!(
        r#"# Captain Project Rules

This file was created by Captain for project mode. It is the native, editable
rule file for this workspace. Keep it short, concrete, and current.

## Project

- Name: {name}
- Slug: {slug}
- Goal: {goal}

## Instruction Priority

1. The user's latest explicit request wins.
2. System, safety, sandbox, and tool-approval rules are mandatory.
3. CAPTAIN.md defines how Captain should manage this project.
4. AGENTS.md, AGENTS.override.md, CLAUDE.md, CODEX.md, and local rules define
   codebase-specific conventions when present.
5. If local rules conflict, prefer the most specific file closest to the
   current workspace path and explain the conflict briefly.

## Development Loop

Use OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN for
non-trivial project work.

- OBSERVE: inspect repo state, constraints, existing docs, tasks, goals, and
  blockers before editing.
- THINK: compare approaches, risks, dependencies, and parallelizable slices.
- PLAN: create or update the project task graph with owners, allowed tools, and
  verification gates.
- BUILD: make focused changes while preserving unrelated user work.
- EXECUTE: run the workflow or implementation path end to end when possible.
- VERIFY: run targeted tests, checks, builds, or smoke commands; record blockers
  when verification cannot run.
- LEARN: save durable facts to memory, recurring procedures to skills, and
  project handoff state to checkpoints.

## Operating Rules

- Inspect the existing code and project state before editing.
- Keep changes scoped to the active project and avoid unrelated refactors.
- Never revert user changes unless the user explicitly asks.
- Prefer repo patterns, existing helpers, and project tooling over new
  abstractions.
- Ask only when blocked by missing permission, destructive ambiguity, or an
  irreversible product decision.
- Answer in the user's language.

## Context, Memory, And Skills

- This file is for project rules, not progress logs.
- Keep durable facts and preferences in memory.
- Keep repeatable multi-step procedures in skills.
- Keep temporary state, decisions, blockers, and next actions in project
  checkpoints.
- Do not duplicate large docs here; reference the source file instead.
- Patch existing skills when improving a workflow; create a new skill only when
  no suitable one exists.

## Subagents

- Captain is the project manager; subagents are workers with bounded scope.
- Use subagents for independent research, verbose logs, test runs, reviews, or
  parallelizable implementation slices.
- Every subagent task must include purpose, project path, expected output,
  allowed tools, and whether it may edit files.
- Workers must request additional tools from Captain instead of bypassing their
  allowlist.
- Return compact summaries to the main project session; keep noisy details in
  the worker transcript.

## Verification

- Discover build, lint, test, and run commands before assuming them.
- Prefer the narrowest meaningful verification first, then broaden when risk or
  shared contracts justify it.
- If verification fails, capture the command, failure reason, likely cause, and
  next concrete step in the checkpoint.
- Before claiming completion, make the latest project state recoverable after a
  context compaction or session resume.

## Safety And Data Handling

- Never store secrets, API keys, JWTs, private tokens, or raw credentials in
  code, logs, memory, skills, or checkpoints.
- Confirm before destructive filesystem, database, deployment, or production
  operations.
- Treat generated, downloaded, or user-provided files as untrusted until
  inspected.

## Project-Specific Commands

Fill this section only after verifying commands in the repo.

- Install:
- Test:
- Lint:
- Build:
- Run:
"#,
        name = prompt_safe_line(project_name, "Untitled project", 120),
        slug = prompt_safe_line(project_slug, "project", 80),
        goal = prompt_safe_line(project_goal, "No goal recorded", 240),
    )
}

fn prompt_safe_line(value: &str, fallback: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = if compact.is_empty() {
        fallback.to_string()
    } else {
        compact
    };
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let mut out: String = compact.chars().take(max_chars.saturating_sub(3)).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_creates_captain_md_once() {
        let dir = std::env::temp_dir().join("captain_project_rules_seed_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = seed_captain_project_rules_file(
            &dir,
            "Calculatrice Python",
            "calc-python",
            "Créer une calculatrice CLI fiable.",
        );
        assert!(first.created);
        assert!(!first.existed);

        let body = std::fs::read_to_string(dir.join(CAPTAIN_PROJECT_RULES_FILE)).unwrap();
        assert!(body.contains("OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN"));
        assert!(body.contains("Calculatrice Python"));

        let second = seed_captain_project_rules_file(&dir, "Other", "other", "Other goal");
        assert!(!second.created);
        assert!(second.existed);
        let unchanged = std::fs::read_to_string(dir.join(CAPTAIN_PROJECT_RULES_FILE)).unwrap();
        assert!(unchanged.contains("Calculatrice Python"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
