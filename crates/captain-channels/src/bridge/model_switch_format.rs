//! Operator-facing model-switch message formatting for channel commands.

pub(super) fn plan_str<'a>(plan: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    plan.get(key).and_then(|v| v.as_str())
}

fn model_switch_issue_lines(plan: &serde_json::Value, key: &str) -> Vec<String> {
    plan.get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn format_model_switch_blocked(plan: &serde_json::Value) -> String {
    let issues = model_switch_issue_lines(plan, "blocking_issues");
    if issues.is_empty() {
        return "Je ne peux pas appliquer ce switch pour l'instant : preflight refusé sans détail."
            .to_string();
    }
    format!(
        "Je ne peux pas appliquer ce switch pour l'instant :\n- {}",
        issues.join("\n- ")
    )
}

pub(super) fn format_model_switch_prompt(plan: &serde_json::Value) -> String {
    let provider = plan_str(plan, "target_provider").unwrap_or("unknown");
    let model = plan_str(plan, "target_model").unwrap_or("unknown");
    let risk = plan_str(plan, "risk").unwrap_or("unknown");
    let mut lines = vec![
        format!("Switch sécurisé prêt vers {provider} / {model}."),
        format!("Risque preflight : {risk}."),
    ];
    let warnings = model_switch_issue_lines(plan, "warnings");
    if !warnings.is_empty() {
        lines.push(format!("Points d'attention :\n- {}", warnings.join("\n- ")));
    }
    lines.push("Choisis comment gérer la session active :".to_string());
    lines.join("\n")
}

pub(super) fn format_model_switch_apply_result(result: &serde_json::Value) -> String {
    let plan = result.get("plan").unwrap_or(result);
    let provider = plan_str(plan, "target_provider").unwrap_or("unknown");
    let model = plan_str(plan, "target_model").unwrap_or("unknown");
    let strategy = plan_str(result, "session_strategy").unwrap_or("new_session");
    let session_label = match strategy {
        "compact_session" => "contexte compacté puis nouvelle session démarrée",
        _ => "nouvelle session démarrée",
    };
    let default_note = if result
        .get("global_default_updated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        " Le modèle par défaut global a aussi été mis à jour."
    } else {
        ""
    };
    format!("✅ Switché sur {provider} / {model} — {session_label}.{default_note}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocked_message_lists_issues_or_static_fallback() {
        assert_eq!(
            format_model_switch_blocked(&json!({})),
            "Je ne peux pas appliquer ce switch pour l'instant : preflight refusé sans détail."
        );
        assert_eq!(
            format_model_switch_blocked(&json!({
                "blocking_issues": ["provider auth missing", 7, "model unavailable"]
            })),
            "Je ne peux pas appliquer ce switch pour l'instant :\n- provider auth missing\n- model unavailable"
        );
    }

    #[test]
    fn prompt_message_includes_target_risk_and_warnings() {
        let message = format_model_switch_prompt(&json!({
            "target_provider": "codex",
            "target_model": "gpt-5.5",
            "risk": "medium",
            "warnings": ["session will restart"]
        }));
        assert!(message.contains("Switch sécurisé prêt vers codex / gpt-5.5."));
        assert!(message.contains("Risque preflight : medium."));
        assert!(message.contains("- session will restart"));
        assert!(message.ends_with("Choisis comment gérer la session active :"));
    }

    #[test]
    fn apply_result_mentions_strategy_and_global_default() {
        assert_eq!(
            format_model_switch_apply_result(&json!({
                "plan": {
                    "target_provider": "codex",
                    "target_model": "gpt-5.5"
                },
                "session_strategy": "compact_session",
                "global_default_updated": true
            })),
            "✅ Switché sur codex / gpt-5.5 — contexte compacté puis nouvelle session démarrée. Le modèle par défaut global a aussi été mis à jour."
        );
    }
}
