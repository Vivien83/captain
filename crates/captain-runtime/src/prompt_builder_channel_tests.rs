use super::*;

/// Active messaging channels and runtime surfaces keep explicit execution
/// context hints. The most critical is `cron`: the LLM must know there is no
/// user to clarify with, so it must act fully or fail loud instead of asking.
#[test]
fn test_active_and_runtime_channels_supported() {
    let channels = [
        "telegram", "discord", "signal", "email", "cron", "cli", "desktop", "acp", "mcp",
    ];
    for ch in channels {
        let section = build_channel_section(ch);
        assert!(
            !section.is_empty(),
            "channel {ch} must produce a non-empty hint"
        );
        assert!(
            section.contains("## Channel"),
            "channel {ch} hint must start with ## Channel"
        );
    }
}

#[test]
fn test_frozen_messaging_channels_use_generic_prompt_hint() {
    for ch in ["slack", "whatsapp", "irc", "matrix", "teams"] {
        let section = build_channel_section(ch);
        assert!(section.contains("Unknown or frozen channel"));
        assert!(section.contains("Use markdown formatting where supported."));
        assert!(
            !section.contains("Slack mrkdwn")
                && !section.contains("Voice notes")
                && !section.contains("plain text only")
                && !section.contains("Matrix supports")
                && !section.contains("Teams-flavored"),
            "frozen channel {ch} must not keep specialized prompt guidance"
        );
    }
}

#[test]
fn test_cron_channel_declares_no_user() {
    let section = build_channel_section("cron");
    let lower = section.to_ascii_lowercase();
    assert!(
        lower.contains("no user") || lower.contains("no human"),
        "cron hint must tell the LLM there is no user present"
    );
}

#[test]
fn test_acp_mcp_channels_supported() {
    let acp = build_channel_section("acp");
    let mcp = build_channel_section("mcp");
    assert!(acp.contains("ACP") || acp.contains("editor"));
    assert!(mcp.contains("MCP") || mcp.contains("tool server"));
}

#[test]
fn test_channel_telegram() {
    let section = build_channel_section("telegram");
    assert!(section.contains("4096"));
    assert!(section.contains("Telegram"));
    assert!(section.contains("No markdown tables"));
    assert!(section.contains("aligned"));
}

#[test]
fn test_channel_discord() {
    let section = build_channel_section("discord");
    assert!(section.contains("2000"));
    assert!(section.contains("Discord"));
}

#[test]
fn test_channel_unknown_gets_default() {
    let section = build_channel_section("smoke_signal");
    assert!(section.contains("4096"));
    assert!(section.contains("smoke_signal"));
}
