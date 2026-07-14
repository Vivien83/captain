//! Pure preflight decision logic for channel model-switch commands.

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ModelSwitchPlanDecision {
    Blocked,
    Unchanged {
        provider: String,
        target_model: String,
    },
    NeedsConfirmation {
        target_model: String,
        target_provider: Option<String>,
        recommended_session_strategy: Option<String>,
    },
    ApplyNow {
        target_model: String,
        target_provider: Option<String>,
    },
}

fn plan_bool(plan: &serde_json::Value, key: &str) -> bool {
    plan.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn plan_str<'a>(plan: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    plan.get(key).and_then(|v| v.as_str())
}

pub(super) fn classify_model_switch_plan(
    plan: &serde_json::Value,
    requested_model: &str,
) -> ModelSwitchPlanDecision {
    if !plan_bool(plan, "can_apply") {
        return ModelSwitchPlanDecision::Blocked;
    }

    let target_model = plan_str(plan, "target_model")
        .unwrap_or(requested_model)
        .to_string();
    let target_provider = plan_str(plan, "target_provider").map(ToString::to_string);

    if !plan_bool(plan, "provider_changed") && !plan_bool(plan, "model_changed") {
        return ModelSwitchPlanDecision::Unchanged {
            provider: target_provider.unwrap_or_else(|| "unknown".to_string()),
            target_model,
        };
    }

    if plan_bool(plan, "session_strategy_required") {
        return ModelSwitchPlanDecision::NeedsConfirmation {
            target_model,
            target_provider,
            recommended_session_strategy: plan_str(plan, "recommended_session_strategy")
                .map(ToString::to_string),
        };
    }

    ModelSwitchPlanDecision::ApplyNow {
        target_model,
        target_provider,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocked_when_preflight_cannot_apply() {
        assert_eq!(
            classify_model_switch_plan(&json!({"can_apply": false}), "gpt-5.5"),
            ModelSwitchPlanDecision::Blocked
        );
    }

    #[test]
    fn unchanged_preserves_provider_and_target_model() {
        assert_eq!(
            classify_model_switch_plan(
                &json!({
                    "can_apply": true,
                    "target_provider": "codex",
                    "target_model": "gpt-5.5",
                    "provider_changed": false,
                    "model_changed": false
                }),
                "gpt-5.5"
            ),
            ModelSwitchPlanDecision::Unchanged {
                provider: "codex".to_string(),
                target_model: "gpt-5.5".to_string()
            }
        );
    }

    #[test]
    fn confirmation_keeps_target_and_recommended_strategy() {
        assert_eq!(
            classify_model_switch_plan(
                &json!({
                    "can_apply": true,
                    "target_provider": "codex",
                    "target_model": "gpt-5.5",
                    "model_changed": true,
                    "session_strategy_required": true,
                    "recommended_session_strategy": "compact_session"
                }),
                "gpt-5.5"
            ),
            ModelSwitchPlanDecision::NeedsConfirmation {
                target_model: "gpt-5.5".to_string(),
                target_provider: Some("codex".to_string()),
                recommended_session_strategy: Some("compact_session".to_string())
            }
        );
    }

    #[test]
    fn apply_now_falls_back_to_requested_model() {
        assert_eq!(
            classify_model_switch_plan(
                &json!({
                    "can_apply": true,
                    "provider_changed": true,
                    "session_strategy_required": false
                }),
                "gpt-5.5"
            ),
            ModelSwitchPlanDecision::ApplyNow {
                target_model: "gpt-5.5".to_string(),
                target_provider: None
            }
        );
    }
}
