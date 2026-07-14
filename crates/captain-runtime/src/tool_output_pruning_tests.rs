use super::*;
use captain_types::message::Role;

fn tool_result_message(id: &str, tool_name: &str, content: String) -> Message {
    Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            tool_name: tool_name.to_string(),
            content,
            is_error: false,
        }]),
    }
}

fn big_output(chars: usize) -> String {
    "x".repeat(chars)
}

/// One old giant tool result, then enough recent text to fill the reserved
/// window — the old result gets pruned, the recent ones stay intact.
#[test]
fn prunes_old_large_tool_result_and_keeps_recent_intact() {
    let mut messages = vec![tool_result_message(
        "call-old",
        "shell_exec",
        big_output(200_000),
    )];
    // 4 × 44k chars (~11k tokens each): the reserved 40k-token window is
    // crossed on the oldest of these, so everything before them is prunable.
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let report = prune_old_tool_outputs(&messages, 40_000);

    assert_eq!(report.pruned_results, 1);
    assert!(report.estimated_tokens_saved >= PRUNE_MINIMUM_SAVINGS_TOKENS);
    let MessageContent::Blocks(blocks) = &report.messages[0].content else {
        panic!("expected blocks");
    };
    let ContentBlock::ToolResult {
        tool_use_id,
        tool_name,
        content,
        ..
    } = &blocks[0]
    else {
        panic!("expected tool result");
    };
    assert_eq!(tool_use_id, "call-old");
    assert_eq!(tool_name, "shell_exec");
    assert!(content.contains("CAPTAIN CONTEXT ECONOMY"));
    assert!(content.contains("200000 chars"));
    assert!(content.len() < 600);
    // The most recent message is untouched.
    let last = report.messages.last().unwrap();
    assert_eq!(last.content.text_length(), 44_000);
}

#[test]
fn small_old_tool_results_are_never_pruned() {
    let mut messages = vec![
        tool_result_message("call-small", "shell_exec", "short output".to_string()),
        tool_result_message("call-big", "shell_exec", big_output(200_000)),
    ];
    // 4 × 44k chars (~11k tokens each): the reserved 40k-token window is
    // crossed on the oldest of these, so everything before them is prunable.
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let report = prune_old_tool_outputs(&messages, 40_000);

    assert_eq!(report.pruned_results, 1);
    let MessageContent::Blocks(blocks) = &report.messages[0].content else {
        panic!("expected blocks");
    };
    let ContentBlock::ToolResult { content, .. } = &blocks[0] else {
        panic!("expected tool result");
    };
    assert_eq!(content, "short output");
}

#[test]
fn no_pruning_when_savings_below_minimum() {
    // One old 10k-char result (~2.5k tokens saved) — below the 20k minimum.
    let mut messages = vec![tool_result_message(
        "call-old",
        "shell_exec",
        big_output(10_000),
    )];
    // 4 × 44k chars (~11k tokens each): the reserved 40k-token window is
    // crossed on the oldest of these, so everything before them is prunable.
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let report = prune_old_tool_outputs(&messages, 40_000);

    assert_eq!(report.pruned_results, 0);
    assert_eq!(report.estimated_tokens_saved, 0);
    assert_eq!(report.messages[0].content.text_length(), 10_000);
}

#[test]
fn everything_inside_reserved_window_is_protected() {
    let messages = vec![
        tool_result_message("call-1", "shell_exec", big_output(50_000)),
        tool_result_message("call-2", "shell_exec", big_output(50_000)),
    ];

    // Reserved window larger than the whole history: nothing prunable.
    let report = prune_old_tool_outputs(&messages, 1_000_000);

    assert_eq!(report.pruned_results, 0);
    assert_eq!(report.messages[0].content.text_length(), 50_000);
}

#[test]
fn plain_text_messages_are_untouched() {
    let mut messages = vec![
        Message::user(big_output(200_000)),
        Message::assistant(big_output(200_000)),
    ];
    // 4 × 44k chars (~11k tokens each): the reserved 40k-token window is
    // crossed on the oldest of these, so everything before them is prunable.
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let report = prune_old_tool_outputs(&messages, 40_000);

    assert_eq!(report.pruned_results, 0);
    assert_eq!(report.messages[0].content.text_length(), 200_000);
    assert_eq!(report.messages[1].content.text_length(), 200_000);
}

/// An old ask_user result keeps its full content: the user's words are not
/// re-derivable by rerunning a tool.
#[test]
fn never_prune_tools_keep_full_content() {
    let mut messages = vec![
        tool_result_message("call-ask", "ask_user", big_output(30_000)),
        tool_result_message("call-old", "shell_exec", big_output(200_000)),
    ];
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let report = prune_old_tool_outputs(&messages, 40_000);

    assert_eq!(report.pruned_results, 1);
    // The ask_user result is intact, the shell_exec one was pruned.
    assert_eq!(report.messages[0].content.text_length(), 30_000);
    assert!(report.messages[1].content.text_length() < 1_000);
}

/// Legacy tool results with an empty tool_name are resolved through the
/// matching ToolUse block before checking the protected list.
#[test]
fn legacy_tool_results_resolve_name_via_tool_use_id() {
    let skill_use = Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: "call-skill".to_string(),
            name: "skill_execute".to_string(),
            input: serde_json::json!({}),
            provider_metadata: None,
        }]),
    };
    let mut messages = vec![
        skill_use,
        tool_result_message("call-skill", "", big_output(30_000)),
        tool_result_message("call-old", "shell_exec", big_output(200_000)),
    ];
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let report = prune_old_tool_outputs(&messages, 40_000);

    assert_eq!(report.pruned_results, 1);
    // The legacy skill_execute result is intact despite its empty tool_name.
    assert_eq!(report.messages[1].content.text_length(), 30_000);
    assert!(report.messages[2].content.text_length() < 1_000);
}

/// A placeholder from a previous pruning pass is small enough to never be
/// pruned again — the pass is idempotent.
#[test]
fn pruning_is_idempotent() {
    let mut messages = vec![tool_result_message(
        "call-old",
        "shell_exec",
        big_output(200_000),
    )];
    // 4 × 44k chars (~11k tokens each): the reserved 40k-token window is
    // crossed on the oldest of these, so everything before them is prunable.
    for i in 0..4 {
        messages.push(tool_result_message(
            &format!("call-recent-{i}"),
            "file_read",
            big_output(44_000),
        ));
    }

    let first = prune_old_tool_outputs(&messages, 40_000);
    assert_eq!(first.pruned_results, 1);

    let second = prune_old_tool_outputs(&first.messages, 40_000);
    assert_eq!(second.pruned_results, 0);
    assert_eq!(
        second.messages[0].content.text_content(),
        first.messages[0].content.text_content()
    );
}
