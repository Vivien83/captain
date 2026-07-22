//! Exact conversion of an immutable workflow-learning Automation draft into
//! the native scheduler contract.

use captain_types::agent::AgentId;
use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::workflow_learning_proposer::AutomationScheduleDraft;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StagedAutomationArtifact {
    pub schema_version: u16,
    pub name: String,
    pub enabled: bool,
    pub schedule: AutomationScheduleDraft,
    pub instruction: String,
}

pub fn build_disabled_automation_job(
    bytes: &[u8],
    expected_name: &str,
    id: CronJobId,
    agent_id: AgentId,
    created_at: DateTime<Utc>,
) -> Result<CronJob, String> {
    let staged: StagedAutomationArtifact = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid staged automation envelope: {error}"))?;
    if staged.schema_version != 1 || staged.name != expected_name || staged.enabled {
        return Err(
            "staged automation envelope is not the exact inactive schema-v1 draft".to_string(),
        );
    }
    let job = CronJob {
        id,
        agent_id,
        name: staged.name,
        enabled: false,
        schedule: match staged.schedule {
            AutomationScheduleDraft::Every { every_secs } => CronSchedule::Every { every_secs },
            AutomationScheduleDraft::Cron {
                expression,
                timezone,
            } => CronSchedule::Cron {
                expr: expression,
                tz: timezone,
            },
        },
        action: CronAction::AgentTurn {
            message: staged.instruction,
            model_override: None,
            timeout_secs: None,
        },
        delivery: CronDelivery::LastChannel,
        created_at,
        last_run: None,
        next_run: None,
    };
    job.validate(0)?;
    Ok(job)
}
