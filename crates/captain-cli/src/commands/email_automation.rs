use std::io::IsTerminal;
use std::path::Path;
use std::str::FromStr;

use captain_memory::gmail_automation::{
    gmail_automation_rule_id, GmailAutomationAction, GmailAutomationCondition,
    GmailAutomationOutboxStatus, GmailAutomationRuleRecord, NewGmailAutomationRule,
};
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentId;
use captain_types::email::GmailAccountAlias;

use super::email_automation_render::{
    print_deliveries, print_delivery, print_rule, print_rule_detail, print_rules,
    GmailDeliveryDetailView, GmailDeliveryView,
};
use crate::{prompt_input, GmailDeliveryCommands, GmailRuleCommands};

pub(super) fn manage_rules(
    config: Option<&Path>,
    command: GmailRuleCommands,
) -> Result<(), String> {
    let memory = open_memory(config)?;
    match command {
        GmailRuleCommands::Add {
            id,
            account,
            name,
            from_contains,
            recipient_contains,
            subject_contains,
            all_label_ids,
            any_label_ids,
            agent,
            instruction,
            include_body,
            max_body_bytes,
            max_delivery_attempts,
            max_fires_per_hour,
            disabled,
            json,
        } => {
            let account_alias = resolve_account(&memory, account.as_deref())?;
            let target_agent_id = resolve_agent(&memory, &agent)?;
            let rule_id = id.unwrap_or_else(|| gmail_automation_rule_id(&account_alias, &name));
            let rule = memory
                .gmail_automation()
                .create_rule(NewGmailAutomationRule {
                    id: rule_id,
                    account_alias,
                    name,
                    condition: GmailAutomationCondition {
                        from_contains,
                        recipient_contains,
                        subject_contains,
                        all_label_ids,
                        any_label_ids,
                    },
                    action: GmailAutomationAction {
                        target_agent_id,
                        instruction,
                        include_body,
                        max_body_bytes,
                        max_delivery_attempts,
                    },
                    enabled: !disabled,
                    max_fires_per_hour,
                    created_at_unix_ms: now_unix_ms(),
                })
                .map_err(|error| error.to_string())?;
            print_rule(&rule, json, "Created")
        }
        GmailRuleCommands::List { account, json } => {
            let account = account
                .map(|value| GmailAccountAlias::parse(&value).map_err(|error| error.to_string()))
                .transpose()?;
            let mut rules = memory
                .gmail_automation()
                .list_rules(1_000)
                .map_err(|error| error.to_string())?;
            if let Some(account) = account {
                rules.retain(|rule| rule.account_alias == account);
            }
            print_rules(&rules, json)
        }
        GmailRuleCommands::Show { id, json } => {
            let rule = require_rule(&memory, &id)?;
            print_rule_detail(&rule, json)
        }
        GmailRuleCommands::Enable { id, json } => set_rule_enabled(&memory, &id, true, json),
        GmailRuleCommands::Disable { id, json } => set_rule_enabled(&memory, &id, false, json),
        GmailRuleCommands::Remove { id, yes, json } => {
            let rule = require_rule(&memory, &id)?;
            confirm(
                yes,
                &format!(
                    "Delete unused Gmail rule '{}' at version {}? [y/N]: ",
                    rule.id, rule.state_version
                ),
                "Rule deletion requires --yes in non-interactive mode",
            )?;
            let deleted = memory
                .gmail_automation()
                .delete_rule(&rule.id, rule.state_version)
                .map_err(|error| error.to_string())?;
            print_rule(&deleted, json, "Removed")
        }
    }
}

pub(super) fn manage_deliveries(
    config: Option<&Path>,
    command: GmailDeliveryCommands,
) -> Result<(), String> {
    let memory = open_memory(config)?;
    match command {
        GmailDeliveryCommands::List {
            status,
            limit,
            json,
        } => {
            if !(1..=1_000).contains(&limit) {
                return Err("Delivery list limit must be between 1 and 1000".to_string());
            }
            let records = memory
                .gmail_automation()
                .list_outbox(status.map(|status| status.status()), limit)
                .map_err(|error| error.to_string())?;
            let views = records
                .iter()
                .map(GmailDeliveryView::from_record)
                .collect::<Vec<_>>();
            print_deliveries(&views, json)
        }
        GmailDeliveryCommands::Show { id, json } => {
            let record = require_delivery(&memory, &id)?;
            print_delivery(&GmailDeliveryDetailView::from_record(&record), json, None)
        }
        GmailDeliveryCommands::Requeue { id, yes, json } => {
            let current = require_delivery(&memory, &id)?;
            if !matches!(
                current.status,
                GmailAutomationOutboxStatus::Dead | GmailAutomationOutboxStatus::Uncertain
            ) {
                return Err(
                    "Only a reviewed dead or uncertain delivery can be requeued".to_string()
                );
            }
            let warning = if current.status == GmailAutomationOutboxStatus::Uncertain {
                "This delivery may already have executed. Inspect its persisted session before requeueing. Requeue anyway? [y/N]: "
            } else {
                "Retry this reviewed dead delivery from attempt zero? [y/N]: "
            };
            confirm(
                yes,
                warning,
                "Delivery requeue requires --yes in non-interactive mode",
            )?;
            let updated = memory
                .gmail_automation()
                .requeue_reviewed(&current.id, "captain-cli", current.status, now_unix_ms())
                .map_err(|error| error.to_string())?;
            print_delivery(
                &GmailDeliveryDetailView::from_record(&updated),
                json,
                Some("Requeued"),
            )
        }
    }
}

