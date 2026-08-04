use captain_memory::gmail_accounts::GmailAccountStore;
use captain_memory::gmail_automation::{
    gmail_automation_rule_id, gmail_delivery_session_id, GmailAutomationAction,
    GmailAutomationCondition, GmailAutomationDeliveryPayload, GmailAutomationOutboxRecord,
    GmailAutomationOutboxStatus, GmailAutomationRuleRecord, GmailAutomationRuleUpdate,
    GmailAutomationStore, NewGmailAutomationRule,
};
use captain_types::agent::AgentId;
use captain_types::email::GmailAccountAlias;
use captain_types::email_automation::{
    GmailAutomationConditionSpec, GmailAutomationDeliveryQuery,
    GmailAutomationDeliveryRequeueRequest, GmailAutomationDeliveryState,
    GmailAutomationDeliveryView, GmailAutomationRuleActionView, GmailAutomationRuleQuery,
    GmailAutomationRuleRemoveRequest, GmailAutomationRuleSaveRequest,
    GmailAutomationRuleStateRequest, GmailAutomationRuleView,
};

use super::CaptainKernel;

#[derive(Clone)]
struct GmailAutomationRuntimeService {
    accounts: GmailAccountStore,
    automation: GmailAutomationStore,
}

impl GmailAutomationRuntimeService {
    fn new(accounts: GmailAccountStore, automation: GmailAutomationStore) -> Self {
        Self {
            accounts,
            automation,
        }
    }

