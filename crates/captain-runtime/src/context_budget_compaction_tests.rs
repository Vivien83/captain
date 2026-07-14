use super::*;

#[test]
fn test_truncate_to_multibyte_emoji() {
    let content: String = "\u{1f600}".repeat(200);
    let result = truncate_to(&content, 100);
    assert!(result.contains("[COMPACTED:"));
    assert!(result.is_char_boundary(0));
}

#[test]
fn test_compact_tool_result_preserves_signal_and_reduces_noise() {
    let budget = ContextBudget::new(16_000);
    let mut content = String::new();
    for _ in 0..300 {
        content.push_str("progress: downloading chunk\n");
    }
    content.push_str("ERROR duplicate entry for key unique_commande\n");
    for _ in 0..300 {
        content.push_str("progress: downloading chunk\n");
    }

    let compacted = compact_tool_result_for_context("ssh_exec", &content, true, &budget);

    assert!(compacted.len() < content.len());
    assert!(compacted.contains("CAPTAIN CONTEXT ECONOMY"));
    assert!(compacted.contains("duplicate entry"));
}
