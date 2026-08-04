use captain_memory::gmail_automation::{
    gmail_delivery_session_id, GmailAutomationDeliveryPayload, GmailAutomationOutboxRecord,
    GmailAutomationRuleRecord,
};
use serde::Serialize;

use crate::ui;

#[derive(Debug, Clone, Serialize)]
pub(super) struct GmailDeliveryView {
    pub(super) id: String,
    pub(super) event_id: String,
    pub(super) target_agent_id: String,
    pub(super) status: String,
    pub(super) attempt_count: u32,
    pub(super) max_attempts: u32,
    pub(super) run_after_unix_ms: i64,
    pub(super) lease_expires_at_unix_ms: Option<i64>,
    pub(super) last_error: Option<String>,
    pub(super) delivered_at_unix_ms: Option<i64>,
    pub(super) created_at_unix_ms: i64,
    pub(super) updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct GmailDeliveryDetailView {
    #[serde(flatten)]
    pub(super) delivery: GmailDeliveryView,
    pub(super) rule_id: Option<String>,
    pub(super) rule_version: Option<u64>,
    pub(super) rule_name: Option<String>,
    pub(super) account_alias: Option<String>,
    pub(super) message_id: Option<String>,
    pub(super) history_id: Option<String>,
    pub(super) instruction: Option<String>,
    pub(super) include_body: Option<bool>,
    pub(super) delivery_session_id: String,
    pub(super) untrusted_message_metadata: Option<serde_json::Value>,
    pub(super) payload_error: Option<&'static str>,
}

impl GmailDeliveryView {
    pub(super) fn from_record(record: &GmailAutomationOutboxRecord) -> Self {
        Self {
            id: record.id.clone(),
            event_id: record.event_id.clone(),
            target_agent_id: record.target_agent_id.to_string(),
            status: record.status.as_str().to_string(),
            attempt_count: record.attempt_count,
            max_attempts: record.max_attempts,
            run_after_unix_ms: record.run_after_unix_ms,
            lease_expires_at_unix_ms: record.lease_expires_at_unix_ms,
            last_error: record.last_error.clone(),
            delivered_at_unix_ms: record.delivered_at_unix_ms,
            created_at_unix_ms: record.created_at_unix_ms,
            updated_at_unix_ms: record.updated_at_unix_ms,
        }
    }
}

impl GmailDeliveryDetailView {
    pub(super) fn from_record(record: &GmailAutomationOutboxRecord) -> Self {
        let payload = serde_json::from_str::<GmailAutomationDeliveryPayload>(&record.payload_json);
        match payload {
            Ok(payload) => Self {
                delivery: GmailDeliveryView::from_record(record),
                rule_id: Some(payload.rule_id),
                rule_version: Some(payload.rule_version),
                rule_name: Some(payload.rule_name),
                account_alias: Some(payload.account_alias.to_string()),
                message_id: Some(payload.message_id),
                history_id: Some(payload.history_id),
                instruction: Some(payload.instruction),
                include_body: Some(payload.include_body),
                delivery_session_id: gmail_delivery_session_id(&record.id).to_string(),
                untrusted_message_metadata: Some(payload.metadata),
                payload_error: None,
            },
            Err(_) => Self {
                delivery: GmailDeliveryView::from_record(record),
                rule_id: None,
                rule_version: None,
                rule_name: None,
                account_alias: None,
                message_id: None,
                history_id: None,
                instruction: None,
                include_body: None,
                delivery_session_id: gmail_delivery_session_id(&record.id).to_string(),
                untrusted_message_metadata: None,
                payload_error: Some("payload is corrupt or incompatible"),
            },
        }
    }
}

pub(super) fn print_rule(
    rule: &GmailAutomationRuleRecord,
    json: bool,
    action: &str,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(rule).map_err(|error| error.to_string())?
        );
    } else {
        ui::success(&format!(
            "{action} Gmail rule '{}' ({}, version {}, {}).",
            rule.id,
            rule.account_alias,
            rule.state_version,
            if rule.enabled { "enabled" } else { "disabled" }
        ));
    }
    Ok(())
}

pub(super) fn print_rules(rules: &[GmailAutomationRuleRecord], json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(rules).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if rules.is_empty() {
        println!("No Gmail automation rules configured.");
        return Ok(());
    }
    println!(
        "{:<28} {:<16} {:<9} {:<8} {:<36} NAME",
        "RULE", "ACCOUNT", "STATE", "VERSION", "TARGET AGENT"
    );
    for rule in rules {
        println!(
            "{:<28} {:<16} {:<9} {:<8} {:<36} {}",
            rule.id,
            rule.account_alias,
            if rule.enabled { "enabled" } else { "disabled" },
            rule.state_version,
            rule.action.target_agent_id,
            safe_terminal_text(&rule.name)
        );
    }
    Ok(())
}