    fn rules(
        &self,
        request: GmailAutomationRuleQuery,
        agent_name: impl Fn(AgentId) -> Option<String>,
    ) -> Result<Vec<GmailAutomationRuleView>, String> {
        if !(1..=1_000).contains(&request.limit) {
            return Err("Gmail automation rule limit must be between 1 and 1000".to_string());
        }
        let records = if let Some(rule_id) = request.rule_id.as_deref() {
            vec![self
                .automation
                .get_rule(rule_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Gmail automation rule '{rule_id}' was not found"))?]
        } else {
            self.automation
                .list_rules(usize::from(request.limit))
                .map_err(|error| error.to_string())?
        };
        records
            .into_iter()
            .filter(|record| {
                request
                    .account_alias
                    .as_ref()
                    .is_none_or(|alias| &record.account_alias == alias)
            })
            .map(|record| Ok(rule_view(record, &agent_name)))
            .collect()
    }

    fn save_rule(
        &self,
        request: GmailAutomationRuleSaveRequest,
        resolve_agent: impl Fn(&str) -> Result<AgentId, String>,
        agent_name: impl Fn(AgentId) -> Option<String>,
    ) -> Result<GmailAutomationRuleView, String> {
        if !request.confirm_automation {
            return Err(
                "Gmail automation mutation refused: confirm_automation=true requires an explicit current user request"
                    .to_string(),
            );
        }
        let target_agent_id = resolve_agent(request.target_agent.trim())?;
        let condition = memory_condition(request.condition);
        let action = GmailAutomationAction {
            target_agent_id,
            instruction: request.instruction,
            include_body: request.include_body,
            max_body_bytes: request.max_body_bytes,
            max_delivery_attempts: request.max_delivery_attempts,
        };
        let now = now_unix_ms();
        let record = if let Some(expected_version) = request.expected_version {
            let rule_id = request.id.as_deref().ok_or_else(|| {
                "Updating a Gmail automation rule requires its exact id".to_string()
            })?;
            let current = self
                .automation
                .get_rule(rule_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Gmail automation rule '{rule_id}' was not found"))?;
            if request
                .account_alias
                .as_ref()
                .is_some_and(|alias| alias != &current.account_alias)
            {
                return Err(
                    "A Gmail automation rule cannot move to another account; create a new rule"
                        .to_string(),
                );
            }
            self.automation
                .update_rule(
                    rule_id,
                    GmailAutomationRuleUpdate {
                        expected_version,
                        name: request.name,
                        condition,
                        action,
                        enabled: request.enabled,
                        max_fires_per_hour: request.max_fires_per_hour,
                        updated_at_unix_ms: now,
                    },
                )
                .map_err(|error| error.to_string())?
        } else {
            let account_alias = self.resolve_account(request.account_alias.as_ref())?;
            let id = request
                .id
                .unwrap_or_else(|| gmail_automation_rule_id(&account_alias, &request.name));
            self.automation
                .create_rule(NewGmailAutomationRule {
                    id,
                    account_alias,
                    name: request.name,
                    condition,
                    action,
                    enabled: request.enabled,
                    max_fires_per_hour: request.max_fires_per_hour,
                    created_at_unix_ms: now,
                })
                .map_err(|error| error.to_string())?
        };
        Ok(rule_view(record, &agent_name))
    }

    fn set_rule_enabled(
        &self,
        request: GmailAutomationRuleStateRequest,
        agent_name: impl Fn(AgentId) -> Option<String>,
    ) -> Result<GmailAutomationRuleView, String> {
        if !request.confirm_change {
            return Err(
                "Gmail automation state change refused: confirm_change=true requires an explicit current user request"
                    .to_string(),
            );
        }
        let current = self
            .automation
            .get_rule(&request.rule_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Gmail automation rule '{}' was not found", request.rule_id))?;
        if current.state_version != request.expected_version {
            return Err(format!(
                "Gmail automation rule version changed (expected {}, found {})",
                request.expected_version, current.state_version
            ));
        }
        let record = self
            .automation
            .set_rule_enabled(
                &current.id,
                current.state_version,
                request.enabled,
                now_unix_ms(),
            )
            .map_err(|error| error.to_string())?;
        Ok(rule_view(record, &agent_name))
    }

    fn remove_rule(
        &self,
        request: GmailAutomationRuleRemoveRequest,
        agent_name: impl Fn(AgentId) -> Option<String>,
    ) -> Result<GmailAutomationRuleView, String> {
        if !request.confirm_delete_unused {
            return Err(
                "Gmail automation deletion refused: confirm_delete_unused=true is required"
                    .to_string(),
            );
        }
        let record = self
            .automation
            .delete_rule(&request.rule_id, request.expected_version)
            .map_err(|error| error.to_string())?;
        Ok(rule_view(record, &agent_name))
    }

    fn deliveries(
        &self,
        request: GmailAutomationDeliveryQuery,
        agent_name: impl Fn(AgentId) -> Option<String>,
    ) -> Result<Vec<GmailAutomationDeliveryView>, String> {
        if !(1..=1_000).contains(&request.limit) {
            return Err("Gmail automation delivery limit must be between 1 and 1000".to_string());
        }
        if let Some(delivery_id) = request.delivery_id.as_deref() {
            let record = self
                .automation
                .get_outbox(delivery_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    format!("Gmail automation delivery '{delivery_id}' was not found")
                })?;
            if request
                .status
                .is_some_and(|status| memory_status(status) != record.status)
            {
                return Ok(Vec::new());
            }
            return Ok(vec![delivery_view(record, true, &agent_name)]);
        }
        self.automation
            .list_outbox(
                request.status.map(memory_status),
                usize::from(request.limit),
            )
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|record| Ok(delivery_view(record, false, &agent_name)))
            .collect()
    }

    fn requeue_delivery(
        &self,
        request: GmailAutomationDeliveryRequeueRequest,
        agent_name: impl Fn(AgentId) -> Option<String>,
    ) -> Result<GmailAutomationDeliveryView, String> {
        if !request.confirm_duplicate_risk {
            return Err(
                "Gmail automation requeue refused: confirm_duplicate_risk=true is required because an uncertain delivery may already have executed"
                    .to_string(),
            );
        }
        if !matches!(
            request.expected_status,
            GmailAutomationDeliveryState::Dead | GmailAutomationDeliveryState::Uncertain
        ) {
            return Err("Only a reviewed dead or uncertain delivery can be requeued".to_string());
        }
        let record = self
            .automation
            .requeue_reviewed(
                &request.delivery_id,
                "captain-agent-tool",
                memory_status(request.expected_status),
                now_unix_ms(),
            )
            .map_err(|error| error.to_string())?;
        Ok(delivery_view(record, true, &agent_name))
    }

    fn resolve_account(
        &self,
        requested: Option<&GmailAccountAlias>,
    ) -> Result<GmailAccountAlias, String> {
        let records = self.accounts.list().map_err(|error| error.to_string())?;
        if let Some(requested) = requested {
            return records
                .iter()
                .find(|record| &record.summary.alias == requested)
                .map(|record| record.summary.alias.clone())
                .ok_or_else(|| format!("Gmail account '{requested}' was not found"));
        }
        records
            .iter()
            .find(|record| record.summary.is_default)
            .map(|record| record.summary.alias.clone())
            .ok_or_else(|| "No default Gmail account is connected".to_string())
    }
}

impl CaptainKernel {
    fn gmail_automation_runtime_service(&self) -> GmailAutomationRuntimeService {
        GmailAutomationRuntimeService::new(
            self.memory.gmail_accounts().clone(),
            self.memory.gmail_automation().clone(),
        )
    }

    pub(super) fn handle_email_automation_rules(
        &self,
        request: GmailAutomationRuleQuery,
    ) -> Result<Vec<GmailAutomationRuleView>, String> {
        self.gmail_automation_runtime_service()
            .rules(request, |id| self.registry.get(id).map(|entry| entry.name))
    }

    pub(super) fn handle_email_automation_rule_save(
        &self,
        request: GmailAutomationRuleSaveRequest,
    ) -> Result<GmailAutomationRuleView, String> {
        self.gmail_automation_runtime_service().save_rule(
            request,
            |reference| self.resolve_gmail_automation_agent(reference),
            |id| self.registry.get(id).map(|entry| entry.name),
        )
    }

    pub(super) fn handle_email_automation_rule_set_enabled(
        &self,
        request: GmailAutomationRuleStateRequest,
    ) -> Result<GmailAutomationRuleView, String> {
        self.gmail_automation_runtime_service()
            .set_rule_enabled(request, |id| self.registry.get(id).map(|entry| entry.name))
    }

    pub(super) fn handle_email_automation_rule_remove(
        &self,
        request: GmailAutomationRuleRemoveRequest,
    ) -> Result<GmailAutomationRuleView, String> {
        self.gmail_automation_runtime_service()
            .remove_rule(request, |id| self.registry.get(id).map(|entry| entry.name))
    }

    pub(super) fn handle_email_automation_deliveries(
        &self,
        request: GmailAutomationDeliveryQuery,
    ) -> Result<Vec<GmailAutomationDeliveryView>, String> {
        self.gmail_automation_runtime_service()
            .deliveries(request, |id| self.registry.get(id).map(|entry| entry.name))
    }

    pub(super) fn handle_email_automation_delivery_requeue(
        &self,
        request: GmailAutomationDeliveryRequeueRequest,
    ) -> Result<GmailAutomationDeliveryView, String> {
        self.gmail_automation_runtime_service()
            .requeue_delivery(request, |id| self.registry.get(id).map(|entry| entry.name))
    }

    fn resolve_gmail_automation_agent(&self, reference: &str) -> Result<AgentId, String> {
        if reference.is_empty() {
            return Err("Gmail automation target agent cannot be empty".to_string());
        }
        if let Ok(id) = reference.parse::<AgentId>() {
            return self.registry.get(id).map(|entry| entry.id).ok_or_else(|| {
                format!("Gmail automation target agent '{reference}' is not running")
            });
        }
        let matches = self
            .registry
            .list()
            .into_iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(reference))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => Ok(entry.id),
            [] => Err(format!(
                "Gmail automation target agent '{reference}' was not found"
            )),
            _ => Err(format!(
                "Gmail automation target agent name '{reference}' is ambiguous; use its exact UUID"
            )),
        }
    }
}

