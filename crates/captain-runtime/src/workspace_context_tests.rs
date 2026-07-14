use super::*;

#[test]
fn test_detect_rust_project() {
    let dir = std::env::temp_dir().join("captain_ws_rust_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    assert_eq!(detect_project_type(&dir), ProjectType::Rust);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_detect_node_project() {
    let dir = std::env::temp_dir().join("captain_ws_node_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), "{}").unwrap();
    assert_eq!(detect_project_type(&dir), ProjectType::Node);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_detect_python_project() {
    let dir = std::env::temp_dir().join("captain_ws_py_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pyproject.toml"), "[tool.poetry]").unwrap();
    assert_eq!(detect_project_type(&dir), ProjectType::Python);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_detect_go_project() {
    let dir = std::env::temp_dir().join("captain_ws_go_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("go.mod"), "module example.com/test").unwrap();
    assert_eq!(detect_project_type(&dir), ProjectType::Go);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_detect_unknown_project() {
    let dir = std::env::temp_dir().join("captain_ws_unk_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(detect_project_type(&dir), ProjectType::Unknown);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_workspace_context_detect() {
    let dir = std::env::temp_dir().join("captain_ws_ctx_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("AGENTS.md"), "# Agent Guidelines\nBe helpful.").unwrap();

    let ctx = WorkspaceContext::detect(&dir);
    assert_eq!(ctx.project_type, ProjectType::Rust);
    assert!(ctx.is_git_repo);
    assert!(ctx.cache.contains_key("AGENTS.md"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_workspace_context_detects_codex_claude_style_guidance() {
    let dir = std::env::temp_dir().join("captain_ws_guidance_ctx_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("CLAUDE.md"), "# Claude Code\nRun pytest.").unwrap();
    std::fs::write(dir.join("CAPTAIN.md"), "# Captain\nUse project tasks.").unwrap();
    std::fs::write(dir.join("CODEX.md"), "# Codex\nKeep diffs small.").unwrap();

    let mut ctx = WorkspaceContext::detect(&dir);
    let section = ctx.build_context_section();
    assert!(ctx.cache.contains_key("CLAUDE.md"));
    assert!(ctx.cache.contains_key("CAPTAIN.md"));
    assert!(ctx.cache.contains_key("CODEX.md"));
    assert!(section.contains("Run pytest"));
    assert!(section.contains("Use project tasks"));
    assert!(section.contains("Keep diffs small"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_workspace_context_skips_generated_agent_markdown() {
    let dir = std::env::temp_dir().join("captain_ws_generated_md_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("AGENTS.md"),
        "# Agent Behavioral Guidelines\n\n## Memory Journal\nUpdate MEMORY.md after significant actions.\n",
    )
    .unwrap();
    std::fs::write(dir.join("TOOLS.md"), "# Tools & Environment\n").unwrap();
    std::fs::write(dir.join("SOUL.md"), "Custom soul").unwrap();

    let ctx = WorkspaceContext::detect(&dir);
    assert!(!ctx.cache.contains_key("AGENTS.md"));
    assert!(!ctx.cache.contains_key("TOOLS.md"));
    assert!(ctx.cache.contains_key("SOUL.md"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_get_file_cache_hit() {
    let dir = std::env::temp_dir().join("captain_ws_cache_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SOUL.md"), "I am a helpful agent.").unwrap();

    let mut ctx = WorkspaceContext::detect(&dir);
    let content1 = ctx.get_file("SOUL.md").map(|s| s.to_string());
    let content2 = ctx.get_file("SOUL.md").map(|s| s.to_string());
    assert_eq!(content1, content2);
    assert!(content1.unwrap().contains("helpful agent"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_file_size_cap() {
    let dir = std::env::temp_dir().join("captain_ws_cap_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Write a file larger than 32KB.
    let big = "x".repeat(40_000);
    std::fs::write(dir.join("AGENTS.md"), &big).unwrap();

    let ctx = WorkspaceContext::detect(&dir);
    assert!(!ctx.cache.contains_key("AGENTS.md"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_build_context_section() {
    let dir = std::env::temp_dir().join("captain_ws_section_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("SOUL.md"), "Be nice").unwrap();

    let mut ctx = WorkspaceContext::detect(&dir);
    let section = ctx.build_context_section();
    assert!(section.contains("Rust"));
    assert!(section.contains("Git repository: yes"));
    assert!(section.contains("SOUL.md"));
    assert!(section.contains("Be nice"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_workspace_state_round_trip() {
    let dir = std::env::temp_dir().join("captain_ws_state_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let state = WorkspaceState {
        version: 1,
        bootstrap_seeded_at: Some("2026-01-01T00:00:00Z".to_string()),
        onboarding_completed_at: None,
    };
    state.save(&dir).unwrap();

    let loaded = WorkspaceState::load(&dir);
    assert_eq!(loaded.version, 1);
    assert_eq!(
        loaded.bootstrap_seeded_at.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert!(loaded.onboarding_completed_at.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_workspace_state_missing_file() {
    let dir = std::env::temp_dir().join("captain_ws_state_missing");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let state = WorkspaceState::load(&dir);
    assert_eq!(state.version, 0);
    assert!(state.bootstrap_seeded_at.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
