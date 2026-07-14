pub(crate) fn invalid_manifest_message(error: impl std::fmt::Display) -> String {
    format!("Invalid manifest: {error}")
}

pub(crate) fn spawn_failed_message(error: impl std::fmt::Display) -> String {
    format!("Spawn failed: {error}")
}

pub(crate) fn no_backend_connected_message() -> &'static str {
    "No backend connected"
}

pub(crate) fn agent_killed_message(id: &str) -> String {
    format!("Agent {id} killed.")
}

pub(crate) fn agent_kill_failed_message(error: impl std::fmt::Display) -> String {
    format!("Kill failed: {error}")
}

pub(crate) fn agent_skills_updated_message(id: &str) -> String {
    format!("Skills updated for agent {id}.")
}

pub(crate) fn agent_mcp_servers_updated_message(id: &str) -> String {
    format!("MCP servers updated for agent {id}.")
}

#[cfg(test)]
#[path = "agent_status/tests.rs"]
mod tests;