fn memory_condition(condition: GmailAutomationConditionSpec) -> GmailAutomationCondition {
    GmailAutomationCondition {
        from_contains: condition.from_contains,
        recipient_contains: condition.recipient_contains,
        subject_contains: condition.subject_contains,
        all_label_ids: condition.all_label_ids,
        any_label_ids: condition.any_label_ids,
    }
}

fn condition_view(condition: GmailAutomationCondition) -> GmailAutomationConditionSpec {
    GmailAutomationConditionSpec {
        from_contains: condition.from_contains,
        recipient_contains: condition.recipient_contains,
        subject_contains: condition.subject_contains,
        all_label_ids: condition.all_label_ids,
        any_label_ids: condition.any_label_ids,
    }
}

fn rule_view(
    record: GmailAutomationRuleRecord,
    agent_name: &impl Fn(AgentId) -> Option<String>,
) -> GmailAutomationRuleView {
    GmailAutomationRuleView {
        id: record.id,
        account_alias: record.account_alias,
        name: record.name,
        condition: condition_view(record.condition),
        action: GmailAutomationRuleActionView {
            target_agent_id: record.action.target_agent_id,
            target_agent_name: agent_name(record.action.target_agent_id),
            instruction: record.action.instruction,
            include_body: record.action.include_body,
            max_body_bytes: record.action.max_body_bytes,
            max_delivery_attempts: record.action.max_delivery_attempts,
        },
        enabled: record.enabled,
        max_fires_per_hour: record.max_fires_per_hour,
        state_version: record.state_version,
        created_at_unix_ms: record.created_at_unix_ms,
        updated_at_unix_ms: record.updated_at_unix_ms,
    }
}

