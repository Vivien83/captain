use crate::context_budget::{truncate_tool_result_dynamic, ContextBudget};

#[test]
fn dynamic_truncate_short_unchanged() {
    let budget = ContextBudget::new(200_000);
    let short = "Hello, world!";
    assert_eq!(truncate_tool_result_dynamic(short, &budget), short);
}

#[test]
fn dynamic_truncate_over_limit() {
    let budget = ContextBudget::new(200_000);
    let long = "x".repeat(budget.per_result_cap() + 10_000);
    let result = truncate_tool_result_dynamic(&long, &budget);
    assert!(result.len() <= budget.per_result_cap() + 200);
    assert!(result.contains("[TRUNCATED:"));
}

#[test]
fn dynamic_truncate_newline_boundary() {
    let budget = ContextBudget::new(1_000);
    let content = (0..200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = truncate_tool_result_dynamic(&content, &budget);
    let before_marker = result.split("[TRUNCATED:").next().unwrap();
    let trimmed = before_marker.trim_end();
    assert!(!trimmed.is_empty());
}
