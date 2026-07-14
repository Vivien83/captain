use std::path::Path;

use crate::reflection_job::ReflectionCompleter;
use crate::session_summarizer::*;

fn turn(role: &str, text: &str) -> PersistedTurn {
    PersistedTurn {
        role: role.into(),
        text: text.into(),
    }
}

#[test]
fn frontmatter_has_all_required_fields() {
    let fm = checkpoint_frontmatter("sid-1", "daemon-x", 100, 5, 99);
    assert!(fm.starts_with("---\n"));
    assert!(fm.ends_with("---\n"));
    assert!(fm.contains("session_id: sid-1"));
    assert!(fm.contains("agent_key: daemon-x"));
    assert!(fm.contains("updated_at: 100"));
    assert!(fm.contains("message_count: 5"));
    assert!(fm.contains("summarized_at_mtime: 99"));
}

#[test]
fn system_prompt_lists_the_five_sections() {
    let p = summarizer_system_prompt();
    for section in [
        "# Sujets",
        "# Décisions",
        "# Erreurs / Échecs",
        "# Réussites",
        "# Infos durables",
    ] {
        assert!(p.contains(section), "missing section: {section}");
    }
}

#[test]
fn user_prompt_includes_transcript_and_truncates() {
    let session = LoadedSession {
        agent_name: "captain".into(),
        messages: vec![
            turn("user", "hello"),
            turn("agent", "hi back"),
            turn("user", "X".repeat(500).as_str()),
        ],
        ..Default::default()
    };
    let p = build_summarizer_user_prompt(&session, 100);
    assert!(p.contains("user: hello"));
    assert!(p.contains("agent: hi back"));
    assert!(p.contains("[transcript truncated]"));
}

#[test]
fn should_summarize_when_no_checkpoint_yet() {
    let s = LoadedSession {
        messages: vec![turn("u", "a"), turn("a", "b"), turn("u", "c")],
        ..Default::default()
    };
    assert!(should_summarize(&s, 100, None));
}

#[test]
fn should_skip_when_session_too_short() {
    let s = LoadedSession {
        messages: vec![turn("u", "a")],
        ..Default::default()
    };
    assert!(!should_summarize(&s, 100, None));
}

#[test]
fn should_skip_when_checkpoint_uptodate() {
    let s = LoadedSession {
        messages: vec![turn("u", "a"), turn("a", "b"), turn("u", "c")],
        ..Default::default()
    };
    assert!(!should_summarize(&s, 100, Some(100)));
    assert!(!should_summarize(&s, 100, Some(200)));
}

#[test]
fn should_resummarize_when_json_newer_than_checkpoint() {
    let s = LoadedSession {
        messages: vec![turn("u", "a"), turn("a", "b"), turn("u", "c")],
        ..Default::default()
    };
    assert!(should_summarize(&s, 200, Some(100)));
}

#[test]
fn checkpoint_path_sits_next_to_json_with_md_extension() {
    let p = Path::new("/tmp/sessions/agent-x/1234.json");
    let cp = checkpoint_path_for(p);
    assert_eq!(cp, Path::new("/tmp/sessions/agent-x/1234.checkpoint.md"));
}

#[test]
fn read_checkpoint_mtime_extracts_the_field() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.md");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "---").unwrap();
    writeln!(f, "session_id: x").unwrap();
    writeln!(f, "summarized_at_mtime: 4242").unwrap();
    writeln!(f, "---").unwrap();
    writeln!(f, "# Sujets").unwrap();
    let m = read_checkpoint_mtime(&path);
    assert_eq!(m, Some(4242));
}

#[test]
fn read_checkpoint_mtime_returns_none_when_missing() {
    let m = read_checkpoint_mtime(Path::new("/nonexistent/x.md"));
    assert_eq!(m, None);
}

struct StubCompleter {
    body: String,
}

#[async_trait::async_trait]
impl ReflectionCompleter for StubCompleter {
    async fn complete(&self, _model: &str, _system: &str, _user: &str) -> Result<String, String> {
        Ok(self.body.clone())
    }
}

#[test]
fn matches_query_handles_multi_word_and_case() {
    assert!(matches_query(
        "Projet Alpha utilise PostgreSQL 16",
        "alpha postgresql"
    ));
    assert!(matches_query("Anything", ""));
    assert!(!matches_query("Projet Alpha", "redis"));
    assert!(!matches_query("foo bar", "foo qux"));
}