fn delivery_view(
    record: GmailAutomationOutboxRecord,
    include_detail: bool,
    agent_name: &impl Fn(AgentId) -> Option<String>,
) -> GmailAutomationDeliveryView {
    let target_agent_name = agent_name(record.target_agent_id);
    let mut view = GmailAutomationDeliveryView {
        id: record.id.clone(),
        event_id: record.event_id,
        target_agent_id: record.target_agent_id,
        target_agent_name,
        status: public_status(record.status),
        attempt_count: record.attempt_count,
        max_attempts: record.max_attempts,
        run_after_unix_ms: record.run_after_unix_ms,
        lease_expires_at_unix_ms: record.lease_expires_at_unix_ms,
        last_error: record.last_error,
        delivered_at_unix_ms: record.delivered_at_unix_ms,
        created_at_unix_ms: record.created_at_unix_ms,
        updated_at_unix_ms: record.updated_at_unix_ms,
        delivery_session_id: gmail_delivery_session_id(&record.id).to_string(),
        rule_id: None,
        rule_version: None,
        rule_name: None,
        account_alias: None,
        message_id: None,
        history_id: None,
        instruction: None,
        include_body: None,
        payload_error: None,
        untrusted_message_metadata: None,
    };
    if include_detail {
        match serde_json::from_str::<GmailAutomationDeliveryPayload>(&record.payload_json) {
            Ok(payload) => {
                view.rule_id = Some(payload.rule_id);
                view.rule_version = Some(payload.rule_version);
                view.rule_name = Some(payload.rule_name);
                view.account_alias = Some(payload.account_alias);
                view.message_id = Some(payload.message_id);
                view.history_id = Some(payload.history_id);
                view.instruction = Some(payload.instruction);
                view.include_body = Some(payload.include_body);
                view.untrusted_message_metadata = Some(payload.metadata);
            }
            Err(_) => view.payload_error = Some("payload is corrupt or incompatible".to_string()),
        }
    }
    view
}

fn memory_status(status: GmailAutomationDeliveryState) -> GmailAutomationOutboxStatus {
    match status {
        GmailAutomationDeliveryState::Pending => GmailAutomationOutboxStatus::Pending,
        GmailAutomationDeliveryState::Delivering => GmailAutomationOutboxStatus::Delivering,
        GmailAutomationDeliveryState::RetryWait => GmailAutomationOutboxStatus::RetryWait,
        GmailAutomationDeliveryState::Delivered => GmailAutomationOutboxStatus::Delivered,
        GmailAutomationDeliveryState::Dead => GmailAutomationOutboxStatus::Dead,
        GmailAutomationDeliveryState::Uncertain => GmailAutomationOutboxStatus::Uncertain,
    }
}

fn public_status(status: GmailAutomationOutboxStatus) -> GmailAutomationDeliveryState {
    match status {
        GmailAutomationOutboxStatus::Pending => GmailAutomationDeliveryState::Pending,
        GmailAutomationOutboxStatus::Delivering => GmailAutomationDeliveryState::Delivering,
        GmailAutomationOutboxStatus::RetryWait => GmailAutomationDeliveryState::RetryWait,
        GmailAutomationOutboxStatus::Delivered => GmailAutomationDeliveryState::Delivered,
        GmailAutomationOutboxStatus::Dead => GmailAutomationDeliveryState::Dead,
        GmailAutomationOutboxStatus::Uncertain => GmailAutomationDeliveryState::Uncertain,
    }
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis().max(0)
}

#[cfg(test)]
mod tests {
    use captain_memory::gmail_accounts::NewGmailAccount;
    use captain_memory::gmail_automation::NewGmailAutomationMatch;
    use captain_memory::MemorySubstrate;
    use captain_types::email::GmailAccessProfile;

