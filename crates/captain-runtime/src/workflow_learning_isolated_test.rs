//! Model-independent isolated execution tests for staged learning artifacts.
//!
//! Every parser and registry is rooted in a fresh temporary directory. The
//! active Captain skill registry, CapSpec registry, and scheduler are never
//! passed into this module, making an isolated test incapable of activation.

use std::path::Path;

use captain_capspec::{CapabilityRegistry, CapabilityScope};
use captain_memory::workflow_learning_control::{WorkflowArtifactKind, WorkflowProposalRecord};
use captain_skills::registry::SkillRegistry;
use captain_types::workflow_learning::{
    ProposalCardKind, ProposalIsolatedTestCheck, WorkflowIsolatedTestReport,
};
use chrono::Utc;

use crate::workflow_learning_automation::build_disabled_automation_job;
use crate::workflow_learning_proposer::{RefinementTargetKind, WorkflowDraftArtifact};
use crate::workflow_learning_staging::WorkflowStagingRoot;

const REPORT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowIsolatedTestError {
    #[error("isolated test cannot identify the published proposal: {0}")]
    InvalidProposal(String),
}

#[derive(Debug, Clone)]
pub struct WorkflowIsolatedTestRunner {
    staging: WorkflowStagingRoot,
}

impl WorkflowIsolatedTestRunner {
    pub fn new(staging: WorkflowStagingRoot) -> Self {
        Self { staging }
    }

    pub fn run(
        &self,
        proposal: &WorkflowProposalRecord,
        completed_at_unix_ms: i64,
    ) -> Result<WorkflowIsolatedTestReport, WorkflowIsolatedTestError> {
        let revision_sha256 = required(&proposal.revision_sha256, "revision_sha256")?;
        let artifact_sha256 = required(&proposal.artifact_sha256, "artifact_sha256")?;
        let staging_job_id = required(&proposal.staging_job_id, "staging_job_id")?;
        let name = required(&proposal.name, "name")?;
        let kind = proposal.kind.ok_or_else(|| {
            WorkflowIsolatedTestError::InvalidProposal("kind is missing".to_string())
        })?;
        let mut report = WorkflowIsolatedTestReport {
            schema_version: REPORT_SCHEMA_VERSION,
            proposal_id: proposal.id.clone(),
            revision_sha256: revision_sha256.clone(),
            artifact_sha256: artifact_sha256.clone(),
            kind: map_kind(kind),
            name: name.clone(),
            passed: false,
            checks: Vec::new(),
            completed_at_unix_ms,
        };

        let staged = match self.staging.load_exact(&staging_job_id, &revision_sha256) {
            Ok(staged) => staged,
            Err(error) => {
                report.checks.push(failed(
                    "immutable_staging_identity",
                    format!("staged revision could not be loaded: {error}"),
                ));
                return Ok(report);
            }
        };
        let identity_matches = staged.manifest.artifact_sha256 == artifact_sha256
            && staged.manifest.name == name
            && staged.manifest.kind == staged.manifest.draft.kind;
        report.checks.push(check(
            "immutable_staging_identity",
            identity_matches,
            if identity_matches {
                "SQLite metadata and immutable staged bytes match"
            } else {
                "SQLite metadata differs from the immutable staged draft"
            },
        ));
        if !identity_matches {
            return Ok(report);
        }

        let isolated_root = match tempfile::tempdir() {
            Ok(root) => root,
            Err(error) => {
                report.checks.push(failed(
                    "private_test_root",
                    format!("private test root could not be created: {error}"),
                ));
                return Ok(report);
            }
        };
        report.checks.push(check(
            "private_test_root",
            true,
            "test registries use a fresh temporary root",
        ));

        match test_artifact(isolated_root.path(), &staged) {
            Ok(mut checks) => report.checks.append(&mut checks),
            Err(error) => report
                .checks
                .push(failed("native_runtime_load", error.to_string())),
        }
        report.checks.push(check(
            "active_state_untouched",
            true,
            "no active Captain registry or scheduler handle was available to the test",
        ));
        report.passed = report.checks.iter().all(|check| check.passed);
        Ok(report)
    }
}

