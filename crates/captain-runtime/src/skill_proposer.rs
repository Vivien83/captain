//! Skill proposer — LLM judge over recurring tool patterns (v3.13b).
//!
//! Consumes `SkillPatternCandidate` events from the pattern detector,
//! asks a cheap LLM whether the pattern deserves to become a Captain
//! skill, and (when yes) returns a drafted `SkillProposal`. Every
//! judged pattern — kept or skipped — is marked `proposed_at` in the
//! storage so the same recurrence does not re-invoke the LLM.
//!
//! Reuses `ReflectionCompleter` from v3.12d. No new driver surface.

use async_trait::async_trait;
use captain_skills::families::infer_manifest_family;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::pattern_detector::SkillPatternCandidate;
use crate::reflection_job::ReflectionCompleter;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillProposal {
    /// Slug-safe skill name. The writer (v3.13d) performs the final
    /// sanitization; the model is asked to produce `[a-z0-9-]{1,48}`.
    pub name: String,
    pub description: String,
    /// One-sentence hint describing when the skill should fire.
    pub trigger_hint: String,
    pub tool_sequence: Vec<String>,
    /// Informal sketch of expected arguments for the skill call.
    pub arg_schema_hint: String,
    pub confidence: f32,
    /// Discovery family used by `skill_search`. The file remains in the
    /// configured generated-skills directory; family is metadata.
    #[serde(default)]
    pub family: Option<String>,
    /// Source pattern hash so the writer can link back.
    pub pattern_hash: String,
    /// Conversation channel that produced the repeated tool pattern, if known.
    /// The proposal stays approval-only, but this lets the UI surface it in
    /// the active CLI/Telegram chat instead of hiding it in a dashboard queue.
    pub origin_channel: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProposerConfig {
    pub primary_model: String,
    pub fallback_models: Vec<String>,
    pub timeout_secs: u64,
    pub min_confidence: f32,
    /// User-facing language for generated descriptions and trigger hints.
    pub language: String,
}

impl Default for ProposerConfig {
    fn default() -> Self {
        Self {
            // Proposals drive file writes, so start with a stronger
            // default than v3.12d's cheap kimi path. Operators can
            // downgrade via `[skills]` config (v3.13e).
            primary_model: "anthropic/claude-haiku-4.5".to_string(),
            fallback_models: vec![
                "anthropic/claude-sonnet-4.6".to_string(),
                "moonshotai/kimi-k2.6".to_string(),
            ],
            timeout_secs: 30,
            min_confidence: 0.7,
            language: "en".to_string(),
        }
    }
}

impl From<&captain_types::config::SkillsConfig> for ProposerConfig {
    fn from(s: &captain_types::config::SkillsConfig) -> Self {
        Self {
            primary_model: s.proposer_model.clone(),
            fallback_models: s.fallback_models.clone(),
            timeout_secs: s.reflection_timeout_secs,
            min_confidence: s.min_confidence,
            language: "en".to_string(),
        }
    }
}

impl ProposerConfig {
    pub fn apply_autonomy_aggressiveness(&mut self, aggressiveness: f32) {
        self.min_confidence =
            crate::reflection_job::scale_confidence_floor(self.min_confidence, aggressiveness);
    }
}

// ---------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------

const SYSTEM_PROMPT: &str = "You judge whether a recurring tool sequence \
deserves to be packaged as a reusable Captain skill.\n\n\
INPUTS: an ordered list of tool calls an agent has made several times.\n\
OUTPUT: ONLY a single JSON object. No prose, no markdown, no code fences.\n\n\
Two valid shapes:\n\
A) {\"skip\": true, \"reason\": \"short explanation\"}\n\
B) {\"name\":\"slug-here\",\"description\":\"one-sentence summary\",\"trigger_hint\":\"when the skill should fire\",\"arg_schema_hint\":\"informal args description\",\"family\":\"software-development|project-management|review-release|platform-devops|data-ai|product-design|business-tools|security-compliance|general-automation\",\"confidence\":0..1}\n\n\
RULES:\n\
1. Skip if the sequence is trivial (single tool repeated), overly generic (read then write a file), or unsafe (arbitrary shell with no context).\n\
2. Name MUST match ^[a-z0-9-]{3,48}$. No spaces, no underscores, no slashes.\n\
3. Confidence reflects how reliably this pattern maps to a single intent. Below 0.7 = skip instead.\n\
4. Never propose a skill that would embed secrets or user-specific paths.\n\
5. Pick the closest family. If uncertain, use general-automation. The system will verify/correct it.\n\
6. Never invent a new family id.\n\
7. If in doubt, skip.";

fn language_label(language: &str) -> &'static str {
    let lang = language.trim().to_ascii_lowercase();
    if lang.starts_with("fr") || lang.contains("français") || lang.contains("francais") {
        "French"
    } else {
        "English"
    }
}

