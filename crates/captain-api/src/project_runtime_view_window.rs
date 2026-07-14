use serde_json::Value;

pub(crate) fn select_priority_recent_items(
    items: &[Value],
    limit: usize,
    is_priority: fn(&Value) -> bool,
) -> Vec<&Value> {
    if items.len() <= limit {
        return items.iter().collect();
    }
    let mut selected = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| is_priority(item).then_some(index))
        .collect::<Vec<_>>();
    if selected.len() > limit {
        selected.drain(0..(selected.len() - limit));
    }
    let remaining = limit.saturating_sub(selected.len());
    if remaining > 0 {
        for index in (0..items.len()).rev() {
            if selected.contains(&index) {
                continue;
            }
            selected.push(index);
            if selected.len() == limit {
                break;
            }
        }
    }
    selected.sort_unstable();
    selected.into_iter().map(|index| &items[index]).collect()
}

pub(crate) fn is_pending_question(question: &Value) -> bool {
    question
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending")
        == "pending"
}

pub(crate) fn is_actionable_worker(worker: &Value) -> bool {
    matches!(
        worker
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("planned"),
        "running" | "blocked" | "failed" | "paused" | "waiting" | "cleaning"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn priority_recent_selection_keeps_priority_then_recent_tail() {
        let items = vec![
            json!({"id": "old-priority", "status": "pending"}),
            json!({"id": "old-done", "status": "done"}),
            json!({"id": "mid-done", "status": "done"}),
            json!({"id": "newer-done", "status": "done"}),
            json!({"id": "newest-done", "status": "done"}),
        ];

        let selected = select_priority_recent_items(&items, 3, is_pending_question);
        let ids = selected
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["old-priority", "newer-done", "newest-done"]);
    }

    #[test]
    fn priority_recent_selection_handles_zero_limit() {
        let items = vec![json!({"id": "pending", "status": "pending"})];

        let selected = select_priority_recent_items(&items, 0, is_pending_question);

        assert!(selected.is_empty());
    }
}
