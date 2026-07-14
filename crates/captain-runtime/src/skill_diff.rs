//! Deterministic diff against existing skills.
//!
//! This is the anti-duplication gate before a generated skill proposal lands in
//! review. It deliberately avoids embeddings or remote calls: the policy path
//! must stay cheap, deterministic, and available while offline.

use crate::skill_proposer::SkillProposal;
use captain_skills::{bundled, SkillManifest};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_DUPLICATE_SCORE: u8 = 82;
const SUPPORTING_SKILL_DIRS: &[&str] = &["references", "templates", "scripts", "assets"];

#[derive(Debug, Clone)]
pub struct SkillDiffConfig {
    pub roots: Vec<PathBuf>,
    pub include_bundled: bool,
    pub duplicate_score: u8,
}

impl SkillDiffConfig {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            include_bundled: true,
            duplicate_score: DEFAULT_DUPLICATE_SCORE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExistingSkill {
    pub name: String,
    pub source: String,
    pub description: String,
    pub trigger_hint: String,
    pub tool_names: Vec<String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillDiffMatch {
    pub existing_name: String,
    pub source: String,
    pub score: u8,
    pub reason: String,
}

pub fn find_duplicate(proposal: &SkillProposal, cfg: &SkillDiffConfig) -> Option<SkillDiffMatch> {
    let existing = load_existing_skills(cfg);
    existing
        .iter()
        .filter_map(|skill| compare(proposal, skill))
        .filter(|m| m.score >= cfg.duplicate_score)
        .max_by_key(|m| m.score)
}

pub fn compare(proposal: &SkillProposal, existing: &ExistingSkill) -> Option<SkillDiffMatch> {
    let prop_name = normalize_slug(&proposal.name);
    let existing_name = normalize_slug(&existing.name);
    if !prop_name.is_empty() && prop_name == existing_name {
        return Some(SkillDiffMatch {
            existing_name: existing.name.clone(),
            source: existing.source.clone(),
            score: 100,
            reason: "same_skill_name".to_string(),
        });
    }

    let name_score = token_jaccard(&proposal.name, &existing.name);
    let text_score = token_jaccard(&proposal_text(proposal), &existing_text(existing));
    let tool_score = tool_overlap(&proposal.tool_sequence, &existing.tool_names);
    let weighted = (name_score * 0.25) + (text_score * 0.55) + (tool_score * 0.20);
    let mut score = (weighted * 100.0).round().clamp(0.0, 100.0) as u8;
    if score == 0 {
        return None;
    }
    let reason = if tool_score >= 0.8 && text_score >= 0.45 {
        score = score.max(86);
        "same_tools_and_similar_procedure"
    } else if tool_score >= 0.8 && text_score >= 0.32 && name_score >= 0.2 {
        score = score.max(82);
        "same_tools_and_related_workflow"
    } else if text_score >= 0.72 {
        "similar_procedure"
    } else if name_score >= 0.85 {
        "similar_name"
    } else {
        "weak_similarity"
    };
    Some(SkillDiffMatch {
        existing_name: existing.name.clone(),
        source: existing.source.clone(),
        score,
        reason: reason.to_string(),
    })
}

pub fn load_existing_skills(cfg: &SkillDiffConfig) -> Vec<ExistingSkill> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    if cfg.include_bundled {
        for (name, raw) in bundled::bundled_skills() {
            if let Ok(manifest) = bundled::parse_bundled(name, raw) {
                let skill = existing_from_manifest(
                    &manifest,
                    format!("<bundled>/{name}"),
                    manifest
                        .prompt_context
                        .clone()
                        .unwrap_or_else(|| raw.to_string()),
                );
                push_unique(&mut out, &mut seen, skill);
            }
        }
    }

    for root in &cfg.roots {
        scan_root(root, &mut out, &mut seen);
    }

    out
}

fn scan_root(root: &Path, out: &mut Vec<ExistingSkill>, seen: &mut HashSet<String>) {
    if !root.exists() {
        return;
    }
    scan_dir(root, 0, out, seen);
}

fn scan_dir(dir: &Path, depth: usize, out: &mut Vec<ExistingSkill>, seen: &mut HashSet<String>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(skill) = load_skill_dir(&path) {
                push_unique(out, seen, skill);
            }
            scan_dir(&path, depth + 1, out, seen);
        } else if file_type.is_file() && is_indexable_markdown_skill(&path) {
            if let Some(skill) = load_markdown_skill(&path) {
                push_unique(out, seen, skill);
            }
        }
    }
}

