//! Ping-pong pattern detection for tool loop guard history.

use std::collections::HashMap;

/// Detect ping-pong patterns (A-B-A-B or A-B-C-A-B-C) in recent call history.
///
/// Checks if the last 6+ calls form a repeating pattern of length 2 or 3.
/// Returns a warning message if a pattern is detected, `None` otherwise.
pub(crate) fn detect_ping_pong(
    recent_calls: &[String],
    hash_to_tool: &HashMap<String, String>,
) -> Option<String> {
    let len = recent_calls.len();

    if len >= 6 {
        let tail = &recent_calls[len - 6..];
        let a = &tail[0];
        let b = &tail[1];
        if a != b && tail[2] == *a && tail[3] == *b && tail[4] == *a && tail[5] == *b {
            let tool_a = tool_name(hash_to_tool, a);
            let tool_b = tool_name(hash_to_tool, b);
            return Some(format!(
                "Ping-pong detected: tools '{}' and '{}' are alternating \
                 repeatedly. Break the cycle by trying a different approach.",
                tool_a, tool_b
            ));
        }
    }

    if len >= 9 {
        let tail = &recent_calls[len - 9..];
        let a = &tail[0];
        let b = &tail[1];
        let c = &tail[2];
        if !(a == b && b == c)
            && tail[3] == *a
            && tail[4] == *b
            && tail[5] == *c
            && tail[6] == *a
            && tail[7] == *b
            && tail[8] == *c
        {
            let tool_a = tool_name(hash_to_tool, a);
            let tool_b = tool_name(hash_to_tool, b);
            let tool_c = tool_name(hash_to_tool, c);
            return Some(format!(
                "Ping-pong detected: tools '{}', '{}', '{}' are cycling \
                 repeatedly. Break the cycle by trying a different approach.",
                tool_a, tool_b, tool_c
            ));
        }
    }

    None
}

/// Count how many full repeats of the detected ping-pong pattern exist
/// in the recent call history.
pub(crate) fn count_ping_pong_repeats(recent_calls: &[String]) -> u32 {
    let len = recent_calls.len();

    if len >= 4 {
        let a = &recent_calls[len - 2];
        let b = &recent_calls[len - 1];
        if a != b {
            let mut repeats: u32 = 0;
            let mut i = len;
            while i >= 2 {
                i -= 2;
                if recent_calls[i] == *a && recent_calls[i + 1] == *b {
                    repeats += 1;
                } else {
                    break;
                }
            }
            if repeats >= 2 {
                return repeats;
            }
        }
    }

    if len >= 6 {
        let a = &recent_calls[len - 3];
        let b = &recent_calls[len - 2];
        let c = &recent_calls[len - 1];
        if !(a == b && b == c) {
            let mut repeats: u32 = 0;
            let mut i = len;
            while i >= 3 {
                i -= 3;
                if recent_calls[i] == *a && recent_calls[i + 1] == *b && recent_calls[i + 2] == *c {
                    repeats += 1;
                } else {
                    break;
                }
            }
            if repeats >= 2 {
                return repeats;
            }
        }
    }

    0
}

fn tool_name(hash_to_tool: &HashMap<String, String>, hash: &str) -> String {
    hash_to_tool
        .get(hash)
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}
