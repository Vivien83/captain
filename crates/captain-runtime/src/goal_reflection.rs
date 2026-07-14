//! R.2.2 — Goal reflection job.
//!
//! Hourly cron that, for every Active goal, asks a cheap LLM (Haiku /
//! Kimi / whatever the user wires as `learning.reflection_model`) to
//! review the recent check history and propose 0..3 adjustments
//! (interval, threshold, recovery command). Each proposal lands as a
//! Pending [`captain-kernel::goals::Suggestion`] the user can apply
//! via the `goal_apply_suggestion` tool — the job never mutates a
//! goal directly.
//!
//! Hard cap: every reflection consumes one slot in the goal's
//! `llm_call_log` sliding 1h window via `try_consume_llm_quota`. If
//! the goal has burned its hourly LLM budget the job logs a warning
//! and skips it (no runaway spend).
//!
//! Decoupling pattern mirrors `goal_loop`: the module declares a
//! narrow [`GoalReflectionOps`] trait so tests can plug a stub
//! without depending on the full kernel surface.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::kernel_handle::KernelHandle;
use crate::reflection_job::ReflectionCompleter;

/// Default cadence for the reflection cron (matches the plan).
pub const REFLECTION_INTERVAL_SECS: u64 = 3600;

/// Narrow operations the reflection job needs from the kernel.
#[async_trait]
pub trait GoalReflectionOps: Send + Sync {
    /// JSON array of every goal (need full state including
    /// recent_checks for prompt building).
    fn goal_list(&self) -> Result<String, String>;
    /// Return false if the goal has exhausted its `max_llm_calls_per_hour`
    /// budget and the reflection should be skipped entirely. Atomically
    /// reserves a slot when returning true.
    fn try_consume_llm_quota(&self, goal_id: &str) -> bool;
    /// Persist a freshly built suggestion JSON (already serialized so
    /// the runtime crate stays decoupled from the kernel `Suggestion`
    /// struct). The kernel-side impl deserializes and validates.
    fn add_suggestion_raw(&self, goal_id: &str, suggestion_json: &str) -> Result<(), String>;
}

/// Production adapter — wraps a `KernelHandle` so the reflection job
/// can stay decoupled from the full kernel surface.
pub struct KernelReflectionOps {
    pub kh: Arc<dyn KernelHandle>,
}

impl GoalReflectionOps for KernelReflectionOps {
    fn goal_list(&self) -> Result<String, String> {
        self.kh.goal_list()
    }
    fn try_consume_llm_quota(&self, goal_id: &str) -> bool {
        self.kh.goal_try_consume_llm_quota(goal_id)
    }
    fn add_suggestion_raw(&self, goal_id: &str, suggestion_json: &str) -> Result<(), String> {
        self.kh.goal_add_suggestion_raw(goal_id, suggestion_json)
    }
}

/// Build the (system, user) pair fed to the LLM for one goal. The
/// system prompt locks the model to a strict JSON schema so the
/// downstream parser stays trivial.
pub fn build_reflection_prompt(goal: &serde_json::Value) -> (String, String) {
    let system = "You are an autopilot reviewer. Read ONE goal's recent \
check history and propose 0..3 adjustments to its configuration. \n\n\
RULES:\n\
1. Output ONLY a JSON array. No prose, no markdown, no code fences.\n\
2. Each element is one of:\n\
   {\"kind\":\"adjust_interval\",\"new_secs\":N,\"reason\":\"...\"}\n\
   {\"kind\":\"adjust_threshold\",\"new_value\":N,\"reason\":\"...\"}\n\
   {\"kind\":\"enable_recovery\",\"command\":\"...\",\"reason\":\"...\"}\n\
   {\"kind\":\"disable_recovery\",\"reason\":\"...\"}\n\
3. NEVER suggest interval_secs < 10. NEVER suggest threshold < 1.\n\
4. NEVER include destructive commands (rm -rf, dd, DROP DATABASE…).\n\
5. Reasons are one short sentence pointing to the data.\n\
6. If the goal looks healthy (no recent fails) return [].";

    // Trim recent_checks to the last 20 entries to keep the prompt
    // small — the LLM rarely benefits from more than that.
    let mut compact = goal.clone();
    if let Some(arr) = compact
        .get_mut("recent_checks")
        .and_then(|v| v.as_array_mut())
    {
        let len = arr.len();
        if len > 20 {
            let drop = len - 20;
            arr.drain(0..drop);
        }
    }
    let user = format!(
        "Review this goal and propose adjustments if any.\n\nGOAL:\n{}\n",
        serde_json::to_string_pretty(&compact).unwrap_or_default()
    );
    (system.to_string(), user)
}

