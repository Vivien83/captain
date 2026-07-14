use super::screens::{chat::PendingModelSwitch, chat_model_label};
use captain_kernel::model_switch::{ModelSwitchPlan, ModelSwitchSessionStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashModel<'a> {
    OpenPicker,
    Switch {
        model: &'a str,
        strategy: Option<&'static str>,
    },
}

pub(crate) fn model_for<'a>(command: &str, args: &'a str) -> Option<SlashModel<'a>> {
    if command != "/model" {
        return None;
    }
    if args.is_empty() {
        Some(SlashModel::OpenPicker)
    } else {
        let (model, strategy) = super::command_args::parse_model_switch_args(args);
        Some(SlashModel::Switch { model, strategy })
    }
}

pub(crate) enum DaemonModelSwitchDecision {
    Apply(String),
    RequestChoice(PendingModelSwitch),
}

pub(crate) enum InProcessModelSwitchDecision {
    Apply(ModelSwitchSessionStrategy),
    RequestChoice(PendingModelSwitch),
}

pub(crate) fn no_models_available_message() -> &'static str {
    "No models available."
}

pub(crate) fn daemon_preflight_parse_failed_message(error: impl std::fmt::Display) -> String {
    format!("Model switch preflight parse failed: {error}")
}

pub(crate) fn daemon_preflight_http_failed_message(status: impl std::fmt::Display) -> String {
    format!("Model switch preflight failed ({status})")
}

pub(crate) fn daemon_preflight_error_message(error: impl std::fmt::Display) -> String {
    format!("Model switch preflight failed: {error}")
}

pub(crate) fn daemon_blocking_issues(plan: &serde_json::Value) -> String {
    plan["blocking_issues"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| "Unknown issue".to_string())
}

pub(crate) fn model_switch_blocked_message(issues: &str) -> String {
    format!("Model switch blocked:\n{issues}")
}

pub(crate) fn safe_switch_http_failed_message(status: impl std::fmt::Display) -> String {
    format!("Safe model switch failed ({status})")
}

pub(crate) fn safe_switch_error_message(error: impl std::fmt::Display) -> String {
    format!("Safe model switch failed: {error}")
}

pub(crate) fn safe_switch_default_success_message() -> &'static str {
    "Model switched safely."
}

pub(crate) fn inprocess_preflight_failed_message(error: impl std::fmt::Display) -> String {
    format!("Switch preflight failed: {error}")
}

pub(crate) fn switch_failed_message(error: impl std::fmt::Display) -> String {
    format!("Switch failed: {error}")
}

pub(crate) fn no_backend_connected_message() -> &'static str {
    "No backend connected."
}

pub(crate) fn daemon_model_switch_decision(
    model_id: &str,
    session_strategy: Option<&str>,
    plan: &serde_json::Value,
) -> DaemonModelSwitchDecision {
    if let Some(strategy) = session_strategy {
        return DaemonModelSwitchDecision::Apply(strategy.to_string());
    }
    if plan["session_strategy_required"].as_bool().unwrap_or(false) {
        return DaemonModelSwitchDecision::RequestChoice(daemon_pending_model_switch(
            model_id, plan,
        ));
    }
    DaemonModelSwitchDecision::Apply(
        plan["recommended_session_strategy"]
            .as_str()
            .unwrap_or("new_session")
            .to_string(),
    )
}

pub(crate) fn inprocess_model_switch_decision(
    model_id: &str,
    session_strategy: Option<&str>,
    plan: &ModelSwitchPlan,
) -> InProcessModelSwitchDecision {
    match session_strategy {
        Some("compact_session") => {
            InProcessModelSwitchDecision::Apply(ModelSwitchSessionStrategy::CompactSession)
        }
        Some(_) => InProcessModelSwitchDecision::Apply(ModelSwitchSessionStrategy::NewSession),
        None if plan.session_strategy_required => InProcessModelSwitchDecision::RequestChoice(
            inprocess_pending_model_switch(model_id, plan),
        ),
        None => InProcessModelSwitchDecision::Apply(plan.recommended_session_strategy),
    }
}

pub(crate) fn daemon_apply_success(
    body: &serde_json::Value,
    fallback_model_id: &str,
) -> (Option<String>, String) {
    let provider = body["plan"]["target_provider"].as_str().unwrap_or("?");
    let model = body["plan"]["target_model"]
        .as_str()
        .unwrap_or(fallback_model_id);
    let label = chat_model_label::compose_model_label(provider, model);
    let message = body["message"]
        .as_str()
        .unwrap_or(safe_switch_default_success_message())
        .to_string();
    (label, message)
}

fn daemon_pending_model_switch(model_id: &str, plan: &serde_json::Value) -> PendingModelSwitch {
    PendingModelSwitch {
        model_id: model_id.to_string(),
        current_provider: plan["current_provider"].as_str().unwrap_or("?").to_string(),
        current_model: plan["current_model"].as_str().unwrap_or("?").to_string(),
        target_provider: plan["target_provider"].as_str().unwrap_or("?").to_string(),
        target_model: plan["target_model"]
            .as_str()
            .unwrap_or(model_id)
            .to_string(),
        risk: plan["risk"].as_str().unwrap_or("medium").to_string(),
        recommended_session_strategy: plan["recommended_session_strategy"]
            .as_str()
            .unwrap_or("new_session")
            .to_string(),
        active_message_count: plan["active_message_count"].as_u64().unwrap_or(0) as usize,
        canonical_summary_present: plan["canonical_summary_present"].as_bool().unwrap_or(false),
    }
}

fn inprocess_pending_model_switch(model_id: &str, plan: &ModelSwitchPlan) -> PendingModelSwitch {
    PendingModelSwitch {
        model_id: model_id.to_string(),
        current_provider: plan.current_provider.clone(),
        current_model: plan.current_model.clone(),
        target_provider: plan.target_provider.clone(),
        target_model: plan.target_model.clone(),
        risk: format!("{:?}", plan.risk).to_ascii_lowercase(),
        recommended_session_strategy: plan.recommended_session_strategy.as_str().to_string(),
        active_message_count: plan.active_message_count,
        canonical_summary_present: plan.canonical_summary_present,
    }
}

#[cfg(test)]
#[path = "slash_model/tests.rs"]
mod tests;