fn language_is_french(language: &str) -> bool {
    language_label(language) == "French"
}

fn system_prompt_for_language(language: &str) -> String {
    format!(
        "{SYSTEM_PROMPT}\n\
         6. CRITICAL: write description, trigger_hint, arg_schema_hint, and skip reason in {language}. \
         Do not mix English into user-facing fields unless the requested language is English. \
         Keep the skill name slug in ASCII.",
        language = language_label(language)
    )
}

pub fn default_procedural_trigger_hint(subject: &str, predicate: &str, language: &str) -> String {
    if language_label(language) == "French" {
        format!(
            "une future tâche correspond à `{subject}` / `{predicate}` et nécessite ce workflow réutilisable."
        )
    } else {
        format!(
            "When a future task matches `{subject}` / `{predicate}` and needs this reusable workflow."
        )
    }
}

pub fn default_procedural_arg_schema_hint(language: &str) -> String {
    if language_label(language) == "French" {
        "Capturé depuis un apprentissage en attente. Relis et ajoute les commandes/outils exacts avant approbation."
            .to_string()
    } else {
        "Captured from staged learning. Review and add exact commands/tools before approval."
            .to_string()
    }
}

pub fn localize_trigger_hint(trigger_hint: &str, language: &str) -> String {
    let trimmed = trigger_hint.trim();
    if !language_is_french(language) {
        return trimmed.to_string();
    }
    const PREFIX: &str = "When a future task matches ";
    const SUFFIX: &str = " and needs this reusable workflow.";
    if let Some(inner) = trimmed
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(SUFFIX))
    {
        return format!(
            "une future tâche correspond à {inner} et nécessite ce workflow réutilisable."
        );
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "user asks for a health check" || lower == "when user asks for a health check" {
        return "l'utilisateur demande un contrôle de santé.".to_string();
    }
    if lower == "user asks to research a topic"
        || lower == "when user asks to research a topic"
        || lower == "when the user asks to research a topic"
    {
        return "l'utilisateur demande de rechercher un sujet.".to_string();
    }
    if looks_like_english_sentence(trimmed) {
        return "une future tâche correspond à ce workflow réutilisable.".to_string();
    }
    trimmed.to_string()
}

pub fn localize_skill_description(
    description: &str,
    name: &str,
    tool_sequence: &[String],
    language: &str,
) -> String {
    let trimmed = description.trim();
    if !language_is_french(language) || trimmed.is_empty() || !looks_like_english_sentence(trimmed)
    {
        return trimmed.to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "checks service health" {
        return "Vérifie l'état de santé d'un service.".to_string();
    }
    if lower == "searches the web then writes a markdown summary" {
        return "Recherche sur le web puis rédige un résumé Markdown.".to_string();
    }
    if tool_sequence.is_empty() {
        format!("Workflow réutilisable proposé pour `{name}`.")
    } else {
        format!(
            "Workflow réutilisable proposé pour `{name}` avec les outils {}.",
            tool_sequence.join(" → ")
        )
    }
}

fn looks_like_english_sentence(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let markers = [
        "user asks",
        "when user",
        "when the user",
        "checks ",
        "searches ",
        "creates ",
        "writes ",
        "reads ",
        "fetches ",
        "runs ",
        "uses ",
        "then ",
        "health check",
        "summary",
    ];
    markers.iter().any(|marker| lower.contains(marker))
        && !lower.contains("l'utilisateur")
        && !lower.contains("utilisateur")
        && !lower.contains("quand")
        && !lower.contains("vérifie")
        && !lower.contains("verifie")
}

pub fn localize_skill_proposal_value(value: &mut serde_json::Value, language: &str) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(description) = obj.get("description").and_then(|v| v.as_str()) {
        let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let tool_sequence = obj
            .get("tool_sequence")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        obj.insert(
            "description".to_string(),
            serde_json::json!(localize_skill_description(
                description,
                name,
                &tool_sequence,
                language
            )),
        );
    }
    if let Some(trigger) = obj.get("trigger_hint").and_then(|v| v.as_str()) {
        obj.insert(
            "trigger_hint".to_string(),
            serde_json::json!(localize_trigger_hint(trigger, language)),
        );
    }
    if let Some(arg_hint) = obj.get("arg_schema_hint").and_then(|v| v.as_str()) {
        if arg_hint
            == "Captured from staged learning. Review and add exact commands/tools before approval."
            && language_is_french(language)
        {
            obj.insert(
                "arg_schema_hint".to_string(),
                serde_json::json!(default_procedural_arg_schema_hint(language)),
            );
        }
    }
}