#[test]
fn recall_returns_freshest_first_and_caps_results() {
    let root = tempfile::tempdir().unwrap();
    let agent_dir = root.path().join("daemon-x");
    std::fs::create_dir_all(&agent_dir).unwrap();
    for (id, ts, body_extra) in [
        ("oldest", 100u64, "prod-server va bien"),
        ("middle", 200u64, "deployment script échoue"),
        ("newest", 300u64, "prod-server et deployment"),
    ] {
        let body = format!(
            "---\nsession_id: {id}\nagent_key: daemon-x\nupdated_at: {ts}\nmessage_count: 5\nsummarized_at_mtime: {ts}\n---\n# Sujets\n- {body_extra}\n"
        );
        std::fs::write(agent_dir.join(format!("{id}.checkpoint.md")), body).unwrap();
    }
    let hits = recall_checkpoints(root.path(), "prod-server", 10, None);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].session_id, "newest");
    assert_eq!(hits[1].session_id, "oldest");

    let capped = recall_checkpoints(root.path(), "prod-server", 1, None);
    assert_eq!(capped.len(), 1);
    assert_eq!(capped[0].session_id, "newest");
}

#[test]
fn recall_respects_agent_filter() {
    let root = tempfile::tempdir().unwrap();
    for agent in ["daemon-a", "daemon-b"] {
        let dir = root.path().join(agent);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("s.checkpoint.md"),
            format!(
                "---\nsession_id: {agent}-s\nagent_key: {agent}\nupdated_at: 1\nmessage_count: 1\nsummarized_at_mtime: 1\n---\n# Sujets\n- ssh\n"
            ),
        )
        .unwrap();
    }
    let only_a = recall_checkpoints(root.path(), "ssh", 10, Some("daemon-a"));
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].agent_key, "daemon-a");
}

#[test]
fn recall_backfills_and_uses_fts_index() {
    let root = tempfile::tempdir().unwrap();
    let agent_dir = root.path().join("daemon-x");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("s1.checkpoint.md"),
        "---\nsession_id: s1\nagent_key: daemon-x\nupdated_at: 42\nmessage_count: 5\nsummarized_at_mtime: 42\n---\n# Decisions\n- Captain should index session recall with sqlite fts\n",
    )
    .unwrap();

    assert!(!session_index_path(root.path()).exists());
    let hits = recall_checkpoints(root.path(), "sqlite fts", 10, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].session_id, "s1");
    assert!(session_index_path(root.path()).exists());
}

#[test]
fn recall_returns_empty_on_missing_root() {
    let hits = recall_checkpoints(Path::new("/nonexistent/root"), "anything", 5, None);
    assert!(hits.is_empty());
}

#[tokio::test]
async fn scan_handles_missing_root_gracefully() {
    let stub = StubCompleter {
        body: "irrelevant".into(),
    };
    scan_and_summarize_once(
        &stub,
        std::path::Path::new("/nonexistent/sessions"),
        "claude-haiku-test",
    )
    .await;
}

#[test]
fn assembled_checkpoint_starts_with_frontmatter_then_body() {
    let s = LoadedSession {
        messages: vec![turn("u", "a"), turn("a", "b")],
        updated_at: 999,
        ..Default::default()
    };
    let out = assemble_checkpoint(
        "sid-42",
        "daemon-x",
        &s,
        500,
        "# Sujets\n- foo\n# Décisions\n- (rien)\n# Erreurs / Échecs\n- (rien)\n# Réussites\n- (rien)\n# Infos durables\n- (rien)",
    );
    assert!(out.starts_with("---\nsession_id: sid-42"));
    assert!(out.contains("# Sujets\n- foo"));
    assert!(out.ends_with('\n'));
}

#[test]
fn staged_learning_review_contains_all_phases_and_dedup_instruction() {
    let review = build_learning_stage_review("main", "captain", "# Infos durables\n- Telegram");
    for phase in [
        "OBSERVE", "THINK", "PLAN", "BUILD", "EXECUTE", "VERIFY", "LEARN",
    ] {
        assert!(review.contains(phase), "missing phase: {phase}");
    }
    assert!(review.contains("reject duplicates"));
    assert!(review.contains("# Infos durables"));
}
