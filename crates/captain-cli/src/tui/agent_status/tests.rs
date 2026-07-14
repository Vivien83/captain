use super::*;

#[test]
fn full_tui_spawn_status_messages_preserve_hermes_text() {
    assert_eq!(
        invalid_manifest_message("expected table"),
        "Invalid manifest: expected table"
    );
    assert_eq!(spawn_failed_message("capacity"), "Spawn failed: capacity");
    assert_eq!(no_backend_connected_message(), "No backend connected");
}

#[test]
fn full_tui_agent_event_status_messages_preserve_hermes_text() {
    assert_eq!(agent_killed_message("agent-1"), "Agent agent-1 killed.");
    assert_eq!(agent_kill_failed_message("offline"), "Kill failed: offline");
    assert_eq!(
        agent_skills_updated_message("agent-1"),
        "Skills updated for agent agent-1."
    );
    assert_eq!(
        agent_mcp_servers_updated_message("agent-1"),
        "MCP servers updated for agent agent-1."
    );
}
