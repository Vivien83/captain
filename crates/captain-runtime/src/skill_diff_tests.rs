use super::*;
use tempfile::TempDir;

fn proposal(name: &str) -> SkillProposal {
    SkillProposal {
        name: name.into(),
        description: "Searches documentation then writes a concise markdown report".into(),
        trigger_hint: "when user asks to research developer documentation".into(),
        tool_sequence: vec!["web_search".into(), "web_fetch".into(), "file_write".into()],
        arg_schema_hint: "topic: string".into(),
        confidence: 0.91,
        family: Some("software-development".into()),
        pattern_hash: "h".into(),
        origin_channel: None,
    }
}

#[test]
fn exact_name_is_duplicate() {
    let existing = ExistingSkill {
        name: "doc-research".into(),
        source: "test".into(),
        description: String::new(),
        trigger_hint: String::new(),
        tool_names: vec![],
        body: String::new(),
    };
    let matched = compare(&proposal("doc-research"), &existing).unwrap();
    assert_eq!(matched.score, 100);
    assert_eq!(matched.reason, "same_skill_name");
}

#[test]
fn similar_tools_and_body_are_duplicate() {
    let existing = ExistingSkill {
        name: "developer-doc-report".into(),
        source: "test".into(),
        description: "Research developer documentation and create a markdown report".into(),
        trigger_hint: "when user asks to research documentation".into(),
        tool_names: vec!["web_search".into(), "web_fetch".into(), "file_write".into()],
        body: "Use web_search, web_fetch, compare sources, and write markdown.".into(),
    };
    let matched = compare(&proposal("doc-research"), &existing).unwrap();
    assert!(matched.score >= DEFAULT_DUPLICATE_SCORE, "{matched:?}");
}

#[test]
fn unrelated_skill_scores_low() {
    let existing = ExistingSkill {
        name: "docker-cleanup".into(),
        source: "test".into(),
        description: "Prune Docker images and containers safely".into(),
        trigger_hint: "when disk is full".into(),
        tool_names: vec!["shell_exec".into()],
        body: "Inspect docker system df before pruning images.".into(),
    };
    let score = compare(&proposal("doc-research"), &existing)
        .map(|matched| matched.score)
        .unwrap_or(0);
    assert!(score < DEFAULT_DUPLICATE_SCORE, "{score}");
}

#[test]
fn scans_skill_toml_and_prompt_context() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("doc-report");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        r#"
[skill]
name = "doc-report"
description = "Research developer documentation and create a markdown report"

[[tools.provided]]
name = "doc_report"
description = "Research docs and write report"
input_schema = { type = "object" }
"#,
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("prompt_context.md"),
        "Use web_search, web_fetch, source comparison, and markdown output.",
    )
    .unwrap();

    let cfg = SkillDiffConfig {
        roots: vec![dir.path().to_path_buf()],
        include_bundled: false,
        duplicate_score: DEFAULT_DUPLICATE_SCORE,
    };
    let matched = find_duplicate(&proposal("doc-research"), &cfg).unwrap();
    assert_eq!(matched.existing_name, "doc-report");
}

#[test]
fn supporting_markdown_files_are_not_indexed_as_skills() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("api-helper");
    std::fs::create_dir_all(skill_dir.join("references")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: api-helper
description: API helper workflow
---
Use web_search and web_fetch to inspect API docs.
"#,
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("references/api.md"),
        r#"---
name: fake-reference-skill
description: Reference documentation, not a runnable skill
---
This file must stay linked context only.
"#,
    )
    .unwrap();

    let cfg = SkillDiffConfig {
        roots: vec![dir.path().to_path_buf()],
        include_bundled: false,
        duplicate_score: DEFAULT_DUPLICATE_SCORE,
    };
    let skills = load_existing_skills(&cfg);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "api-helper");
}
