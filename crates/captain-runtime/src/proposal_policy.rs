//! Proposal policy (v3.13d.1).
//!
//! Security + quality gate for `SkillProposal` drafts before they land
//! in the review queue. Reuses v3.12e's secret + injection scanners
//! and adds skill-specific checks: slug shape, name collision with
//! existing proposals, and a global daily rate limit.
//!
//! The policy is cheap and deterministic — no LLM, no embeddings.

use regex_lite::Regex;
use rusqlite::{params, Connection};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

use crate::memory_policy::scan_for_secrets;
use crate::prompt_sanitizer::{scan_for_injection, ScanResult};
use crate::skill_diff::{find_duplicate, SkillDiffConfig};
use crate::skill_proposer::SkillProposal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyVerdict {
    Accept,
    RejectBadName(&'static str),
    RejectLowSignal(&'static str),
    RejectInjection(&'static str),
    RejectSecret(&'static str),
    RejectNameCollision,
    RejectExistingSkillDuplicate {
        skill: String,
        score: u8,
        reason: String,
    },
    RejectRateLimited,
}

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub max_per_day: u32,
    pub min_confidence: f32,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            max_per_day: 3,
            min_confidence: 0.7,
        }
    }
}

impl From<&captain_types::config::SkillsConfig> for PolicyConfig {
    fn from(s: &captain_types::config::SkillsConfig) -> Self {
        Self {
            max_per_day: s.rate_limit_per_day,
            min_confidence: s.min_confidence,
        }
    }
}

impl PolicyConfig {
    pub fn apply_autonomy_aggressiveness(&mut self, aggressiveness: f32) {
        self.max_per_day =
            crate::memory_policy::scale_daily_limit(self.max_per_day, aggressiveness);
        self.min_confidence =
            crate::reflection_job::scale_confidence_floor(self.min_confidence, aggressiveness);
    }
}

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]{3,48}$").expect("static regex"));

pub struct ProposalPolicy {
    cfg: PolicyConfig,
    skill_diff: Option<SkillDiffConfig>,
    daily: Mutex<DailyCounter>,
    clock: Box<dyn Clock>,
}

impl ProposalPolicy {
    pub fn new(cfg: PolicyConfig) -> Self {
        Self {
            cfg,
            skill_diff: None,
            daily: Mutex::new(DailyCounter::default()),
            clock: Box::new(SystemClock),
        }
    }

    pub fn with_skill_diff(cfg: PolicyConfig, skill_diff: SkillDiffConfig) -> Self {
        Self {
            cfg,
            skill_diff: Some(skill_diff),
            daily: Mutex::new(DailyCounter::default()),
            clock: Box::new(SystemClock),
        }
    }

    #[cfg(test)]
    fn with_clock(cfg: PolicyConfig, clock: Box<dyn Clock>) -> Self {
        Self {
            cfg,
            skill_diff: None,
            daily: Mutex::new(DailyCounter::default()),
            clock,
        }
    }

    /// Evaluate a proposal against every filter. Non-mutating on
    /// reject — only Accept consumes a rate-limit slot.
    pub fn evaluate(&self, proposal: &SkillProposal, conn: &Connection) -> PolicyVerdict {
        if !NAME_RE.is_match(&proposal.name) {
            return PolicyVerdict::RejectBadName("slug_shape");
        }
        if proposal.confidence < self.cfg.min_confidence {
            return PolicyVerdict::RejectBadName("low_confidence");
        }
        if let Some(reason) = low_signal_reason(proposal) {
            return PolicyVerdict::RejectLowSignal(reason);
        }

        let joined = format!(
            "{} {} {} {} {}",
            proposal.name,
            proposal.description,
            proposal.trigger_hint,
            proposal.arg_schema_hint,
            proposal.tool_sequence.join(" "),
        );
        if let ScanResult::Blocked(r) = scan_for_injection(&joined) {
            return PolicyVerdict::RejectInjection(r);
        }
        if let Some(kind) = scan_for_secrets(&joined) {
            return PolicyVerdict::RejectSecret(kind);
        }

        if let Some(diff_cfg) = &self.skill_diff {
            if let Some(matched) = find_duplicate(proposal, diff_cfg) {
                return PolicyVerdict::RejectExistingSkillDuplicate {
                    skill: matched.existing_name,
                    score: matched.score,
                    reason: matched.reason,
                };
            }
        }

        if let Ok(exists) = name_in_queue(conn, &proposal.name) {
            if exists {
                return PolicyVerdict::RejectNameCollision;
            }
        } else {
            debug!("proposal_policy: name lookup failed, treating as no collision");
        }

        let today = self.clock.unix_day();
        if !self.consume_slot(today) {
            return PolicyVerdict::RejectRateLimited;
        }

        PolicyVerdict::Accept
    }

