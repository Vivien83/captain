use serde_json::Value;

pub(super) fn select_priority_recent_workers<'a>(
    workers: &[&'a Value],
    limit: usize,
) -> Vec<&'a Value> {
    if workers.len() <= limit {
        return workers.to_vec();
    }
    let mut selected = workers
        .iter()
        .enumerate()
        .filter_map(|(index, worker)| worker_is_actionable(worker).then_some(index))
        .collect::<Vec<_>>();
    if selected.len() > limit {
        selected.drain(0..(selected.len() - limit));
    }
    let remaining = limit.saturating_sub(selected.len());
    if remaining > 0 {
        for index in (0..workers.len()).rev() {
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
    selected.into_iter().map(|index| workers[index]).collect()
}

fn worker_is_actionable(worker: &Value) -> bool {
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
    fn selection_keeps_actionable_and_recent_tail() {
        let workers = vec![
            json!({"id": "old-running", "status": "running"}),
            json!({"id": "old-done", "status": "done"}),
            json!({"id": "mid-done", "status": "done"}),
            json!({"id": "recent-done", "status": "done"}),
            json!({"id": "recent-failed", "status": "failed"}),
        ];
        let refs = workers.iter().collect::<Vec<_>>();

        let selected = select_priority_recent_workers(&refs, 3);
        let ids = selected
            .into_iter()
            .filter_map(|worker| worker["id"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["old-running", "recent-done", "recent-failed"]);
    }

    #[test]
    fn selection_with_zero_limit_returns_empty() {
        let workers = vec![json!({"id": "running", "status": "running"})];
        let refs = workers.iter().collect::<Vec<_>>();

        assert!(select_priority_recent_workers(&refs, 0).is_empty());
    }
}