fn set_rule_enabled(
    memory: &MemorySubstrate,
    id: &str,
    enabled: bool,
    json: bool,
) -> Result<(), String> {
    let current = require_rule(memory, id)?;
    let updated = memory
        .gmail_automation()
        .set_rule_enabled(&current.id, current.state_version, enabled, now_unix_ms())
        .map_err(|error| error.to_string())?;
    let action = if updated.state_version == current.state_version {
        if enabled {
            "Already enabled"
        } else {
            "Already disabled"
        }
    } else if enabled {
        "Enabled"
    } else {
        "Disabled"
    };
    print_rule(&updated, json, action)
}

fn require_rule(memory: &MemorySubstrate, id: &str) -> Result<GmailAutomationRuleRecord, String> {
    memory
        .gmail_automation()
        .get_rule(id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Gmail automation rule '{id}' was not found"))
}

fn require_delivery(
    memory: &MemorySubstrate,
    id: &str,
) -> Result<captain_memory::gmail_automation::GmailAutomationOutboxRecord, String> {
    memory
        .gmail_automation()
        .get_outbox(id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Gmail automation delivery '{id}' was not found"))
}

fn resolve_account(
    memory: &MemorySubstrate,
    requested: Option<&str>,
) -> Result<GmailAccountAlias, String> {
    let records = memory
        .gmail_accounts()
        .list()
        .map_err(|error| error.to_string())?;
    if let Some(requested) = requested {
        let alias = GmailAccountAlias::parse(requested).map_err(|error| error.to_string())?;
        return records
            .iter()
            .find(|record| record.summary.alias == alias)
            .map(|record| record.summary.alias.clone())
            .ok_or_else(|| format!("Gmail account '{alias}' was not found"));
    }
    records
        .iter()
        .find(|record| record.summary.is_default)
        .map(|record| record.summary.alias.clone())
        .ok_or_else(|| "No default Gmail account is connected".to_string())
}

fn resolve_agent(memory: &MemorySubstrate, reference: &str) -> Result<AgentId, String> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err("Target agent cannot be empty".to_string());
    }
    let agents = memory
        .load_all_agents()
        .map_err(|error| error.to_string())?;
    if let Ok(id) = AgentId::from_str(reference) {
        return agents
            .iter()
            .any(|entry| entry.id == id)
            .then_some(id)
            .ok_or_else(|| format!("Target agent '{reference}' is not persisted"));
    }

    let by_name = agents
        .iter()
        .filter(|entry| entry.manifest.name.eq_ignore_ascii_case(reference))
        .collect::<Vec<_>>();
    if by_name.len() == 1 {
        return Ok(by_name[0].id);
    }
    if by_name.len() > 1 {
        return Err(format!(
            "Agent name '{reference}' is ambiguous; use its UUID"
        ));
    }
    if reference.len() < 8 {
        return Err(format!(
            "Target agent '{reference}' was not found; UUID prefixes require at least 8 characters"
        ));
    }
    let prefix = reference.to_ascii_lowercase();
    let by_prefix = agents
        .iter()
        .filter(|entry| entry.id.to_string().starts_with(&prefix))
        .collect::<Vec<_>>();
    match by_prefix.as_slice() {
        [entry] => Ok(entry.id),
        [] => Err(format!("Target agent '{reference}' was not found")),
        _ => Err(format!(
            "Agent ID prefix '{reference}' is ambiguous; use more characters"
        )),
    }
}

fn confirm(yes: bool, prompt: &str, non_interactive_error: &str) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(non_interactive_error.to_string());
    }
    let answer = prompt_input(prompt);
    if matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "o" | "oui"
    ) {
        Ok(())
    } else {
        Err("Operation cancelled; durable Gmail automation state was not changed".to_string())
    }
}

fn open_memory(config_path: Option<&Path>) -> Result<MemorySubstrate, String> {
    let config = captain_kernel::config::load_config(config_path);
    std::fs::create_dir_all(&config.data_dir)
        .map_err(|error| format!("Could not create Captain data directory: {error}"))?;
    let db_path = config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| config.data_dir.join("captain.db"));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Gmail database directory: {error}"))?;
    }
    MemorySubstrate::open(&db_path, config.memory.decay_rate)
        .map_err(|error| format!("Could not open Captain memory database: {error}"))
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis().max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_id_derivation_is_stable_bounded_and_handles_non_ascii_names() {
        let account = GmailAccountAlias::parse("work").unwrap();
        assert_eq!(
            gmail_automation_rule_id(&account, "Invoice Review"),
            "work-invoice-review"
        );
        assert_eq!(
            gmail_automation_rule_id(&account, "Factures été"),
            "work-factures-t"
        );
        let first = gmail_automation_rule_id(&account, "合同");
        assert_eq!(first, gmail_automation_rule_id(&account, "合同"));
        assert!(first.starts_with("gmail-"));
        assert!(gmail_automation_rule_id(&account, &"a".repeat(200)).len() <= 96);
    }

    #[test]
    fn confirmation_is_non_interactive_safe_when_yes_is_explicit() {
        assert!(confirm(true, "ignored", "ignored").is_ok());
    }
}