/// Parse the LLM raw output into a vector of suggestion JSON objects.
/// Robust to model junk: skips invalid entries instead of failing,
/// strips optional ```json fences, and caps the result at 3 entries
/// (defense against a verbose model).
pub fn parse_suggestion_response(raw: &str) -> Vec<serde_json::Value> {
    let trimmed = raw.trim();
    // Strip optional ```json … ``` fences (some models add them
    // despite the rule against it).
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end_matches("```").trim())
        .unwrap_or(trimmed);

    let parsed: serde_json::Value = match serde_json::from_str(stripped) {
        Ok(v) => v,
        Err(e) => {
            warn!("reflection: failed to parse LLM output as JSON: {e}");
            return Vec::new();
        }
    };
    let arr = match parsed.as_array() {
        Some(a) => a,
        None => {
            warn!("reflection: LLM returned non-array, skipping");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for entry in arr.iter().take(3) {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(kind) = obj.get("kind").and_then(|k| k.as_str()) else {
            continue;
        };
        let reason = obj
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("(no reason)")
            .to_string();
        let suggestion = match kind {
            "adjust_interval" => {
                let n = obj.get("new_secs").and_then(|v| v.as_u64());
                match n {
                    Some(n) if n >= 10 => serde_json::json!({
                        "kind": "adjust_interval",
                        "new_secs": n,
                        "reason": reason,
                    }),
                    _ => continue,
                }
            }
            "adjust_threshold" => {
                let n = obj.get("new_value").and_then(|v| v.as_u64());
                match n {
                    Some(n) if n >= 1 => serde_json::json!({
                        "kind": "adjust_threshold",
                        "new_value": n,
                        "reason": reason,
                    }),
                    _ => continue,
                }
            }
            "enable_recovery" => {
                let cmd = obj.get("command").and_then(|v| v.as_str());
                match cmd {
                    Some(c) if !c.is_empty() => serde_json::json!({
                        "kind": "enable_recovery",
                        "command": c,
                        "reason": reason,
                    }),
                    _ => continue,
                }
            }
            "disable_recovery" => serde_json::json!({
                "kind": "disable_recovery",
                "reason": reason,
            }),
            _ => continue,
        };
        out.push(suggestion);
    }
    out
}

/// Reflect on every Active goal in turn. Skips paused/escalated and
/// any goal that has burned its hourly LLM budget.
pub async fn reflect_all_goals(
    ops: Arc<dyn GoalReflectionOps>,
    completer: Arc<dyn ReflectionCompleter>,
    model: String,
) {
    let raw = match ops.goal_list() {
        Ok(j) => j,
        Err(e) => {
            warn!("reflection: cannot list goals: {e}");
            return;
        }
    };
    let goals: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    let mut reviewed = 0usize;
    let mut skipped_quota = 0usize;
    for g in goals {
        let status = g.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if status != "active" {
            continue;
        }
        let id = g
            .get("id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }

        // R.2.1 hard cap — refuse to call the LLM if the goal has
        // exhausted its hourly budget.
        if !ops.try_consume_llm_quota(&id) {
            skipped_quota += 1;
            continue;
        }

        let (system, user) = build_reflection_prompt(&g);
        let raw = match completer.complete(&model, &system, &user).await {
            Ok(s) => s,
            Err(e) => {
                warn!(goal = %id, "reflection LLM call failed: {e}");
                continue;
            }
        };
        let suggestions = parse_suggestion_response(&raw);
        if suggestions.is_empty() {
            debug!(goal = %id, "reflection produced no suggestions");
        }
        for s in suggestions {
            let json = match serde_json::to_string(&s) {
                Ok(j) => j,
                Err(e) => {
                    warn!(goal = %id, "serialize suggestion: {e}");
                    continue;
                }
            };
            if let Err(e) = ops.add_suggestion_raw(&id, &json) {
                warn!(goal = %id, "add_suggestion failed: {e}");
            }
        }
        reviewed += 1;
    }
    if reviewed > 0 || skipped_quota > 0 {
        info!(reviewed, skipped_quota, "reflection job pass complete");
    }
}

/// Spawn the recurring reflection cron. Calls `reflect_all_goals`
/// every `REFLECTION_INTERVAL_SECS`. Spawned once per daemon at boot.
pub fn spawn_reflection_cron(
    ops: Arc<dyn GoalReflectionOps>,
    completer: Arc<dyn ReflectionCompleter>,
    model: String,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(interval_secs.max(60)));
        // Skip the immediate-fire tick so we don't reflect at boot.
        tick.tick().await;
        info!(
            interval_secs,
            "Goal reflection cron started (model={model})"
        );
        loop {
            tick.tick().await;
            reflect_all_goals(ops.clone(), completer.clone(), model.clone()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Stub ops that records suggestions added per goal.
    struct StubOps {
        goals_json: String,
        added: Mutex<Vec<(String, String)>>,
        quota_remaining: Mutex<u32>,
    }

    impl StubOps {
        fn new(goals_json: &str, quota: u32) -> Arc<Self> {
            Arc::new(Self {
                goals_json: goals_json.to_string(),
                added: Mutex::new(Vec::new()),
                quota_remaining: Mutex::new(quota),
            })
        }
    }

    #[async_trait]
    impl GoalReflectionOps for StubOps {
        fn goal_list(&self) -> Result<String, String> {
            Ok(self.goals_json.clone())
        }
        fn try_consume_llm_quota(&self, _goal_id: &str) -> bool {
            let mut q = self.quota_remaining.lock().unwrap();
            if *q == 0 {
                false
            } else {
                *q -= 1;
                true
            }
        }
        fn add_suggestion_raw(&self, goal_id: &str, suggestion_json: &str) -> Result<(), String> {
            self.added
                .lock()
                .unwrap()
                .push((goal_id.to_string(), suggestion_json.to_string()));
            Ok(())
        }
    }

    /// Static completer that always returns the same canned response.
    struct CannedCompleter(String);
    #[async_trait]
    impl ReflectionCompleter for CannedCompleter {
        async fn complete(
            &self,
            _model: &str,
            _system: &str,
            _user: &str,
        ) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn parse_suggestion_response_accepts_well_formed_array() {
        let raw = r#"[
          {"kind":"adjust_interval","new_secs":120,"reason":"slower polling fine"},
          {"kind":"adjust_threshold","new_value":5,"reason":"more tolerant"},
          {"kind":"disable_recovery","reason":"recovery makes it worse"}
        ]"#;
        let out = parse_suggestion_response(raw);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn parse_suggestion_response_strips_json_fences() {
        let raw = "```json\n[{\"kind\":\"disable_recovery\",\"reason\":\"x\"}]\n```";
        let out = parse_suggestion_response(raw);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parse_suggestion_response_rejects_too_short_interval() {
        let raw = r#"[{"kind":"adjust_interval","new_secs":1,"reason":"too aggressive"}]"#;
        let out = parse_suggestion_response(raw);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn parse_suggestion_response_rejects_zero_threshold() {
        let raw = r#"[{"kind":"adjust_threshold","new_value":0,"reason":"x"}]"#;
        let out = parse_suggestion_response(raw);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn parse_suggestion_response_rejects_unknown_kind() {
        let raw = r#"[{"kind":"delete_goal","reason":"yolo"}]"#;
        let out = parse_suggestion_response(raw);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn parse_suggestion_response_caps_at_three_entries() {
        let raw = r#"[
          {"kind":"disable_recovery","reason":"a"},
          {"kind":"disable_recovery","reason":"b"},
          {"kind":"disable_recovery","reason":"c"},
          {"kind":"disable_recovery","reason":"d"},
          {"kind":"disable_recovery","reason":"e"}
        ]"#;
        let out = parse_suggestion_response(raw);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn parse_suggestion_response_handles_garbage_gracefully() {
        assert_eq!(parse_suggestion_response("not json").len(), 0);
        assert_eq!(parse_suggestion_response("{}").len(), 0); // not an array
        assert_eq!(parse_suggestion_response("").len(), 0);
    }

    #[test]
    fn build_reflection_prompt_trims_history_to_20() {
        let mut checks = Vec::new();
        for i in 0..50 {
            checks.push(serde_json::json!({"ts": i, "ok": true}));
        }
        let goal = serde_json::json!({
            "id": "g",
            "recent_checks": checks,
        });
        let (_sys, user) = build_reflection_prompt(&goal);
        // Crude check: serialize the user prompt and confirm only 20
        // checks made it through.
        let count = user.matches("\"ts\":").count();
        assert_eq!(count, 20);
    }

    #[tokio::test]
    async fn reflect_all_goals_skips_paused() {
        let goals = r#"[
          {"id":"g1","status":"paused","recent_checks":[]}
        ]"#;
        let ops = StubOps::new(goals, 10);
        let dyn_ops: Arc<dyn GoalReflectionOps> = ops.clone();
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(CannedCompleter("[]".into()));
        reflect_all_goals(dyn_ops, completer, "m".into()).await;
        assert!(ops.added.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reflect_all_goals_skips_when_quota_exhausted() {
        let goals = r#"[
          {"id":"g1","status":"active","recent_checks":[]}
        ]"#;
        let ops = StubOps::new(goals, 0); // no quota
        let dyn_ops: Arc<dyn GoalReflectionOps> = ops.clone();
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(CannedCompleter(
            r#"[{"kind":"disable_recovery","reason":"x"}]"#.into(),
        ));
        reflect_all_goals(dyn_ops, completer, "m".into()).await;
        // Even though the LLM would have suggested something, no
        // suggestion is added because the quota guard fired.
        assert!(ops.added.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reflect_all_goals_persists_suggestions_for_active_goal() {
        let goals = r#"[
          {"id":"g1","status":"active","recent_checks":[
            {"ts":"2026-04-27T00:00:00Z","ok":false,"output":"e","latency_ms":1}
          ]}
        ]"#;
        let ops = StubOps::new(goals, 10);
        let dyn_ops: Arc<dyn GoalReflectionOps> = ops.clone();
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(CannedCompleter(
            r#"[{"kind":"adjust_interval","new_secs":120,"reason":"slow it down"}]"#.into(),
        ));
        reflect_all_goals(dyn_ops, completer, "m".into()).await;
        let added = ops.added.lock().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].0, "g1");
        assert!(added[0].1.contains("adjust_interval"));
        assert!(added[0].1.contains("120"));
    }
}