pub fn normalize_skill_family(
    requested: Option<&str>,
    name: &str,
    description: &str,
    trigger_hint: &str,
    tool_sequence: &[String],
) -> String {
    if let Some(family) = requested.and_then(captain_skills::families::known_family) {
        return family.id.to_string();
    }

    let manifest = captain_skills::SkillManifest {
        skill: captain_skills::SkillMeta {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: format!("{description} {trigger_hint} {}", tool_sequence.join(" ")),
            author: String::new(),
            license: String::new(),
            tags: vec!["generated".to_string()],
        },
        runtime: captain_skills::SkillRuntimeConfig {
            runtime_type: captain_skills::SkillRuntime::PromptOnly,
            entry: String::new(),
        },
        tools: captain_skills::SkillTools::default(),
        requirements: Default::default(),
        prompt_context: None,
        source: Some(captain_skills::SkillSource::Native),
    };
    infer_manifest_family(&manifest).to_string()
}

pub fn proposal_why_text(tool_sequence: &[String], confidence: f32, language: &str) -> String {
    if tool_sequence.is_empty() {
        if language_is_french(language) {
            return format!(
                "Captain a identifié un workflow réutilisable décrit dans le résumé, sans trace d'outil automatique capturée. Vérifie que les étapes sont assez concrètes avant d'approuver. Confiance estimée : {:.0}%.",
                confidence * 100.0
            );
        }
        return format!(
            "Captain identified a reusable workflow described in the summary, with no automatic tool trace captured. Confirm the steps are concrete enough before approving. Estimated confidence: {:.0}%.",
            confidence * 100.0
        );
    }
    let tools = tool_sequence.join(" -> ");
    if language_is_french(language) {
        format!(
            "Captain a repéré un workflow réutilisable avec une trace d'outils concrète : {tools}. Confiance estimée : {:.0}%.",
            confidence * 100.0
        )
    } else {
        format!(
            "Captain detected a reusable workflow with a concrete tool trace: {tools}. Estimated confidence: {:.0}%.",
            confidence * 100.0
        )
    }
}

pub fn build_prompt(candidate: &SkillPatternCandidate) -> (String, String) {
    build_prompt_with_language(candidate, "en")
}

