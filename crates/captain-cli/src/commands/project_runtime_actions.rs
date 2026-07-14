use serde_json::Value;

pub(crate) fn project_command(action: &str, project_id: &str) -> String {
    format!("captain project {action} {}", shell_quote_arg(project_id))
}

pub(crate) fn project_answer_command(project_id: &str, ask_id: &str) -> String {
    format!(
        "captain project answer {} --ask-id {} --answer \"...\"",
        shell_quote_arg(project_id),
        shell_quote_arg(ask_id)
    )
}

pub(crate) fn project_runtime_action_commands(
    body: &Value,
    fallback_project_id: &str,
) -> Vec<String> {
    let project_id = project_runtime_command_id(body, fallback_project_id);
    let Some(actions) = body
        .pointer("/operator_status/actions")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    actions
        .iter()
        .filter_map(|action| project_runtime_action_command(action, &project_id))
        .collect()
}

fn project_runtime_command_id(body: &Value, fallback_project_id: &str) -> String {
    [
        "/operator_status/project_slug",
        "/project/slug",
        "/operator_status/project_id",
        "/project/id",
    ]
    .iter()
    .filter_map(|pointer| body.pointer(pointer).and_then(Value::as_str))
    .find(|value| !value.trim().is_empty())
    .unwrap_or(fallback_project_id)
    .trim()
    .to_string()
}

fn project_runtime_action_command(action: &Value, project_id: &str) -> Option<String> {
    let label = action.get("label").and_then(Value::as_str)?;
    match label {
        "answer_question" => {
            let ask_id = action
                .pointer("/body_hint/ask_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("<ask_id>");
            Some(project_answer_command(project_id, ask_id))
        }
        "respond_tool_request" => {
            let mut command = format!(
                "captain project tool-request {} approve",
                shell_quote_arg(project_id)
            );
            if let Some(phase) = action
                .pointer("/body_hint/phase")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                command.push_str(&format!(" --phase {}", shell_quote_arg(phase)));
            }
            command.push_str(" --reason \"...\"");
            Some(format!("{command} (or replace approve with deny)"))
        }
        "resume_runtime" => Some(project_command("resume", project_id)),
        "start_runtime" => Some(project_command("start", project_id)),
        "pause_runtime" => Some(project_command("pause", project_id)),
        "takeover_runtime" => Some(project_command("takeover", project_id)),
        _ => None,
    }
}

fn shell_quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '=' | '+')
    }) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn action_commands_prefer_project_slug_and_keep_all_actions() {
        let body = json!({
            "operator_status": {
                "project_id": "project-1",
                "project_slug": "demo",
                "actions": [
                    {"label": "pause_runtime"},
                    {"label": "takeover_runtime"}
                ]
            }
        });

        assert_eq!(
            project_runtime_action_commands(&body, "fallback"),
            vec![
                "captain project pause demo",
                "captain project takeover demo",
            ]
        );
    }

    #[test]
    fn action_commands_format_interactive_actions() {
        let body = json!({
            "operator_status": {
                "project_id": "project-1",
                "actions": [
                    {
                        "label": "answer_question",
                        "body_hint": {"ask_id": "ask-1"}
                    },
                    {
                        "label": "respond_tool_request",
                        "body_hint": {"phase": "verify"}
                    }
                ]
            }
        });

        assert_eq!(
            project_runtime_action_commands(&body, "fallback"),
            vec![
                "captain project answer project-1 --ask-id ask-1 --answer \"...\"",
                "captain project tool-request project-1 approve --phase verify --reason \"...\" (or replace approve with deny)",
            ]
        );
    }

    #[test]
    fn action_commands_quote_runtime_identifiers() {
        let body = json!({
            "operator_status": {
                "project_id": "project 1",
                "actions": [
                    {
                        "label": "answer_question",
                        "body_hint": {"ask_id": "ask 'quoted'"}
                    },
                    {
                        "label": "respond_tool_request",
                        "body_hint": {"phase": "verify now"}
                    }
                ]
            }
        });

        assert_eq!(
            project_runtime_action_commands(&body, "fallback"),
            vec![
                "captain project answer 'project 1' --ask-id 'ask '\\''quoted'\\''' --answer \"...\"",
                "captain project tool-request 'project 1' approve --phase 'verify now' --reason \"...\" (or replace approve with deny)",
            ]
        );
    }
}