fn test_artifact(
    root: &Path,
    staged: &crate::workflow_learning_staging::LoadedStagedWorkflowDraft,
) -> Result<Vec<ProposalIsolatedTestCheck>, Box<dyn std::error::Error + Send + Sync>> {
    match &staged.manifest.draft.artifact {
        WorkflowDraftArtifact::SkillMarkdown { .. } => {
            test_skill(root, &staged.manifest.name, &staged.artifact_bytes)
        }
        WorkflowDraftArtifact::CapspecToml { .. } => {
            test_capspec(root, &staged.manifest.name, &staged.artifact_bytes)
        }
        WorkflowDraftArtifact::Automation { .. } => {
            test_automation(&staged.manifest.name, &staged.artifact_bytes)
        }
        WorkflowDraftArtifact::Refinement {
            target_kind,
            target_name,
            ..
        } => match target_kind {
            RefinementTargetKind::Skill => test_skill(root, target_name, &staged.artifact_bytes),
            RefinementTargetKind::Capspec => {
                test_capspec(root, target_name, &staged.artifact_bytes)
            }
        },
    }
}

fn test_skill(
    root: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<Vec<ProposalIsolatedTestCheck>, Box<dyn std::error::Error + Send + Sync>> {
    let skills = root.join("skills");
    let learned = skills.join("learned");
    captain_types::durable_fs::create_dir_all(&learned)?;
    let path = learned.join(format!("{name}.md"));
    captain_types::durable_fs::atomic_write(&path, bytes)?;
    let mut registry = SkillRegistry::new(skills);
    registry.load_all()?;
    let loaded = registry
        .get(name)
        .ok_or_else(|| invalid_test(format!("native skill registry did not load {name}")))?;
    let exact_owner = loaded.path == path;
    Ok(vec![check(
        "native_skill_registry",
        exact_owner,
        if exact_owner {
            "native skill parser loaded the exact private artifact"
        } else {
            "another source owns the private skill name"
        },
    )])
}

fn test_capspec(
    root: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<Vec<ProposalIsolatedTestCheck>, Box<dyn std::error::Error + Send + Sync>> {
    let capabilities = root.join("capabilities");
    captain_types::durable_fs::create_dir_all(&capabilities)?;
    let path = capabilities.join(format!("{name}.captain"));
    captain_types::durable_fs::atomic_write(&path, bytes)?;
    let registry = CapabilityRegistry::open(&capabilities, &root.join("capspec.sqlite"))?;
    let view = registry.capability(&CapabilityScope::Global, name)?;
    let expected_hash = blake3::hash(bytes).to_hex().to_string();
    let exact_hash = view.active_hash.as_deref() == Some(expected_hash.as_str())
        || view.pending_hash.as_deref() == Some(expected_hash.as_str());
    let exact_path = view.source_path.canonicalize()? == path.canonicalize()?;
    Ok(vec![check(
        "native_capspec_registry",
        exact_hash && exact_path,
        if exact_hash && exact_path {
            "native CapSpec compiler loaded the exact private artifact"
        } else {
            "private CapSpec registry identity differs from the staged artifact"
        },
    )])
}

fn test_automation(
    expected_name: &str,
    bytes: &[u8],
) -> Result<Vec<ProposalIsolatedTestCheck>, Box<dyn std::error::Error + Send + Sync>> {
    build_disabled_automation_job(
        bytes,
        expected_name,
        captain_types::scheduler::CronJobId::new(),
        captain_types::agent::AgentId::from_string("captain"),
        Utc::now(),
    )
    .map_err(invalid_test)?;
    Ok(vec![check(
        "native_scheduler_contract",
        true,
        "native scheduler accepted the disabled job without registering it",
    )])
}

fn map_kind(kind: WorkflowArtifactKind) -> ProposalCardKind {
    match kind {
        WorkflowArtifactKind::Skill => ProposalCardKind::Skill,
        WorkflowArtifactKind::Capspec => ProposalCardKind::Capspec,
        WorkflowArtifactKind::Automation => ProposalCardKind::Automation,
        WorkflowArtifactKind::Refinement => ProposalCardKind::Refinement,
    }
}

fn required(value: &Option<String>, field: &str) -> Result<String, WorkflowIsolatedTestError> {
    value
        .as_ref()
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| WorkflowIsolatedTestError::InvalidProposal(format!("{field} is missing")))
}

fn check(
    code: impl Into<String>,
    passed: bool,
    detail: impl Into<String>,
) -> ProposalIsolatedTestCheck {
    ProposalIsolatedTestCheck {
        code: code.into(),
        passed,
        detail: detail.into(),
    }
}

fn failed(code: impl Into<String>, detail: impl Into<String>) -> ProposalIsolatedTestCheck {
    check(code, false, detail)
}

fn invalid_test(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
