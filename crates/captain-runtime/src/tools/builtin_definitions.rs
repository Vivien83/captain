//! Complete builtin tool definition registry.

use captain_types::tool::ToolDefinition;

use super::{
    a2a_tool_definitions, agent_tool_definitions, browser_tool_definitions,
    capspec_management_tool_definitions, channel_tool_definitions, config_tool_definitions,
    coordination_tool_definitions, discovery_tool_definitions, document_tool_definitions,
    file_tool_definitions, fleet_tool_definitions, goal_tool_definitions, hand_tool_definitions,
    improvement_tool_definitions, knowledge_tool_definitions, location_tool_definitions,
    mcp_tool_definitions, memory_tool_definitions, meta_tool_definitions,
    multimedia_tool_definitions, package_tool_definitions, peer_tool_definitions,
    project_tool_definitions, schedule_tool_definitions, session_workspace_tool_definitions,
    shell_tool_definitions, skill_tool_definitions, ssh_tool_definitions,
    tool_run_tool_definitions, update_tool_definitions, web_tool_definitions,
};

pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = desktop_tool_definitions();
    definitions.extend(config_tool_definitions());
    definitions.extend(mcp_tool_definitions());
    definitions.extend(tool_run_tool_definitions());
    definitions.extend(goal_tool_definitions());
    definitions.extend(peer_tool_definitions());
    definitions.extend(coordination_tool_definitions());
    definitions.extend(schedule_tool_definitions());
    definitions.extend(knowledge_tool_definitions());
    definitions.extend(meta_tool_definitions());
    definitions.extend(capspec_management_tool_definitions());
    definitions.extend(update_tool_definitions());
    definitions.extend(multimedia_tool_definitions());
    definitions.extend(location_tool_definitions());
    definitions.extend(browser_tool_definitions());
    definitions.extend(project_tool_definitions());
    definitions.extend(session_workspace_tool_definitions());
    definitions.extend(improvement_tool_definitions());
    definitions.extend(channel_tool_definitions());
    definitions.extend(hand_tool_definitions());
    definitions.extend(a2a_tool_definitions());
    definitions.extend(skill_tool_definitions());
    definitions.extend(file_tool_definitions());
    definitions.extend(document_tool_definitions());
    definitions.extend(web_tool_definitions());
    definitions.extend(package_tool_definitions());
    definitions.extend(ssh_tool_definitions());
    definitions.extend(shell_tool_definitions());
    definitions.extend(agent_tool_definitions());
    definitions.extend(fleet_tool_definitions());
    definitions.extend(memory_tool_definitions());
    definitions.extend(discovery_tool_definitions());
    definitions
}

fn desktop_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "screenshot".to_string(),
        description: "Capture une capture d'ecran de l'ecran entier et l'enregistre sur disque. Utiliser pour envoyer a l'utilisateur un apercu visuel du bureau ou diagnostiquer un probleme UI. Detecte automatiquement la commande native : screencapture (macOS), grim/gnome-screenshot/import (Linux), nircmd (Windows). Si aucune commande n'est disponible, retourne une erreur explicite. Le fichier PNG retourne peut ensuite etre envoye via channel_send avec file_path.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "save_path": {
                    "type": "string",
                    "description": "Chemin absolu ou relatif au workspace ou enregistrer le PNG. Si omis, /tmp/captain_screenshot_<timestamp>.png."
                }
            }
        }),
    }]
}
