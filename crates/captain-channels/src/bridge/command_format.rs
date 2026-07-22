//! Operator-facing text for channel commands.

use captain_types::agent::AgentId;

pub(crate) fn format_start_message(agents: &[(AgentId, String)]) -> String {
    let mut msg =
        "Welcome to Captain! I connect you to AI agents.\n\nAvailable agents:\n".to_string();
    append_agent_list(&mut msg, agents, "  (none running)\n");
    msg.push_str(
        "\nCommands:\n/agents - list agents\n/agent <name> - select an agent\n/help - show this help",
    );
    msg
}

pub(crate) fn format_help_message() -> String {
    "Captain Bot Commands:\n\
     \n\
     Session:\n\
     /agents - list running agents\n\
     /agent <name> - select which agent to talk to\n\
     /new - reset session (clear messages)\n\
     /compact - trigger LLM session compaction\n\
     /model [name] - show or switch agent model\n\
     /stop - cancel current agent run\n\
     /usage - show session token usage and cost\n\
     /think [on|off] - toggle extended thinking\n\
     \n\
     Info:\n\
     /models - list available AI models\n\
     /providers - show configured providers\n\
     /skills - list installed skills\n\
     /hands - list available and active hands\n\
     /status - show system status\n\
     /health - show daemon health\n\
     /version - show daemon version and paths\n\
     /config - show exact config.toml (owner only)\n\
     /reload - hot-reload config.toml (owner only)\n\
     /restart - restart the daemon (owner only)\n\
     /shutdown confirm - stop the daemon (owner only)\n\
     \n\
     Automation:\n\
     /workflows - list workflows\n\
     /workflow run <name> [input] - run a workflow\n\
     /triggers - list event triggers\n\
     /trigger add <agent> <pattern> <prompt> - create trigger\n\
     /trigger del <id> - remove trigger\n\
     /schedules - list cron jobs\n\
     /schedule add <agent> <cron-5-fields> <message> - create job\n\
     /schedule del <id> - remove job\n\
     /schedule run <id> - run job now\n\
     /approvals - list pending approvals\n\
     /approve <id> - approve a request\n\
     /reject <id> - reject a request\n\
     /learnings - list pending learning candidates\n\
     /learn_approve <id> - approve a learning candidate\n\
     /learn_reject <id> - reject a learning candidate\n\
     /skill_refinements - list existing-skill refinements\n\
     /skill_refine_approve <id> - approve a skill refinement\n\
     /skill_refine_reject <id> - reject a skill refinement\n\
     /project_answer <id> <réponse> - answer a project question\n\
     \n\
     Monitoring:\n\
     /budget - show spending limits and current costs\n\
     /peers - show OFP peer network status\n\
     /a2a - list discovered external A2A agents\n\
     \n\
     Channel:\n\
     /sethome [chat_id] - register this chat as home (v3.8h)\n\
     /gethome - show current home chat for this channel\n\
     \n\
     /start - show welcome message\n\
     /help - show this help"
        .to_string()
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
        assert!(text.contains("/agent <name> - select an agent"));
    }

    #[test]
    fn start_message_handles_empty_agent_list() {
        let text = format_start_message(&[]);

        assert!(text.contains("  (none running)"));
        assert!(text.contains("/help - show this help"));
    }

    #[test]
    fn help_message_keeps_operational_sections() {
        let text = format_help_message();

        assert!(text.contains("Session:"));
        assert!(text.contains("Automation:"));
        assert!(text.contains("Monitoring:"));
        assert!(text.contains("/project_answer <id> <réponse>"));
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