fn load_skill_dir(dir: &Path) -> Option<ExistingSkill> {
    let manifest_path = dir.join("skill.toml");
    if manifest_path.exists() {
        let raw = read_small(&manifest_path)?;
        let manifest: SkillManifest = toml::from_str(&raw).ok()?;
        let body = dir
            .join("prompt_context.md")
            .exists()
            .then(|| read_small(&dir.join("prompt_context.md")))
            .flatten()
            .or_else(|| manifest.prompt_context.clone())
            .unwrap_or_default();
        return Some(existing_from_manifest(
            &manifest,
            dir.display().to_string(),
            body,
        ));
    }

    let skill_md = dir.join("SKILL.md");
    if skill_md.exists() {
        return load_markdown_skill(&skill_md);
    }
    None
}

fn is_indexable_markdown_skill(path: &Path) -> bool {
    if path.extension().and_then(|s| s.to_str()) != Some("md") {
        return false;
    }
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name == "DESCRIPTION.md" {
        return false;
    }
    if file_name == "SKILL.md" {
        return true;
    }
    !path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|part| SUPPORTING_SKILL_DIRS.contains(&part))
    })
}

fn existing_from_manifest(manifest: &SkillManifest, source: String, body: String) -> ExistingSkill {
    let mut tool_names: BTreeSet<String> = manifest
        .tools
        .provided
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    tool_names.extend(manifest.requirements.tools.iter().cloned());
    tool_names.extend(extract_tool_names(&body));
    ExistingSkill {
        name: manifest.skill.name.clone(),
        source,
        description: manifest.skill.description.clone(),
        trigger_hint: String::new(),
        tool_names: tool_names.into_iter().collect(),
        body,
    }
}

fn load_markdown_skill(path: &Path) -> Option<ExistingSkill> {
    let raw = read_small(path)?;
    let name = frontmatter_value(&raw, "name")
        .or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })?;
    let description = frontmatter_value(&raw, "description")
        .or_else(|| first_nonempty_markdown_line(&raw))
        .unwrap_or_default();
    let trigger_hint = section_after_heading(&raw, "trigger").unwrap_or_default();
    let tool_names = extract_tool_names(&raw);
    Some(ExistingSkill {
        name,
        source: path.display().to_string(),
        description,
        trigger_hint,
        tool_names,
        body: raw,
    })
}

fn push_unique(out: &mut Vec<ExistingSkill>, seen: &mut HashSet<String>, skill: ExistingSkill) {
    let key = format!("{}|{}", normalize_slug(&skill.name), skill.source);
    if seen.insert(key) {
        out.push(skill);
    }
}

fn read_small(path: &Path) -> Option<String> {
    let metadata = path.metadata().ok()?;
    if metadata.len() > 128 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn proposal_text(p: &SkillProposal) -> String {
    format!(
        "{} {} {} {} {}",
        p.name,
        p.description,
        p.trigger_hint,
        p.arg_schema_hint,
        p.tool_sequence.join(" ")
    )
}

fn existing_text(s: &ExistingSkill) -> String {
    format!(
        "{} {} {} {} {}",
        s.name,
        s.description,
        s.trigger_hint,
        s.tool_names.join(" "),
        s.body
    )
}

fn frontmatter_value(raw: &str, key: &str) -> Option<String> {
    let mut lines = raw.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let prefix = format!("{key}:");
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix(&prefix) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn first_nonempty_markdown_line(raw: &str) -> Option<String> {
    raw.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("---")
                && !line.starts_with('#')
                && !line.starts_with("name:")
        })
        .map(|line| line.trim_start_matches("- ").to_string())
}

fn section_after_heading(raw: &str, heading: &str) -> Option<String> {
    let heading_lower = heading.to_ascii_lowercase();
    let mut capture = false;
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let title = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
            if capture {
                break;
            }
            capture = title == heading_lower;
            continue;
        }
        if capture && !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

fn extract_tool_names(raw: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for token in
        raw.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.'))
    {
        if token.contains('_') && token.len() >= 4 {
            out.insert(token.to_string());
        }
    }
    out.into_iter().collect()
}

fn tool_overlap(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a: BTreeSet<String> = a.iter().map(|s| normalize_slug(s)).collect();
    let b: BTreeSet<String> = b.iter().map(|s| normalize_slug(s)).collect();
    let intersection = a.iter().filter(|tok| b.contains(*tok)).count();
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        0.0
    } else {
        intersection as f32 / smaller as f32
    }
}

fn token_jaccard(a: &str, b: &str) -> f32 {
    let a = tokens(a);
    let b = tokens(b);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.iter().filter(|tok| b.contains(*tok)).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|tok| tok.len() >= 3 && !STOPWORDS.contains(&tok.as_str()))
        .collect()
}

fn normalize_slug(value: &str) -> String {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "when",
    "then",
    "that",
    "this",
    "from",
    "into",
    "user",
    "asks",
    "captain",
    "skill",
    "workflow",
    "proposal",
    "query",
    "string",
    "write",
    "writes",
    "summary",
    "summaries",
];

#[cfg(test)]
#[path = "skill_diff_tests.rs"]
mod skill_diff_tests;
