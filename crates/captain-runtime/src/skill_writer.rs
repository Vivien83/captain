//! SkillWriter (v3.13d.2).
//!
//! Turns an approved `SkillProposal` into a `.md` file on disk under
//! the configured generated-skills root. Trusts that the caller has already
//! passed the draft through `ProposalPolicy::evaluate` — this module
//! only enforces the filesystem-level invariants the policy can't
//! handle: path canonicalisation and collision suffixing.
//!
//! The generated file has YAML front-matter (source, pattern hash,
//! confidence, agent, tool sequence) followed by a Markdown body with
//! description + trigger hint + arg schema hint. Same shape as the
//! hand-written skills under `crates/captain-hands/bundled/`.

use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{info, warn};

use crate::skill_proposer::SkillProposal;

#[derive(Debug, Clone, Copy)]
pub struct SkillWriteContext<'a> {
    pub approved_by: Option<&'a str>,
    pub verified_by: &'a str,
    pub success_rate: Option<f32>,
}

impl Default for SkillWriteContext<'_> {
    fn default() -> Self {
        Self {
            approved_by: None,
            verified_by: "human",
            success_rate: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum WriteError {
    #[error("invalid skill name: {0}")]
    InvalidName(String),
    #[error("path escape attempt: {0}")]
    PathEscape(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Write the proposal as Markdown inside `<root>/<name>.md`.
/// Collisions (same name on disk) are resolved by appending a
/// millisecond timestamp suffix. Returns the final path written.
pub fn write(proposal: &SkillProposal, root: &Path) -> Result<PathBuf, WriteError> {
    write_with_context(proposal, root, SkillWriteContext::default())
}

pub fn write_with_context(
    proposal: &SkillProposal,
    root: &Path,
    context: SkillWriteContext<'_>,
) -> Result<PathBuf, WriteError> {
    sanitize_name(&proposal.name)?;
    let target_dir = root.to_path_buf();
    fs::create_dir_all(&target_dir)?;
    let canonical_root = target_dir
        .canonicalize()
        .unwrap_or_else(|_| target_dir.clone());

    let mut filename = format!("{}.md", proposal.name);
    let mut path = canonical_root.join(&filename);
    if path.exists() {
        let stamp = Utc::now().timestamp_millis();
        filename = format!("{}-{stamp}.md", proposal.name);
        path = canonical_root.join(&filename);
    }

    // Belt-and-braces: after resolving the name, verify the final path
    // still lives under `canonical_root`. A race condition between
    // `create_dir_all` and `exists()` could in theory plant a symlink;
    // this check catches it before write.
    if let Some(parent) = path.parent() {
        let parent_canon = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if !parent_canon.starts_with(&canonical_root) {
            return Err(WriteError::PathEscape(path.display().to_string()));
        }
    }

    let contents = render_markdown(proposal, context);
    let mut f = fs::File::create(&path)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    info!(
        path = %path.display(),
        name = %proposal.name,
        confidence = proposal.confidence,
        "skill_writer: wrote generated skill"
    );
    Ok(path)
}

/// Verify the proposal name is the slug we expect. The policy has
/// already checked this regex, but the writer repeats the check as
/// a defence against a future caller that forgets policy.
pub fn sanitize_name(name: &str) -> Result<(), WriteError> {
    if name.is_empty() || name.len() > 48 {
        return Err(WriteError::InvalidName(name.to_string()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(WriteError::InvalidName(name.to_string()));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        // Slug aesthetics: no leading/trailing dashes, no double-dashes.
        warn!(name, "skill name has ugly dash pattern but passes");
    }
    Ok(())
}

fn render_markdown(p: &SkillProposal, context: SkillWriteContext<'_>) -> String {
    let timestamp = Utc::now().to_rfc3339();
    let tools = if p.tool_sequence.is_empty() {
        "  - _No observed tool sequence; prompt-only workflow candidate._".to_string()
    } else {
        p.tool_sequence
            .iter()
            .map(|t| format!("  - {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let family = p.family.as_deref().unwrap_or("general-automation");
    let description_yaml = yaml_block_scalar(&p.description);
    let approved_by = yaml_string(context.approved_by.unwrap_or("unknown"));
    let verified_by = yaml_string(context.verified_by);
    let success_rate = context
        .success_rate
        .map(|rate| rate.clamp(0.0, 1.0).to_string())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "---\n\
         id: {name}\n\
         name: {name}\n\
         description: {description_yaml}\n\
         owner: agent\n\
         locked: true\n\
         approved: true\n\
         version: 1\n\
         verified_by: {verified_by}\n\
         success_rate: {success_rate}\n\
         approved_by: {approved_by}\n\
         quarantine: true\n\
         promotion_status: \"quarantined\"\n\
         promotion_required: \"schema_diff_tests_human\"\n\
         validated_at: {ts}\n\
         generated_at: {ts}\n\
         source: v3.13 SkillSynthesizer\n\
         family: {family}\n\
         pattern_hash: {hash}\n\
         confidence: {conf}\n\
         tags:\n\
           - generated\n\
           - quarantine\n\
           - family:{family}\n\
         tool_sequence:\n{tools}\n\
         ---\n\n\
         # {name}\n\n\
         {description}\n\n\
         ## Trigger\n\n\
         {trigger}\n\n\
         ## Arguments\n\n\
         {args}\n\n\
         ## Tool sequence\n\n\
         The SkillSynthesizer observed this sequence recur before proposing\n\
         this skill. Review the ordering and edit as needed:\n\n\
         {tools}\n",
        name = p.name,
        ts = timestamp,
        family = family,
        approved_by = approved_by,
        verified_by = verified_by,
        success_rate = success_rate,
        hash = p.pattern_hash,
        conf = p.confidence,
        tools = tools,
        description = p.description,
        trigger = if p.trigger_hint.is_empty() {
            "_No trigger hint provided._"
        } else {
            p.trigger_hint.as_str()
        },
        args = if p.arg_schema_hint.is_empty() {
            "_No argument hint provided._"
        } else {
            p.arg_schema_hint.as_str()
        },
    )
}

fn yaml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

fn yaml_block_scalar(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "\"\"".to_string();
    }
    let mut out = String::from("|-\n");
    for line in trimmed.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out.pop();
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample(name: &str) -> SkillProposal {
        SkillProposal {
            name: name.into(),
            description: "Search the web then write a summary file".into(),
            trigger_hint: "user asks to research a topic".into(),
            tool_sequence: vec!["web_search".into(), "web_fetch".into(), "file_write".into()],
            arg_schema_hint: "query: string, path: string".into(),
            confidence: 0.9,
            family: Some("general-automation".into()),
            pattern_hash: "h123".into(),
            origin_channel: None,
        }
    }

    #[test]
    fn sanitize_name_rejects_uppercase() {
        assert!(sanitize_name("ResearchLog").is_err());
    }

    #[test]
    fn sanitize_name_rejects_slash() {
        assert!(sanitize_name("foo/bar").is_err());
    }

    #[test]
    fn sanitize_name_rejects_dotdot() {
        assert!(sanitize_name("../evil").is_err());
    }

    #[test]
    fn sanitize_name_accepts_simple_slug() {
        assert!(sanitize_name("research-log").is_ok());
    }

    #[test]
    fn sanitize_name_rejects_empty_and_too_long() {
        assert!(sanitize_name("").is_err());
        let too_long = "a".repeat(49);
        assert!(sanitize_name(&too_long).is_err());
    }

    #[test]
    fn write_creates_file_with_frontmatter_and_body() {
        let dir = TempDir::new().unwrap();
        let path = write(&sample("research-log"), dir.path()).unwrap();
        assert!(path.exists());
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("---\n"));
        assert!(body.contains("id: research-log"));
        assert!(body.contains("name: research-log"));
        assert!(body.contains("owner: agent"));
        assert!(body.contains("locked: true"));
        assert!(body.contains("approved: true"));
        assert!(body.contains("verified_by: \"human\""));
        assert!(body.contains("quarantine: true"));
        assert!(body.contains("promotion_status: \"quarantined\""));
        assert!(body.contains("promotion_required: \"schema_diff_tests_human\""));
        assert!(body.contains("success_rate: null"));
        assert!(body.contains("family: general-automation"));
        assert!(body.contains("- quarantine"));
        assert!(body.contains("- family:general-automation"));
        assert!(body.contains("pattern_hash: h123"));
        assert!(body.contains("# research-log"));
        assert!(body.contains("Search the web then write a summary file"));
        assert!(body.contains("- web_search"));
    }

    #[test]
    fn write_with_context_records_approval_provenance() {
        let dir = TempDir::new().unwrap();
        let path = write_with_context(
            &sample("verified-skill"),
            dir.path(),
            SkillWriteContext {
                approved_by: Some("channel"),
                verified_by: "tests",
                success_rate: Some(0.97),
            },
        )
        .unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("approved_by: \"channel\""));
        assert!(body.contains("verified_by: \"tests\""));
        assert!(body.contains("success_rate: 0.97"));
    }

    #[test]
    fn write_lands_under_configured_generated_root() {
        let dir = TempDir::new().unwrap();
        let path = write(&sample("skill-a"), dir.path()).unwrap();
        let parent = path.parent().unwrap();
        assert_eq!(parent, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn write_suffixes_on_collision() {
        let dir = TempDir::new().unwrap();
        let first = write(&sample("dup"), dir.path()).unwrap();
        let second = write(&sample("dup"), dir.path()).unwrap();
        assert_ne!(first, second);
        assert!(first.exists());
        assert!(second.exists());
        // Second name contains a numeric suffix.
        let second_name = second.file_name().unwrap().to_string_lossy().to_string();
        assert!(second_name.starts_with("dup-"));
    }

    #[test]
    fn write_rejects_invalid_name() {
        let dir = TempDir::new().unwrap();
        let mut p = sample("research-log");
        p.name = "Evil/Path".into();
        assert!(matches!(
            write(&p, dir.path()).unwrap_err(),
            WriteError::InvalidName(_)
        ));
    }

    #[test]
    fn write_renders_empty_trigger_as_placeholder() {
        let dir = TempDir::new().unwrap();
        let mut p = sample("no-trigger");
        p.trigger_hint = String::new();
        let path = write(&p, dir.path()).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("_No trigger hint provided._"));
    }

    #[test]
    fn write_renders_prompt_only_workflow_without_tools() {
        let dir = TempDir::new().unwrap();
        let mut p = sample("prompt-only");
        p.tool_sequence.clear();
        let path = write(&p, dir.path()).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("_No observed tool sequence"));
    }

    #[test]
    fn write_creates_missing_root() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("deep");
        let path = write(&sample("deep"), &nested).unwrap();
        assert!(path.exists());
        // Canonicalize both sides to tolerate macOS `/private` symlink resolution.
        let got = path.canonicalize().unwrap();
        let expected_root = nested.canonicalize().unwrap();
        assert!(
            got.starts_with(&expected_root),
            "expected {got:?} to start with {expected_root:?}"
        );
    }

    #[test]
    fn write_yaml_frontmatter_terminates_properly() {
        let dir = TempDir::new().unwrap();
        let path = write(&sample("yaml-test"), dir.path()).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        // Front-matter has exactly two `---` delimiter lines.
        let count = body.matches("\n---\n").count() + if body.starts_with("---\n") { 1 } else { 0 };
        assert!(count >= 2, "expected two front-matter delimiters");
    }
}