pub(super) fn print_rule_detail(
    rule: &GmailAutomationRuleRecord,
    json: bool,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(rule).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!("Rule: {}", rule.id);
    println!("Name: {}", safe_terminal_text(&rule.name));
    println!("Account: {}", rule.account_alias);
    println!(
        "State: {}",
        if rule.enabled { "enabled" } else { "disabled" }
    );
    println!("Version: {}", rule.state_version);
    println!("Target agent: {}", rule.action.target_agent_id);
    println!("Maximum fires/hour: {}", rule.max_fires_per_hour);
    println!("Include body: {}", rule.action.include_body);
    println!("Maximum body bytes: {}", rule.action.max_body_bytes);
    println!(
        "Maximum delivery attempts: {}",
        rule.action.max_delivery_attempts
    );
    println!(
        "Instruction: {}",
        safe_terminal_text(&rule.action.instruction)
    );
    println!(
        "Conditions: {}",
        serde_json::to_string(&rule.condition).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub(super) fn print_deliveries(deliveries: &[GmailDeliveryView], json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(deliveries).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if deliveries.is_empty() {
        println!("No Gmail automation deliveries found.");
        return Ok(());
    }
    println!(
        "{:<38} {:<12} {:<9} {:<36} UPDATED",
        "DELIVERY", "STATE", "ATTEMPTS", "TARGET AGENT"
    );
    for delivery in deliveries {
        println!(
            "{:<38} {:<12} {:>2}/{:<6} {:<36} {}",
            delivery.id,
            delivery.status,
            delivery.attempt_count,
            delivery.max_attempts,
            delivery.target_agent_id,
            delivery.updated_at_unix_ms
        );
    }
    Ok(())
}

pub(super) fn print_delivery(
    delivery: &GmailDeliveryDetailView,
    json: bool,
    action: Option<&str>,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(delivery).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if let Some(action) = action {
        ui::success(&format!(
            "{action} Gmail delivery '{}' ({}).",
            delivery.delivery.id, delivery.delivery.status
        ));
        return Ok(());
    }
    println!("Delivery: {}", delivery.delivery.id);
    println!("State: {}", delivery.delivery.status);
    println!(
        "Attempts: {}/{}",
        delivery.delivery.attempt_count, delivery.delivery.max_attempts
    );
    println!("Target agent: {}", delivery.delivery.target_agent_id);
    if let Some(rule) = delivery.rule_name.as_deref() {
        println!(
            "Rule: {} ({})",
            safe_terminal_text(rule),
            delivery.rule_id.as_deref().unwrap_or("unknown")
        );
    }
    if let Some(message_id) = delivery.message_id.as_deref() {
        println!("Gmail message: {message_id}");
    }
    println!(
        "Delivery session ID (if created): {}",
        delivery.delivery_session_id
    );
    if let Some(error) = delivery.delivery.last_error.as_deref() {
        println!("Last error: {}", safe_terminal_text(error));
    }
    if delivery.delivery.status == "uncertain" {
        println!(
            "Warning: the agent turn may already have executed. Inspect its session before requeueing."
        );
    }
    if delivery.payload_error.is_some() {
        println!("Payload: corrupt or incompatible");
    }
    Ok(())
}

fn safe_terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() && character != '\t' {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::gmail_automation::GmailAutomationOutboxStatus;
    use captain_types::agent::AgentId;

    #[test]
    fn delivery_list_view_never_exposes_payload_or_email_metadata() {
        let record = GmailAutomationOutboxRecord {
            id: "outbox".to_string(),
            idempotency_key: "delivery:outbox".to_string(),
            event_id: "event".to_string(),
            target_agent_id: AgentId::from_string("captain"),
            payload_json: r#"{"metadata":{"subject":"secret subject"}}"#.to_string(),
            status: GmailAutomationOutboxStatus::Dead,
            attempt_count: 3,
            max_attempts: 3,
            run_after_unix_ms: 1,
            lease_owner: None,
            lease_expires_at_unix_ms: None,
            delivery_result_json: None,
            last_error: Some("failed".to_string()),
            delivered_at_unix_ms: None,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 2,
        };
        let encoded = serde_json::to_string(&GmailDeliveryView::from_record(&record)).unwrap();
        assert!(!encoded.contains("secret subject"));
        assert!(!encoded.contains("payload_json"));
        assert!(encoded.contains("dead"));
    }

    #[test]
    fn terminal_error_text_strips_escape_controls() {
        assert_eq!(safe_terminal_text("bad\u{1b}[31m\nline"), "bad [31m line");
    }
}
