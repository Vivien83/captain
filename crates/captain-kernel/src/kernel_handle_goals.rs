use crate::goals::{CheckResult, Goal, GoalStatus, Suggestion};
use std::collections::VecDeque;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_goal_create(&self, goal_json: &str) -> Result<String, String> {
        let goal = goal_from_json(goal_json)?;
        let id = goal.id.clone();
        self.goal_store.add(goal).map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub(super) fn handle_goal_list(&self) -> Result<String, String> {
        let goals = self.goal_store.list();
        serde_json::to_string_pretty(&goals).map_err(|e| format!("serialize: {e}"))
    }

    pub(super) fn handle_goal_pause(&self, id: &str) -> Result<bool, String> {
        self.goal_store
            .set_status(id, GoalStatus::Paused)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_goal_resume(&self, id: &str) -> Result<bool, String> {
        self.goal_store
            .set_status(id, GoalStatus::Active)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_goal_status(&self, id: &str) -> Result<String, String> {
        match self.goal_store.get(id) {
            Some(g) => {
                let mut v = serde_json::to_value(&g).map_err(|e| format!("serialize: {e}"))?;
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "llm_calls_last_hour".into(),
                        serde_json::json!(self.goal_store.llm_calls_last_hour(id)),
                    );
                }
                serde_json::to_string_pretty(&v).map_err(|e| format!("serialize: {e}"))
            }
            None => Err(format!("Goal '{id}' not found")),
        }
    }

    pub(super) fn handle_goal_delete(&self, id: &str) -> Result<bool, String> {
        Ok(self
            .goal_store
            .remove(id)
            .map_err(|e| e.to_string())?
            .is_some())
    }

    pub(super) fn handle_goal_record_check(
        &self,
        id: &str,
        ok: bool,
        output: &str,
        latency_ms: u64,
    ) -> Result<u32, String> {
        let result = CheckResult::new(ok, output.to_string(), latency_ms);
        self.goal_store
            .record_check(id, result)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_goal_mark_escalated(&self, id: &str) -> Result<bool, String> {
        self.goal_store
            .set_status(id, GoalStatus::Escalated)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_goal_try_consume_llm_quota(&self, id: &str) -> bool {
        self.goal_store.try_consume_llm_quota(id)
    }

    pub(super) fn handle_goal_list_suggestions(&self, id: &str) -> Result<String, String> {
        let list = self.goal_store.list_suggestions(id);
        serde_json::to_string_pretty(&list).map_err(|e| format!("serialize: {e}"))
    }

    pub(super) fn handle_goal_add_suggestion_raw(
        &self,
        id: &str,
        suggestion_json: &str,
    ) -> Result<(), String> {
        let suggestion: Suggestion = serde_json::from_str(suggestion_json)
            .map_err(|e| format!("invalid suggestion JSON: {e}"))?;
        self.goal_store
            .add_suggestion(id, suggestion)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_goal_apply_suggestion(
        &self,
        id: &str,
        suggestion_id: &str,
    ) -> Result<bool, String> {
        self.goal_store
            .apply_suggestion(id, suggestion_id)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_goal_reject_suggestion(
        &self,
        id: &str,
        suggestion_id: &str,
    ) -> Result<bool, String> {
        self.goal_store
            .reject_suggestion(id, suggestion_id)
            .map_err(|e| e.to_string())
    }
}

fn goal_from_json(goal_json: &str) -> Result<Goal, String> {
    // Parse the LLM-supplied JSON into a fully-formed Goal, applying
    // defaults that the tool schema marks as optional so the autopilot
    // always has the required fields.
    let value: serde_json::Value =
        serde_json::from_str(goal_json).map_err(|e| format!("invalid goal JSON: {e}"))?;
    let now = chrono::Utc::now();
    Ok(Goal {
        id: value
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        name: value
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        description: value
            .get("description")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        project_id: value
            .get("project_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        project_slug: value
            .get("project_slug")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        status: GoalStatus::Active,
        interval_secs: value
            .get("interval_secs")
            .and_then(|x| x.as_u64())
            .unwrap_or(300),
        check_command: value
            .get("check_command")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        recovery_command: value
            .get("recovery_command")
            .and_then(|x| x.as_str())
            .map(String::from),
        escalation_threshold: value
            .get("escalation_threshold")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32)
            .unwrap_or(3),
        max_llm_calls_per_hour: value
            .get("max_llm_calls_per_hour")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32)
            .unwrap_or(20),
        escalation_channel: value
            .get("escalation_channel")
            .and_then(|c| serde_json::from_value(c.clone()).ok()),
        created_at: now,
        updated_at: now,
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks: VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_from_json_applies_runtime_defaults() {
        let goal = goal_from_json(
            r#"{
                "id": "uptime",
                "name": "Uptime",
                "description": "Keep service alive",
                "check_command": "curl -f https://example.com"
            }"#,
        )
        .unwrap();

        assert_eq!(goal.id, "uptime");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.interval_secs, 300);
        assert_eq!(goal.escalation_threshold, 3);
        assert_eq!(goal.max_llm_calls_per_hour, 20);
        assert!(goal.recent_checks.is_empty());
        assert!(goal.llm_call_log.is_empty());
    }

    #[test]
    fn goal_from_json_trims_empty_project_fields() {
        let goal = goal_from_json(
            r#"{
                "id": "project-goal",
                "project_id": "  ",
                "project_slug": "  launch  "
            }"#,
        )
        .unwrap();

        assert_eq!(goal.project_id, None);
        assert_eq!(goal.project_slug.as_deref(), Some("launch"));
    }

    #[test]
    fn goal_from_json_preserves_explicit_limits() {
        let goal = goal_from_json(
            r#"{
                "id": "quota",
                "interval_secs": 60,
                "escalation_threshold": 5,
                "max_llm_calls_per_hour": 8
            }"#,
        )
        .unwrap();

        assert_eq!(goal.interval_secs, 60);
        assert_eq!(goal.escalation_threshold, 5);
        assert_eq!(goal.max_llm_calls_per_hour, 8);
    }
}