    fn consume_slot(&self, today: i64) -> bool {
        let mut g = self.daily.lock().expect("poisoned policy mutex");
        if g.day != today {
            g.day = today;
            g.count = 0;
        }
        if g.count >= self.cfg.max_per_day {
            return false;
        }
        g.count += 1;
        true
    }
}

fn low_signal_reason(proposal: &SkillProposal) -> Option<&'static str> {
    let description = proposal.description.trim();
    let trigger = proposal.trigger_hint.trim();
    if description.len() < 18 || trigger.len() < 18 {
        return Some("underspecified");
    }
    if proposal.tool_sequence.is_empty() && !has_concrete_workflow_evidence(proposal) {
        return Some("missing_observed_steps");
    }
    None
}

fn has_concrete_workflow_evidence(proposal: &SkillProposal) -> bool {
    let text = format!(
        "{}\n{}\n{}\n{}",
        proposal.name, proposal.description, proposal.trigger_hint, proposal.arg_schema_hint
    );
    let lower = text.to_lowercase();
    let actions = marker_count(&lower, WORKFLOW_ACTION_MARKERS);
    let sequence_markers = marker_count(&lower, WORKFLOW_SEQUENCE_MARKERS);
    let has_structure = has_step_structure(&text);
    let has_artifact = has_workflow_artifact_marker(&lower);

    has_structure && actions >= 2
        || actions >= 3 && sequence_markers >= 1
        || actions >= 2 && sequence_markers >= 1 && has_artifact
}

const WORKFLOW_ACTION_MARKERS: &[&str] = &[
    "read ",
    "write ",
    "create ",
    "patch ",
    "update ",
    "verify",
    "check ",
    "run ",
    "execute",
    "call ",
    "use ",
    "fetch ",
    "parse ",
    "document",
    "search ",
    "compare ",
    "open ",
    "test ",
    "send ",
    "export ",
    "filter ",
    "build",
    "deploy",
    "smoke",
    "publish",
    "invoke",
    "lire",
    "écrire",
    "ecrire",
    "créer",
    "creer",
    "patcher",
    "mettre à jour",
    "mettre a jour",
    "vérifier",
    "verifier",
    "contrôler",
    "controler",
    "tester",
    "exécuter",
    "executer",
    "lancer",
    "appeler",
    "utiliser",
    "récupérer",
    "recuperer",
    "parser",
    "documenter",
    "chercher",
    "comparer",
    "ouvrir",
    "envoyer",
    "exporter",
    "filtrer",
    "builder",
    "déployer",
    "deployer",
    "publier",
    "invoquer",
];

const WORKFLOW_SEQUENCE_MARKERS: &[&str] = &[
    "1.", "2.", "3.", "step 1", "step 2", "étape 1", "étape 2", "etape 1", "etape 2", "then",
    "before ", "after ", "finally", "ensuite", "puis", "avant de", "après", "apres", "enfin",
];

const WORKFLOW_ARTIFACT_MARKERS: &[&str] = &[
    "get /", "post /", "put /", "patch /", "delete /", "head /", "curl ", "python ", "pytest ",
    "cargo ", "npm ", "git ", "gh ", "sql ", "endpoint", "api ", "webhook", "cli ", "command",
    "commande", "fichier", "file ",
];

fn marker_count(text: &str, markers: &[&str]) -> usize {
    markers
        .iter()
        .filter(|marker| text.contains(**marker))
        .count()
}

fn has_workflow_artifact_marker(text: &str) -> bool {
    WORKFLOW_ARTIFACT_MARKERS
        .iter()
        .any(|marker| text.contains(*marker))
}

