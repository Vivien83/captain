use super::*;

#[test]
fn test_budget_defaults() {
    let budget = ContextBudget::default();
    assert_eq!(budget.context_window_tokens, 200_000);
    assert_eq!(budget.per_result_cap(), 120_000);
}

#[test]
fn test_codex_economy_caps_large_context_tool_replay() {
    let budget = ContextBudget::codex_economy(200_000);
    assert_eq!(budget.per_result_cap(), 8_000);
    assert_eq!(budget.single_result_max(), 10_000);
    assert_eq!(budget.total_tool_headroom_chars(), 6_000);
    assert_eq!(budget.cold_tool_result_cap(), 800);
}

#[test]
fn test_small_model_budget() {
    let budget = ContextBudget::new(8_000);
    assert_eq!(budget.per_result_cap(), 4_800);
}

#[test]
fn test_truncate_within_limit() {
    let budget = ContextBudget::default();
    let short = "Hello world";
    assert_eq!(truncate_tool_result_dynamic(short, &budget), short);
}

#[test]
fn test_truncate_breaks_at_newline() {
    let budget = ContextBudget::new(100);
    let content =
        "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11\nline12";
    let result = truncate_tool_result_dynamic(content, &budget);
    assert!(result.contains("[TRUNCATED:"));
    assert!(result.starts_with("line1\n") || result.is_empty() || result.contains("[TRUNCATED:"));
}

#[test]
fn test_context_guard_no_compaction_needed() {
    let budget = ContextBudget::default();
    let mut messages = vec![Message::user("hello")];
    let compacted = apply_context_guard(&mut messages, &budget, &[]);
    assert_eq!(compacted, 0);
}

#[test]
fn test_collect_tool_result_locations_preserves_order_and_total() {
    let messages = vec![
        Message::user("hello"),
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: "not a tool".to_string(),
                    provider_metadata: None,
                },
                ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    tool_name: "shell_exec".to_string(),
                    content: "abc".to_string(),
                    is_error: false,
                },
            ]),
        },
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t2".to_string(),
                tool_name: "web_research".to_string(),
                content: "12345".to_string(),
                is_error: false,
            }]),
        },
    ];

    let (locations, total_chars) = collect_tool_result_locations(&messages);

    assert_eq!(total_chars, 8);
    assert_eq!(locations.len(), 2);
    assert_eq!(locations[0].msg_idx, 1);
    assert_eq!(locations[0].block_idx, 1);
    assert_eq!(locations[0].char_len, 3);
    assert_eq!(locations[1].msg_idx, 2);
    assert_eq!(locations[1].block_idx, 0);
    assert_eq!(locations[1].char_len, 5);
}

#[test]
fn test_context_guard_compacts_oldest() {
    let budget = ContextBudget::new(100);
    let big_result = "x".repeat(500);
    let mut messages = vec![
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                tool_name: String::new(),
                content: big_result.clone(),
                is_error: false,
            }]),
        },
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t2".to_string(),
                tool_name: String::new(),
                content: big_result,
                is_error: false,
            }]),
        },
    ];

    let compacted = apply_context_guard(&mut messages, &budget, &[]);
    assert!(compacted > 0);

    if let MessageContent::Blocks(blocks) = &messages[0].content {
        if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert!(content.len() < 500);
        }
    }
}

#[test]
fn test_codex_context_guard_compacts_cold_replay_but_preserves_current_result() {
    let budget = ContextBudget::codex_economy(200_000);
    let cold = "cold-result\n".repeat(220);
    let current = "current-result\n".repeat(360);
    let mut messages = vec![
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "old-1".to_string(),
                tool_name: "captain_docs".to_string(),
                content: cold.clone(),
                is_error: false,
            }]),
        },
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "old-2".to_string(),
                tool_name: "capability_search".to_string(),
                content: cold,
                is_error: false,
            }]),
        },
        Message {
            role: captain_types::message::Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "current".to_string(),
                tool_name: "web_research_batch".to_string(),
                content: current.clone(),
                is_error: false,
            }]),
        },
    ];

    let compacted = apply_context_guard_preserving_recent(&mut messages, &budget, &[], 1);
    assert_eq!(compacted, 2);

    for msg in messages.iter().take(2) {
        if let MessageContent::Blocks(blocks) = &msg.content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(content.len() <= budget.cold_tool_result_cap() + 120);
            }
        }
    }
    if let MessageContent::Blocks(blocks) = &messages[2].content {
        if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert_eq!(content, &current);
        }
    }
}

#[test]
fn test_codex_context_guard_compacts_single_stale_result_when_not_protected() {
    let budget = ContextBudget::codex_economy(200_000);
    let stale = "stale-result\n".repeat(700);
    let mut messages = vec![Message {
        role: captain_types::message::Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "old".to_string(),
            tool_name: "captain_docs".to_string(),
            content: stale,
            is_error: false,
        }]),
    }];

    let compacted = apply_context_guard_preserving_recent(&mut messages, &budget, &[], 0);
    assert_eq!(compacted, 1);
    if let MessageContent::Blocks(blocks) = &messages[0].content {
        if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert!(content.len() <= budget.cold_tool_result_cap() + 120);
        }
    }
}

#[test]
fn test_truncate_tool_result_multibyte_chinese() {
    let budget = ContextBudget::new(100);
    let content: String = "\u{4f60}\u{597d}\u{4e16}\u{754c}".repeat(25);
    assert_eq!(content.len(), 300);
    let result = truncate_tool_result_dynamic(&content, &budget);
    assert!(result.contains("[TRUNCATED:"));
    assert!(result.is_char_boundary(0));
}

#[test]
fn test_context_guard_multibyte_tool_results() {
    let budget = ContextBudget::new(100);
    let big_chinese: String = "\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}\u{6570}\u{636e}".repeat(83);
    let mut messages = vec![Message {
        role: captain_types::message::Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "t1".to_string(),
            tool_name: String::new(),
            content: big_chinese,
            is_error: false,
        }]),
    }];
    let compacted = apply_context_guard(&mut messages, &budget, &[]);
    assert!(compacted > 0);
}
