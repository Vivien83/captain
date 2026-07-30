//! Operator-facing text for channel commands.

use crate::channel_commands::{format_command_help, format_command_subset, CommandLanguage};
use captain_types::agent::AgentId;

pub(crate) fn format_start_message(agents: &[(AgentId, String)]) -> String {
    let mut msg =
        "Welcome to Captain! I connect you to AI agents.\n\nAvailable agents:\n".to_string();
    append_agent_list(&mut msg, agents, "  (none running)\n");
    msg.push_str("\nCommands:\n");
    msg.push_str(&format_command_subset(
        &["agents", "agent", "help"],
        CommandLanguage::English,
    ));
    msg
}

pub(crate) fn format_help_message() -> String {
    format_command_help(CommandLanguage::English)
}

pub(crate) fn format_agents_message(agents: &[(AgentId, String)]) -> String {
    if agents.is_empty() {
        return "No agents running.".to_string();
    }
    let mut msg = "Running agents:\n".to_string();
    append_agent_list(&mut msg, agents, "");
    msg
}

pub(crate) fn format_workflow_usage() -> String {
    "Usage: /workflow run <name> [input]".to_string()
}

pub(crate) fn format_trigger_usage() -> String {
    "Usage:\n  /trigger add <agent> <pattern> <prompt>\n  /trigger del <id-prefix>".to_string()
}

pub(crate) fn format_schedule_usage() -> String {
    "Usage:\n  /schedule add <agent> <cron-5-fields> <message>\n  /schedule del <id-prefix>\n  /schedule run <id-prefix>".to_string()
}

pub(crate) fn format_id_prefix_usage(command: &str) -> String {
    format!("Usage: /{command} <id-prefix>")
}

pub(crate) fn legacy_skill_synthesizer_retired() -> String {
    "L'ancien SkillSynthesizer est archivé en lecture seule. Utilise Learning dans Telegram, le TUI, le Web ou le Desktop pour consulter et décider les workflows durables Skill Learning V2."
        .to_string()
}

pub(crate) fn format_project_answer_usage() -> String {
    "Usage: /project_answer <id-prefix> <réponse>".to_string()
}

fn append_agent_list(msg: &mut String, agents: &[(AgentId, String)], empty_line: &str) {
    if agents.is_empty() {
        msg.push_str(empty_line);
    } else {
        for (_, name) in agents {
            msg.push_str(&format!("  - {name}\n"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents(names: &[&str]) -> Vec<(AgentId, String)> {
        names
            .iter()
            .map(|name| (AgentId::new(), (*name).to_string()))
            .collect()
    }

    #[test]
    fn start_message_lists_agents_and_core_commands() {
        let text = format_start_message(&agents(&["captain", "vision"]));

        assert!(text.contains("Available agents:"));
        assert!(text.contains("  - captain"));
        assert!(text.contains("  - vision"));
        assert!(text.contains("/agent <name> - Select an agent"));
    }

    #[test]
    fn start_message_handles_empty_agent_list() {
        let text = format_start_message(&[]);

        assert!(text.contains("  (none running)"));
        assert!(text.contains("/help - Show every available command"));
    }

    #[test]
    fn help_message_keeps_operational_sections() {
        let text = format_help_message();

        assert!(text.contains("Session:"));
        assert!(text.contains("Automation:"));
        assert!(text.contains("Monitoring:"));
        assert!(text.contains("/project_answer <id> <answer>"));
        assert!(text.contains("/reasoning [auto|level]"));
        assert!(text.contains("/learn_approve <id>"));
    }

    #[test]
    fn agents_message_reports_empty_or_running_agents() {
        assert_eq!(format_agents_message(&[]), "No agents running.");

        let text = format_agents_message(&agents(&["captain"]));
        assert_eq!(text, "Running agents:\n  - captain\n");
    }

    #[test]
    fn automation_usage_messages_stay_specific() {
        assert_eq!(
            format_workflow_usage(),
            "Usage: /workflow run <name> [input]"
        );
        assert!(format_trigger_usage().contains("/trigger del <id-prefix>"));
        assert!(format_schedule_usage().contains("/schedule run <id-prefix>"));
    }

    #[test]
    fn active_review_usage_messages_use_command_names() {
        assert_eq!(
            format_id_prefix_usage("approve_session"),
            "Usage: /approve_session <id-prefix>"
        );
        assert_eq!(
            format_project_answer_usage(),
            "Usage: /project_answer <id-prefix> <réponse>"
        );
    }
}
