//! Message boundary selection for session compaction.

use captain_types::message::{ContentBlock, Message, MessageContent, Role};

pub(crate) fn coherent_recent_split(messages: &[Message], keep_recent: usize) -> usize {
    let msg_count = messages.len();
    let split_at = msg_count.saturating_sub(keep_recent);
    if split_at == 0 || split_at >= msg_count {
        return split_at;
    }

    let split_at = align_to_recent_user_boundary(messages, split_at, keep_recent);
    protect_latest_user_message(messages, split_at)
}

fn align_to_recent_user_boundary(
    messages: &[Message],
    split_at: usize,
    keep_recent: usize,
) -> usize {
    if is_human_user_boundary(&messages[split_at]) {
        return split_at;
    }

    let search_window = keep_recent.saturating_mul(3).max(keep_recent + 8);
    let search_floor = split_at.saturating_sub(search_window);
    for idx in (search_floor..split_at).rev() {
        if is_human_user_boundary(&messages[idx]) {
            return idx;
        }
    }

    split_at
}

fn protect_latest_user_message(messages: &[Message], split_at: usize) -> usize {
    let Some(last_user_idx) = latest_human_user_index(messages) else {
        return split_at;
    };
    if last_user_idx < split_at {
        last_user_idx
    } else {
        split_at
    }
}

fn latest_human_user_index(messages: &[Message]) -> Option<usize> {
    messages.iter().rposition(is_human_user_boundary)
}

pub(crate) fn is_human_user_boundary(msg: &Message) -> bool {
    if msg.role != Role::User {
        return false;
    }
    match &msg.content {
        MessageContent::Text(text) => !text.trim().is_empty(),
        MessageContent::Blocks(blocks) => blocks.iter().any(
            |block| matches!(block, ContentBlock::Text { text, .. } if !text.trim().is_empty()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_latest_user_even_when_far_before_recent_tail() {
        let mut messages = Vec::new();
        for idx in 0..20 {
            messages.push(Message::user(format!("old request {idx}")));
            messages.push(Message::assistant(format!("old answer {idx}")));
        }
        let active_idx = messages.len();
        messages.push(Message::user("continue the release audit"));
        for idx in 0..30 {
            messages.push(Message::assistant(format!("long tool/report output {idx}")));
        }

        let split_at = coherent_recent_split(&messages, 3);

        assert_eq!(split_at, active_idx);
        assert_eq!(
            messages[split_at].content.text_content(),
            "continue the release audit"
        );
    }

    #[test]
    fn tool_results_do_not_count_as_human_user_boundary() {
        let messages = vec![
            Message::assistant("setup"),
            Message::user("fix the failing cron job"),
            Message::assistant("running tool"),
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "call-1".to_string(),
                    tool_name: "shell_exec".to_string(),
                    content: "ok".to_string(),
                    is_error: false,
                }]),
            },
            Message::assistant("diagnostic output"),
            Message::assistant("more diagnostic output"),
        ];

        let split_at = coherent_recent_split(&messages, 1);

        assert_eq!(split_at, 1);
        assert_eq!(
            messages[split_at].content.text_content(),
            "fix the failing cron job"
        );
    }
}