    use super::*;

    fn service() -> (MemorySubstrate, GmailAutomationRuntimeService) {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        memory
            .gmail_accounts()
            .upsert(NewGmailAccount {
                alias: GmailAccountAlias::parse("work").unwrap(),
                email_address: "owner@example.com".to_string(),
                access_profile: GmailAccessProfile::Assistant,
                granted_scopes: GmailAccessProfile::Assistant
                    .required_scopes()
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect(),
                token_vault_key: "CAPTAIN_GMAIL_TOKEN_TEST".to_string(),
                client_vault_key: "CAPTAIN_GMAIL_CLIENT_TEST".to_string(),
                history_id: Some("100".to_string()),
                make_default: true,
            })
            .unwrap();
        let service = GmailAutomationRuntimeService::new(
            memory.gmail_accounts().clone(),
            memory.gmail_automation().clone(),
        );
        (memory, service)
    }

    fn request(confirm_automation: bool) -> GmailAutomationRuleSaveRequest {
        GmailAutomationRuleSaveRequest {
            id: None,
            expected_version: None,
            account_alias: None,
            name: "Invoice review".to_string(),
            condition: GmailAutomationConditionSpec {
                from_contains: Some("billing@example.com".to_string()),
                recipient_contains: None,
                subject_contains: None,
                all_label_ids: vec!["INBOX".to_string()],
                any_label_ids: Vec::new(),
            },
            target_agent: "captain".to_string(),
            instruction: "Create a review task".to_string(),
            include_body: false,
            max_body_bytes: 32 * 1024,
            max_delivery_attempts: 3,
            enabled: true,
            max_fires_per_hour: 20,
            confirm_automation,
        }
    }

    #[test]
    fn rule_save_requires_confirmation_and_updates_with_compare_and_swap() {
        let (_memory, service) = service();
        let captain = AgentId::from_string("captain");
        assert!(service
            .save_rule(request(false), |_| Ok(captain), |_| None)
            .unwrap_err()
            .contains("confirm_automation=true"));

        let created = service
            .save_rule(request(true), |_| Ok(captain), |_| Some("captain".into()))
            .unwrap();
        assert_eq!(created.id, "work-invoice-review");
        assert_eq!(created.state_version, 1);
        let mut update = request(true);
        update.id = Some(created.id.clone());
        update.expected_version = Some(created.state_version);
        update.enabled = false;
        let updated = service
            .save_rule(update, |_| Ok(captain), |_| Some("captain".into()))
            .unwrap();
        assert_eq!(updated.state_version, 2);
        assert!(!updated.enabled);
    }

    #[test]
    fn delivery_lists_are_redacted_and_explicit_inspection_marks_metadata_untrusted() {
        let (memory, service) = service();
        let captain = AgentId::from_string("captain");
        let rule = service
            .save_rule(request(true), |_| Ok(captain), |_| Some("captain".into()))
            .unwrap();
        let queued = memory
            .gmail_automation()
            .enqueue_match(&NewGmailAutomationMatch {
                idempotency_key: "gmail:invoice:message-1".to_string(),
                rule_id: rule.id,
                expected_rule_version: rule.state_version,
                account_alias: GmailAccountAlias::parse("work").unwrap(),
                message_id: "message-1".to_string(),
                history_id: "101".to_string(),
                metadata_json: r#"{"subject":"private invoice"}"#.to_string(),
                occurred_at_unix_ms: now_unix_ms(),
            })
            .unwrap()
            .outbox
            .unwrap();

        let listed = service
            .deliveries(
                GmailAutomationDeliveryQuery {
                    delivery_id: None,
                    status: None,
                    limit: 10,
                },
                |_| Some("captain".into()),
            )
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].message_id.is_none());
        assert!(listed[0].untrusted_message_metadata.is_none());

        let detail = service
            .deliveries(
                GmailAutomationDeliveryQuery {
                    delivery_id: Some(queued.id),
                    status: None,
                    limit: 1,
                },
                |_| Some("captain".into()),
            )
            .unwrap();
        assert_eq!(detail[0].message_id.as_deref(), Some("message-1"));
        assert_eq!(
            detail[0].untrusted_message_metadata.as_ref().unwrap()["subject"],
            "private invoice"
        );
        assert!(!serde_json::to_string(&detail[0])
            .unwrap()
            .contains("private invoice"));
    }
}