fn has_step_structure(text: &str) -> bool {
    let line_steps = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || starts_with_numbered_step(trimmed)
        })
        .count();
    if line_steps >= 2 {
        return true;
    }
    let lower = text.to_lowercase();
    (lower.contains("1.") && lower.contains("2."))
        || (lower.contains("step 1") && lower.contains("step 2"))
        || (lower.contains("étape 1") && lower.contains("étape 2"))
        || (lower.contains("etape 1") && lower.contains("etape 2"))
}

fn starts_with_numbered_step(text: &str) -> bool {
    let chars = text.chars();
    let mut saw_digit = false;
    for c in chars {
        if c.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        return saw_digit && (c == '.' || c == ')');
    }
    false
}

/// Spawn the policy middleware. Drains `SkillProposal`s, runs
/// `evaluate` under a shared connection, and enqueues accepted drafts
/// into `skill_proposals`. Rejected drafts are logged at debug level
/// and dropped. Returns the join handle — the output channel returns
/// the ID of every row that landed in the queue so the UI can tail.
pub fn spawn_middleware(
    mut rx: tokio::sync::mpsc::Receiver<SkillProposal>,
    policy: std::sync::Arc<ProposalPolicy>,
    conn: std::sync::Arc<std::sync::Mutex<Connection>>,
    source_agent_id: String,
    output_capacity: usize,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Receiver<captain_memory::skill_proposals::Proposal>,
) {
    let (tx, out_rx) = tokio::sync::mpsc::channel(output_capacity);
    let handle = tokio::spawn(async move {
        while let Some(proposal) = rx.recv().await {
            let verdict = {
                let guard = match conn.lock() {
                    Ok(g) => g,
                    Err(e) => {
                        debug!(error = %e, "proposal_policy middleware: sqlite poisoned");
                        continue;
                    }
                };
                policy.evaluate(&proposal, &guard)
            };
            if verdict != PolicyVerdict::Accept {
                debug!(?verdict, name = %proposal.name, "proposal_policy rejected");
                continue;
            }
            let input = captain_memory::skill_proposals::NewProposal {
                pattern_hash: proposal.pattern_hash.clone(),
                name: proposal.name.clone(),
                description: proposal.description.clone(),
                trigger_hint: proposal.trigger_hint.clone(),
                tool_sequence: proposal.tool_sequence.clone(),
                arg_schema_hint: proposal.arg_schema_hint.clone(),
                confidence: proposal.confidence,
                family: crate::skill_proposer::normalize_skill_family(
                    proposal.family.as_deref(),
                    &proposal.name,
                    &proposal.description,
                    &proposal.trigger_hint,
                    &proposal.tool_sequence,
                ),
                source_agent_id: source_agent_id.clone(),
                origin_channel: proposal.origin_channel.clone(),
            };
            let row = {
                let guard = match conn.lock() {
                    Ok(g) => g,
                    Err(e) => {
                        debug!(error = %e, "proposal_policy middleware: sqlite poisoned on enqueue");
                        continue;
                    }
                };
                match captain_memory::skill_proposals::enqueue(&guard, input) {
                    Ok(row) => row,
                    Err(e) => {
                        debug!(error = %e, "proposal_policy middleware: enqueue failed");
                        continue;
                    }
                }
            };
            if let Err(e) = tx.try_send(row) {
                debug!(error = %e, "proposal_policy middleware: downstream full");
            }
        }
    });
    (handle, out_rx)
}

fn name_in_queue(conn: &Connection, name: &str) -> Result<bool, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM skill_proposals WHERE name = ?1 AND (status IS NULL OR status = 'approved')",
    )?;
    let n: i64 = stmt.query_row(params![name], |r| r.get(0))?;
    Ok(n > 0)
}

#[derive(Debug, Default, Clone, Copy)]
struct DailyCounter {
    day: i64,
    count: u32,
}

pub trait Clock: Send + Sync {
    fn unix_day(&self) -> i64;
}

struct SystemClock;
impl Clock for SystemClock {
    fn unix_day(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| (d.as_secs() / 86_400) as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
#[path = "proposal_policy_tests.rs"]
mod tests;