pub fn build_prompt_with_language(
    candidate: &SkillPatternCandidate,
    language: &str,
) -> (String, String) {
    let tools = candidate
        .tool_sequence
        .iter()
        .enumerate()
        .map(|(i, t)| format!("  {}. {}", i + 1, t))
        .collect::<Vec<_>>()
        .join("\n");
    let first = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(candidate.first_seen)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| candidate.first_seen.to_string());
    let last = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(candidate.last_seen)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| candidate.last_seen.to_string());
    let user = format!(
        "Agent {agent} has run the following tool sequence {count} times \
         between {first} and {last}:\n\n{tools}\n\n\
         Should this become a skill? Emit JSON per the system rules.",
        agent = candidate.agent_id,
        count = candidate.count,
        first = first,
        last = last,
        tools = tools,
    );
    (system_prompt_for_language(language), user)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub fn parse_proposal(raw: &str, pattern_hash: &str, tool_sequence: &[String]) -> ParseOutcome {
    let Some(slice) = extract_json_object(raw) else {
        debug!("skill_proposer parse: no JSON object found");
        return ParseOutcome::Invalid;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(slice) else {
        debug!("skill_proposer parse: invalid JSON");
        return ParseOutcome::Invalid;
    };
    if value.get("skip").and_then(|v| v.as_bool()).unwrap_or(false) {
        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("model skipped")
            .to_string();
        return ParseOutcome::Skip(reason);
    }
    // Shape B fields
    let obj = match value.as_object() {
        Some(o) => o,
        None => return ParseOutcome::Invalid,
    };
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let description = obj
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let trigger_hint = obj
        .get("trigger_hint")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let arg_schema_hint = obj
        .get("arg_schema_hint")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let confidence = obj
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let family = normalize_skill_family(
        obj.get("family").and_then(|v| v.as_str()),
        name,
        description,
        &trigger_hint,
        tool_sequence,
    );
    if name.is_empty() || description.is_empty() {
        return ParseOutcome::Invalid;
    }
    ParseOutcome::Propose(SkillProposal {
        name: name.to_string(),
        description: description.to_string(),
        trigger_hint,
        tool_sequence: tool_sequence.to_vec(),
        arg_schema_hint,
        confidence,
        family: Some(family),
        pattern_hash: pattern_hash.to_string(),
        origin_channel: None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseOutcome {
    Propose(SkillProposal),
    Skip(String),
    Invalid,
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ProposerOutcome {
    Proposed(SkillProposal),
    Skipped(String),
    Failed,
}

pub async fn run_proposer(
    completer: &dyn ReflectionCompleter,
    cfg: &ProposerConfig,
    candidate: &SkillPatternCandidate,
) -> ProposerOutcome {
    let (system, user) = build_prompt_with_language(candidate, &cfg.language);
    let timeout_dur = Duration::from_secs(cfg.timeout_secs);
    let mut chain = vec![cfg.primary_model.clone()];
    chain.extend(cfg.fallback_models.iter().cloned());

    for model in &chain {
        match timeout(timeout_dur, completer.complete(model, &system, &user)).await {
            Ok(Ok(raw)) => match parse_proposal(&raw, &candidate.hash, &candidate.tool_sequence) {
                ParseOutcome::Propose(mut p) => {
                    if p.confidence < cfg.min_confidence {
                        debug!(
                            model = %model,
                            confidence = p.confidence,
                            min = cfg.min_confidence,
                            "skill_proposer: confidence below threshold"
                        );
                        return ProposerOutcome::Skipped("low_confidence".into());
                    }
                    p.origin_channel = candidate.origin_channel.clone();
                    info!(
                        model = %model,
                        name = %p.name,
                        confidence = p.confidence,
                        "skill_proposer produced proposal"
                    );
                    return ProposerOutcome::Proposed(p);
                }
                ParseOutcome::Skip(reason) => {
                    debug!(model = %model, reason = %reason, "skill_proposer skipped");
                    return ProposerOutcome::Skipped(reason);
                }
                ParseOutcome::Invalid => {
                    warn!(model = %model, "skill_proposer: invalid output, trying next");
                }
            },
            Ok(Err(e)) => warn!(model = %model, error = %e, "skill_proposer call failed"),
            Err(_) => warn!(model = %model, "skill_proposer timed out"),
        }
    }
    warn!("skill_proposer chain exhausted — giving up");
    ProposerOutcome::Failed
}

/// Spawn the consumer loop. For each `SkillPatternCandidate`:
/// 1. Run the LLM judge (`run_proposer`).
/// 2. Mark the source pattern as `proposed_at` (skip or propose — one
///    judgment per pattern, period). This is the only cost control;
///    failures do NOT mark so the pattern can be judged again on the
///    next matching event.
/// 3. Forward a `SkillProposal` downstream only when the model kept it.
pub fn spawn_consumer(
    mut rx: mpsc::Receiver<SkillPatternCandidate>,
    completer: Arc<dyn ReflectionCompleter>,
    cfg: ProposerConfig,
    conn: Arc<StdMutex<Connection>>,
    output_capacity: usize,
) -> (tokio::task::JoinHandle<()>, mpsc::Receiver<SkillProposal>) {
    let (tx, out_rx) = mpsc::channel(output_capacity);
    let handle = tokio::spawn(async move {
        while let Some(cand) = rx.recv().await {
            let outcome = run_proposer(completer.as_ref(), &cfg, &cand).await;
            match outcome {
                ProposerOutcome::Proposed(p) => {
                    mark_pattern_proposed(&conn, &cand.hash);
                    if let Err(e) = tx.try_send(p) {
                        debug!(error = %e, "skill_proposer consumer: downstream full");
                    }
                }
                ProposerOutcome::Skipped(_) => {
                    mark_pattern_proposed(&conn, &cand.hash);
                }
                ProposerOutcome::Failed => {
                    // Do NOT mark — allow retry on next pattern event.
                }
            }
        }
    });
    (handle, out_rx)
}

fn mark_pattern_proposed(conn: &StdMutex<Connection>, hash: &str) {
    let Ok(guard) = conn.lock() else {
        warn!("skill_proposer: sqlite poisoned, cannot mark_proposed");
        return;
    };
    if let Err(e) = captain_memory::skill_patterns::mark_proposed(&guard, hash) {
        warn!(error = %e, hash, "skill_proposer: mark_proposed failed");
    }
}

// ---------------------------------------------------------------------------
// Shim: allow a NoopCompleter-style probe so v3.13b is wire-testable
// without a live LLM.
// ---------------------------------------------------------------------------

/// Deterministic completer that always accepts a canned JSON string.
/// Useful in tests for the proposer pipeline.
pub struct EchoCompleter {
    pub response: String,
}

#[async_trait]
impl ReflectionCompleter for EchoCompleter {
    async fn complete(&self, _m: &str, _s: &str, _u: &str) -> Result<String, String> {
        Ok(self.response.clone())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::migration::run_migrations;

    fn fresh_db() -> Arc<StdMutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        Arc::new(StdMutex::new(conn))
    }

    fn sample_candidate() -> SkillPatternCandidate {
        SkillPatternCandidate {
            hash: "h123".into(),
            agent_id: "captain".into(),
            tool_sequence: vec!["web_search".into(), "web_fetch".into(), "file_write".into()],
            count: 5,
            first_seen: 1_700_000_000_000,
            last_seen: 1_700_000_300_000,
            origin_channel: Some("telegram".into()),
        }
    }

    fn cfg() -> ProposerConfig {
        ProposerConfig {
            primary_model: "model-a".into(),
            fallback_models: vec!["model-b".into()],
            timeout_secs: 2,
            min_confidence: 0.7,
            language: "fr".into(),
        }
    }

    // ---- prompt ----

    #[test]
    fn prompt_lists_all_tools_numbered() {
        let (_sys, user) = build_prompt(&sample_candidate());
        assert!(user.contains("1. web_search"));
        assert!(user.contains("2. web_fetch"));
        assert!(user.contains("3. file_write"));
    }

    #[test]
    fn prompt_uses_configured_output_language() {
        let (sys, _user) = build_prompt_with_language(&sample_candidate(), "fr");
        assert!(sys.contains("write description"));
        assert!(sys.contains("in French"));
    }

    #[test]
    fn localize_legacy_trigger_hint_to_french() {
        let hint = localize_trigger_hint(
            "When a future task matches `Smoke check approach` / `uses command` and needs this reusable workflow.",
            "fr",
        );
        assert_eq!(
            hint,
            "une future tâche correspond à `Smoke check approach` / `uses command` et nécessite ce workflow réutilisable."
        );
    }

    #[test]
    fn localize_common_english_skill_fields_to_french() {
        assert_eq!(
            localize_trigger_hint("user asks for a health check", "fr"),
            "l'utilisateur demande un contrôle de santé."
        );
        assert_eq!(
            localize_skill_description("Checks service health", "status-checker", &[], "fr"),
            "Vérifie l'état de santé d'un service."
        );
    }

    #[test]
    fn localize_unknown_english_description_to_french_fallback() {
        let description = localize_skill_description(
            "Creates a smoke report then writes summary",
            "smoke-report",
            &["shell_exec".into(), "file_write".into()],
            "fr",
        );
        assert!(description.starts_with("Workflow réutilisable proposé"));
        assert!(description.contains("shell_exec → file_write"));
        assert!(!description.contains("Creates"));
    }

    #[test]
    fn localize_skill_proposal_value_updates_description_and_trigger() {
        let mut value = serde_json::json!({
            "name": "status-checker",
            "description": "Checks service health",
            "trigger_hint": "user asks for a health check",
            "tool_sequence": ["ssh_exec"]
        });
        localize_skill_proposal_value(&mut value, "fr");
        assert_eq!(
            value["description"].as_str(),
            Some("Vérifie l'état de santé d'un service.")
        );
        assert_eq!(
            value["trigger_hint"].as_str(),
            Some("l'utilisateur demande un contrôle de santé.")
        );
    }

    #[test]
    fn prompt_includes_count_and_agent() {
        let (_sys, user) = build_prompt(&sample_candidate());
        assert!(user.contains("captain"));
        assert!(user.contains("5 times"));
    }

    #[test]
    fn system_prompt_forbids_secrets_and_requires_json_only() {
        let (sys, _u) = build_prompt(&sample_candidate());
        let lower = sys.to_lowercase();
        assert!(lower.contains("json"));
        assert!(lower.contains("secret"));
        assert!(lower.contains("skip"));
    }

    // ---- parse ----

    #[test]
    fn parse_skip_shape_returns_skip() {
        let raw = r#"{"skip": true, "reason": "too trivial"}"#;
        let out = parse_proposal(raw, "h", &[]);
        assert!(matches!(out, ParseOutcome::Skip(r) if r == "too trivial"));
    }

    #[test]
    fn parse_proposal_shape_returns_propose() {
        let raw = r#"{"name":"research-and-log","description":"search then fetch then write","trigger_hint":"when user asks to research","arg_schema_hint":"query:string","confidence":0.85}"#;
        match parse_proposal(raw, "h", &["a".into()]) {
            ParseOutcome::Propose(p) => {
                assert_eq!(p.name, "research-and-log");
                assert_eq!(p.confidence, 0.85);
                assert_eq!(p.family.as_deref(), Some("general-automation"));
                assert_eq!(p.pattern_hash, "h");
                assert_eq!(p.tool_sequence, vec!["a".to_string()]);
                assert_eq!(p.origin_channel, None);
            }
            other => panic!("expected Propose, got {other:?}"),
        }
    }

    #[test]
    fn parse_tolerates_prose_around_json() {
        let raw = "Sure — here's my judgment:\n{\"skip\": true}\nHope that helps!";
        assert!(matches!(
            parse_proposal(raw, "h", &[]),
            ParseOutcome::Skip(_)
        ));
    }

    #[test]
    fn parse_missing_required_fields_is_invalid() {
        let raw = r#"{"name":"ok","confidence":0.9}"#;
        assert!(matches!(
            parse_proposal(raw, "h", &[]),
            ParseOutcome::Invalid
        ));
    }

    #[test]
    fn parse_non_json_is_invalid() {
        assert!(matches!(
            parse_proposal("nothing to see here", "h", &[]),
            ParseOutcome::Invalid
        ));
    }

    // ---- runner ----

    #[tokio::test]
    async fn run_proposer_returns_proposed_on_valid_response() {
        let c = EchoCompleter {
            response: r#"{"name":"research-log","description":"x","trigger_hint":"y","arg_schema_hint":"z","confidence":0.85}"#.into(),
        };
        match run_proposer(&c, &cfg(), &sample_candidate()).await {
            ProposerOutcome::Proposed(p) => {
                assert_eq!(p.name, "research-log");
                assert_eq!(p.origin_channel.as_deref(), Some("telegram"));
                assert_eq!(p.family.as_deref(), Some("general-automation"));
            }
            other => panic!("expected Proposed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_proposer_returns_skipped_on_model_skip() {
        let c = EchoCompleter {
            response: r#"{"skip":true,"reason":"trivial"}"#.into(),
        };
        assert!(matches!(
            run_proposer(&c, &cfg(), &sample_candidate()).await,
            ProposerOutcome::Skipped(_)
        ));
    }

    #[tokio::test]
    async fn run_proposer_skips_low_confidence() {
        let c = EchoCompleter {
            response: r#"{"name":"x","description":"y","trigger_hint":"z","arg_schema_hint":"","confidence":0.3}"#.into(),
        };
        let out = run_proposer(&c, &cfg(), &sample_candidate()).await;
        assert!(matches!(out, ProposerOutcome::Skipped(r) if r == "low_confidence"));
    }

    struct FailCompleter;
    #[async_trait]
    impl ReflectionCompleter for FailCompleter {
        async fn complete(&self, _m: &str, _s: &str, _u: &str) -> Result<String, String> {
            Err("boom".into())
        }
    }

    #[tokio::test]
    async fn run_proposer_returns_failed_when_chain_exhausted() {
        assert!(matches!(
            run_proposer(&FailCompleter, &cfg(), &sample_candidate()).await,
            ProposerOutcome::Failed
        ));
    }

    // ---- consumer + mark_proposed ----

    #[tokio::test]
    async fn consumer_marks_proposed_on_skip_but_does_not_forward() {
        let db = fresh_db();
        // Seed the pattern row so mark_proposed has something to update.
        {
            let guard = db.lock().unwrap();
            captain_memory::skill_patterns::incr_or_insert(
                &guard,
                "h123",
                "captain",
                &["t1".to_string(), "t2".into(), "t3".into()],
            )
            .unwrap();
        }

        let (tx, rx) = mpsc::channel(4);
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(EchoCompleter {
            response: r#"{"skip":true,"reason":"too generic"}"#.into(),
        });
        let (_h, mut out_rx) = spawn_consumer(rx, completer, cfg(), db.clone(), 4);

        tx.send(sample_candidate()).await.unwrap();
        drop(tx);

        let extra = tokio::time::timeout(Duration::from_millis(100), out_rx.recv()).await;
        assert!(extra.is_err() || extra.unwrap().is_none());

        let guard = db.lock().unwrap();
        let row = captain_memory::skill_patterns::get(&guard, "h123")
            .unwrap()
            .unwrap();
        assert!(row.proposed_at.is_some(), "skip still marks proposed");
    }

    #[tokio::test]
    async fn consumer_forwards_and_marks_on_propose() {
        let db = fresh_db();
        {
            let guard = db.lock().unwrap();
            captain_memory::skill_patterns::incr_or_insert(
                &guard,
                "h123",
                "captain",
                &["t1".to_string()],
            )
            .unwrap();
        }

        let (tx, rx) = mpsc::channel(4);
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(EchoCompleter {
            response: r#"{"name":"research-log","description":"x","trigger_hint":"y","arg_schema_hint":"z","confidence":0.9}"#.into(),
        });
        let (_h, mut out_rx) = spawn_consumer(rx, completer, cfg(), db.clone(), 4);

        tx.send(sample_candidate()).await.unwrap();
        drop(tx);

        let proposal = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .expect("proposal must arrive")
            .expect("channel yields");
        assert_eq!(proposal.name, "research-log");
        assert_eq!(proposal.family.as_deref(), Some("general-automation"));

        let guard = db.lock().unwrap();
        let row = captain_memory::skill_patterns::get(&guard, "h123")
            .unwrap()
            .unwrap();
        assert!(row.proposed_at.is_some());
    }

    #[tokio::test]
    async fn consumer_does_not_mark_on_failure() {
        let db = fresh_db();
        {
            let guard = db.lock().unwrap();
            captain_memory::skill_patterns::incr_or_insert(
                &guard,
                "h123",
                "captain",
                &["t1".to_string()],
            )
            .unwrap();
        }

        let (tx, rx) = mpsc::channel(4);
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(FailCompleter);
        let (_h, _out_rx) = spawn_consumer(rx, completer, cfg(), db.clone(), 4);

        tx.send(sample_candidate()).await.unwrap();
        drop(tx);

        // Give the spawned task a moment to drain + fail + give up.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let guard = db.lock().unwrap();
        let row = captain_memory::skill_patterns::get(&guard, "h123")
            .unwrap()
            .unwrap();
        assert!(
            row.proposed_at.is_none(),
            "failure must not consume the re-evaluation quota"
        );
    }
}
