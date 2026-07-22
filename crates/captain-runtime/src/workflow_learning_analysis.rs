//! Deterministic eligibility, canonical grouping, and routing for durable
//! workflow episodes. Model judgment starts only after this module has
//! established recurrence, safety, novelty, and a stable action graph.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use captain_memory::workflow_learning::WorkflowEpisodeEvidence;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow_learning_canonical::{canonical_signature, canonicalize_episode, tool_role};

const MAX_STEPS_PER_EPISODE: usize = 64;
const MIN_AUTOMATIC_EPISODES: usize = 3;
const MIN_AUTOMATIC_SESSIONS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowClassification {
    Memory,
    Refinement,
    Skill,
    Capspec,
    Automation,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingCapabilityKind {
    Skill,
    Capspec,
    Automation,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScope {
    Global,
    Project,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRejectionReason {
    EpisodeNotSuccessful,
    SecretBearingInput,
    FailedOrInterruptedAttempt,
    UnverifiedMutation,
    BackgroundNoise,
    MissingSteps,
    TooManySteps,
    MalformedEvidence,
    UnknownToolAuthority,
    NoActionableSteps,
    InsufficientRecurrence,
    OrdinarySingleStep,
    RepetitiveNoise,
    MemoryOnly,
    DuplicatePending,
    DuplicateExistingAutomation,
    DuplicateNativeCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalWorkflowNode {
    pub index: u32,
    pub tool_name: String,
    pub role: String,
    pub input_shape: Value,
    pub effect_class: String,
    pub verification_shape: String,
    pub dependencies: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalWorkflow {
    pub version: u16,
    pub nodes: Vec<CanonicalWorkflowNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedWorkflowEpisode {
    pub episode_id: String,
    pub reasons: Vec<WorkflowRejectionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowGroupAnalysis {
    pub signature: String,
    pub classification: WorkflowClassification,
    pub eligible: bool,
    pub reasons: Vec<WorkflowRejectionReason>,
    pub occurrence_count: usize,
    pub distinct_turn_count: usize,
    pub distinct_session_count: usize,
    pub explicit_reuse_request: bool,
    pub scope: WorkflowScope,
    pub episode_ids: Vec<String>,
    pub intent_samples: Vec<String>,
    pub canonical: CanonicalWorkflow,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowAnalysisBatch {
    pub groups: Vec<WorkflowGroupAnalysis>,
    pub rejected_episodes: Vec<RejectedWorkflowEpisode>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowAnalysisCatalog {
    pub existing_signatures: BTreeMap<String, ExistingCapabilityKind>,
    pub pending_signatures: BTreeSet<String>,
}

struct GroupEvidence {
    canonical: CanonicalWorkflow,
    episodes: Vec<WorkflowEpisodeEvidence>,
}

pub fn analyze_workflow_evidence(
    evidence: Vec<WorkflowEpisodeEvidence>,
    catalog: &WorkflowAnalysisCatalog,
) -> WorkflowAnalysisBatch {
    let mut grouped: BTreeMap<String, GroupEvidence> = BTreeMap::new();
    let mut rejected_episodes = Vec::new();

    for episode in evidence {
        let reasons = episode_rejection_reasons(&episode);
        if !reasons.is_empty() {
            rejected_episodes.push(RejectedWorkflowEpisode {
                episode_id: episode.episode.id,
                reasons,
            });
            continue;
        }
        match canonicalize_episode(&episode.steps) {
            Ok(canonical) => {
                let signature = canonical_signature(&canonical);
                grouped
                    .entry(signature)
                    .or_insert_with(|| GroupEvidence {
                        canonical,
                        episodes: Vec::new(),
                    })
                    .episodes
                    .push(episode);
            }
            Err(reason) => rejected_episodes.push(RejectedWorkflowEpisode {
                episode_id: episode.episode.id,
                reasons: vec![reason],
            }),
        }
    }

    let mut groups = grouped
        .into_iter()
        .map(|(signature, mut group)| {
            group.episodes.sort_by(|left, right| {
                left.episode
                    .completed_at_unix_ms
                    .cmp(&right.episode.completed_at_unix_ms)
                    .then_with(|| left.episode.id.cmp(&right.episode.id))
            });
            analyze_group(signature, group, catalog)
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| left.signature.cmp(&right.signature));
    rejected_episodes.sort_by(|left, right| left.episode_id.cmp(&right.episode_id));

    WorkflowAnalysisBatch {
        groups,
        rejected_episodes,
    }
}

fn episode_rejection_reasons(evidence: &WorkflowEpisodeEvidence) -> Vec<WorkflowRejectionReason> {
    let mut reasons = BTreeSet::new();
    if evidence.episode.status != "succeeded" {
        reasons.insert(WorkflowRejectionReason::EpisodeNotSuccessful);
    }
    if evidence.episode.has_secret_input || evidence.steps.iter().any(|step| step.secret_detected) {
        reasons.insert(WorkflowRejectionReason::SecretBearingInput);
    }
    if evidence.episode.failure_count > 0
        || evidence.steps.iter().any(|step| step.status != "succeeded")
    {
        reasons.insert(WorkflowRejectionReason::FailedOrInterruptedAttempt);
    }
    if evidence.episode.has_unverified_mutation {
        reasons.insert(WorkflowRejectionReason::UnverifiedMutation);
    }
    if evidence.steps.is_empty() {
        reasons.insert(WorkflowRejectionReason::MissingSteps);
    }
    if evidence.steps.len() > MAX_STEPS_PER_EPISODE {
        reasons.insert(WorkflowRejectionReason::TooManySteps);
    }
    if evidence
        .steps
        .iter()
        .any(|step| step.effect_class == "unknown")
    {
        reasons.insert(WorkflowRejectionReason::UnknownToolAuthority);
    }
    if is_background_noise(evidence) {
        reasons.insert(WorkflowRejectionReason::BackgroundNoise);
    }
    reasons.into_iter().collect()
}

fn analyze_group(
    signature: String,
    group: GroupEvidence,
    catalog: &WorkflowAnalysisCatalog,
) -> WorkflowGroupAnalysis {
    let sessions = group
        .episodes
        .iter()
        .map(|episode| episode.episode.session_id.as_str())
        .collect::<HashSet<_>>();
    let turns = group
        .episodes
        .iter()
        .map(|episode| episode.episode.turn_id.as_str())
        .collect::<HashSet<_>>();
    let explicit_reuse_request = group
        .episodes
        .iter()
        .any(|episode| episode.episode.explicit_reuse_request);
    let mut reasons = BTreeSet::new();
    let mut classification = base_classification(&group.canonical);

    if catalog.pending_signatures.contains(&signature) {
        classification = WorkflowClassification::None;
        reasons.insert(WorkflowRejectionReason::DuplicatePending);
    } else if let Some(existing) = catalog.existing_signatures.get(&signature) {
        match existing {
            ExistingCapabilityKind::Skill | ExistingCapabilityKind::Capspec => {
                classification = WorkflowClassification::Refinement;
            }
            ExistingCapabilityKind::Automation => {
                classification = WorkflowClassification::None;
                reasons.insert(WorkflowRejectionReason::DuplicateExistingAutomation);
            }
            ExistingCapabilityKind::Native => {
                classification = WorkflowClassification::None;
                reasons.insert(WorkflowRejectionReason::DuplicateNativeCapability);
            }
        }
    }

    if !explicit_reuse_request
        && (group.episodes.len() < MIN_AUTOMATIC_EPISODES
            || sessions.len() < MIN_AUTOMATIC_SESSIONS)
    {
        reasons.insert(WorkflowRejectionReason::InsufficientRecurrence);
    }
    if classification == WorkflowClassification::Memory {
        reasons.insert(WorkflowRejectionReason::MemoryOnly);
    } else if classification != WorkflowClassification::None {
        apply_structure_gate(&group.canonical, &mut reasons);
        if reasons.iter().any(|reason| {
            matches!(
                reason,
                WorkflowRejectionReason::OrdinarySingleStep
                    | WorkflowRejectionReason::RepetitiveNoise
                    | WorkflowRejectionReason::UnverifiedMutation
            )
        }) {
            classification = WorkflowClassification::None;
        }
    }

    let reasons = reasons.into_iter().collect::<Vec<_>>();
    let eligible = reasons.is_empty()
        && matches!(
            classification,
            WorkflowClassification::Refinement
                | WorkflowClassification::Skill
                | WorkflowClassification::Capspec
                | WorkflowClassification::Automation
        );
    let episode_ids = group
        .episodes
        .iter()
        .map(|episode| episode.episode.id.clone())
        .collect();
    let mut intent_samples = Vec::new();
    for episode in &group.episodes {
        if intent_samples.len() == 3 {
            break;
        }
        if !intent_samples.contains(&episode.episode.intent_redacted) {
            intent_samples.push(episode.episode.intent_redacted.clone());
        }
    }

    WorkflowGroupAnalysis {
        signature,
        classification,
        eligible,
        reasons,
        occurrence_count: group.episodes.len(),
        distinct_turn_count: turns.len(),
        distinct_session_count: sessions.len(),
        explicit_reuse_request,
        scope: workflow_scope(&group.episodes),
        episode_ids,
        intent_samples,
        canonical: group.canonical,
    }
}

fn apply_structure_gate(
    canonical: &CanonicalWorkflow,
    reasons: &mut BTreeSet<WorkflowRejectionReason>,
) {
    if canonical
        .nodes
        .iter()
        .any(|node| node.effect_class != "read" && node.verification_shape == "unverified")
    {
        reasons.insert(WorkflowRejectionReason::UnverifiedMutation);
    }
    let unique_nodes = canonical
        .nodes
        .iter()
        .map(|node| {
            serde_json::to_string(&(
                &node.tool_name,
                &node.input_shape,
                &node.effect_class,
                &node.verification_shape,
            ))
            .expect("canonical workflow node serializes")
        })
        .collect::<BTreeSet<_>>();
    match canonical.nodes.as_slice() {
        [node] if !high_value_single_step(node) => {
            reasons.insert(WorkflowRejectionReason::OrdinarySingleStep);
        }
        nodes if nodes.len() > 1 && unique_nodes.len() == 1 => {
            reasons.insert(WorkflowRejectionReason::RepetitiveNoise);
        }
        _ => {}
    }
}

fn base_classification(canonical: &CanonicalWorkflow) -> WorkflowClassification {
    if canonical.nodes.iter().all(|node| node.role == "memory") {
        WorkflowClassification::Memory
    } else if canonical.nodes.iter().any(|node| node.role == "automation") {
        WorkflowClassification::Automation
    } else if canonical.nodes.iter().any(|node| {
        matches!(
            node.role.as_str(),
            "research" | "browser" | "delegation" | "human_input"
        )
    }) {
        WorkflowClassification::Skill
    } else {
        WorkflowClassification::Capspec
    }
}

fn workflow_scope(episodes: &[WorkflowEpisodeEvidence]) -> WorkflowScope {
    let projects = episodes
        .iter()
        .filter_map(|episode| episode.episode.project_id.as_deref())
        .collect::<HashSet<_>>();
    if projects.len() == 1
        && episodes
            .iter()
            .all(|episode| episode.episode.project_id.is_some())
    {
        WorkflowScope::Project
    } else {
        let workspaces = episodes
            .iter()
            .filter_map(|episode| episode.episode.workspace_scope.as_deref())
            .collect::<HashSet<_>>();
        if workspaces.len() == 1
            && episodes
                .iter()
                .all(|episode| episode.episode.workspace_scope.is_some())
        {
            WorkflowScope::Workspace
        } else {
            WorkflowScope::Global
        }
    }
}

fn is_background_noise(evidence: &WorkflowEpisodeEvidence) -> bool {
    let has_automation = evidence
        .steps
        .iter()
        .any(|step| tool_role(&step.tool_name.to_ascii_lowercase()) == "automation");
    if has_automation {
        return false;
    }
    let origin = evidence
        .episode
        .origin_channel
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let intent = evidence.episode.intent_redacted.to_ascii_lowercase();
    matches!(
        origin.as_str(),
        "heartbeat" | "autonomy" | "background" | "maintenance"
    ) || ["heartbeat", "autonomous tick", "maintenance tick"]
        .iter()
        .any(|marker| intent.contains(marker))
}

fn high_value_single_step(node: &CanonicalWorkflowNode) -> bool {
    matches!(
        node.role.as_str(),
        "automation" | "integration" | "remote_health" | "remote_command" | "document"
    ) && node.verification_shape != "unverified"
}
