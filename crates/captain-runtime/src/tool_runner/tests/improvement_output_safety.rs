use super::*;

struct ImprovementOutputSafetyKernel {
    local_path: String,
    raw_secret: String,
    memory: std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>,
    learning_limits: std::sync::Mutex<Vec<usize>>,
    proposal_limits: std::sync::Mutex<Vec<usize>>,
    proposal_decisions: std::sync::Mutex<Vec<(String, bool)>>,
    learning_decisions: std::sync::Mutex<Vec<(String, bool)>>,
}

#[async_trait::async_trait]
impl KernelHandle for ImprovementOutputSafetyKernel {
    async fn spawn_agent(
        &self,
        _manifest: &str,
        _parent: Option<&str>,
    ) -> Result<(String, String), String> {
        Err("stub".into())
    }

    async fn send_to_agent(&self, _id: &str, _msg: &str) -> Result<String, String> {
        Err("stub".into())
    }

    fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }

    fn kill_agent(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(self.memory.lock().unwrap().get(key).cloned())
    }

    fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        self.memory.lock().unwrap().insert(key.to_string(), value);
        Ok(())
    }

    fn learning_review_list(&self, limit: usize) -> Result<serde_json::Value, String> {
        self.learning_limits.lock().unwrap().push(limit);
        Ok(serde_json::json!([{
            "id": "learn-1",
            "agent_id": self.local_path,
            "outcome": self.raw_secret,
            "limit_seen": limit,
            "subject": format!("workflow seen in {}", self.local_path),
            "predicate": "should_not_store",
            "object": self.raw_secret,
            "source": format!("reviewed at {}", self.local_path),
            "written_write_id": self.raw_secret
        }]))
    }

    async fn learning_review_decide(
        &self,
        review_id: &str,
        approve: bool,
        _decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.learning_decisions
            .lock()
            .unwrap()
            .push((review_id.to_string(), approve));
        Ok(serde_json::json!({
            "status": if approve { "committed" } else { "denied" },
            "id": review_id,
            "object": format!("approved from {}", self.local_path),
            "debug": self.raw_secret
        }))
    }

    fn skill_proposal_list(&self, limit: usize) -> Result<serde_json::Value, String> {
        self.proposal_limits.lock().unwrap().push(limit);
        Ok(serde_json::json!([{
            "id": "proposal-1",
            "pattern_hash": self.raw_secret,
            "limit_seen": limit,
            "name": "path-safe-proposal",
            "description": format!("Generate a skill from {}", self.local_path),
            "trigger_hint": self.raw_secret,
            "tool_sequence": ["file_read", "file_write"],
            "source_agent_id": self.local_path,
            "origin_channel": self.raw_secret,
            "written_path": self.local_path
        }]))
    }

    async fn skill_proposal_decide(
        &self,
        proposal_id: &str,
        approve: bool,
        _decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.proposal_decisions
            .lock()
            .unwrap()
            .push((proposal_id.to_string(), approve));
        if !approve {
            return Ok(serde_json::json!({ "status": "denied", "id": proposal_id }));
        }
        Ok(serde_json::json!({
            "status": self.raw_secret,
            "id": self.local_path,
            "written_path": self.local_path,
            "generated_path": self.local_path,
            "metadata": {
                "path": self.local_path,
                "file_path": self.local_path
            },
            "description": format!("generated from {}", self.local_path),
            "debug": self.raw_secret
        }))
    }

    fn find_agents(&self, _q: &str) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }

    async fn task_post(
        &self,
        _t: &str,
        _d: &str,
        _a: Option<&str>,
        _c: Option<&str>,
    ) -> Result<String, String> {
        Err("stub".into())
    }

    async fn task_claim(&self, _id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(&self, _id: &str, _r: &str) -> Result<(), String> {
        Ok(())
    }
}

fn output_safety_kernel() -> (Arc<dyn KernelHandle>, String, String) {
    output_safety_kernel_with_memory(Vec::new())
}

fn output_safety_kernel_state(
    memory: Vec<(&'static str, serde_json::Value)>,
) -> (
    Arc<ImprovementOutputSafetyKernel>,
    Arc<dyn KernelHandle>,
    String,
    String,
) {
    let local_path = "/Users/example/.captain/skills/generated/path-safe.md".to_string();
    let raw_secret = "AIzaSyC3_abcdefghij1234567890ABCDEFGhij".to_string();
    let memory = memory
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    let state = Arc::new(ImprovementOutputSafetyKernel {
        local_path: local_path.clone(),
        raw_secret: raw_secret.clone(),
        memory: std::sync::Mutex::new(memory),
        learning_limits: std::sync::Mutex::new(Vec::new()),
        proposal_limits: std::sync::Mutex::new(Vec::new()),
        proposal_decisions: std::sync::Mutex::new(Vec::new()),
        learning_decisions: std::sync::Mutex::new(Vec::new()),
    });
    let kh: Arc<dyn KernelHandle> = state.clone();
    (state, kh, local_path, raw_secret)
}

fn output_safety_kernel_with_memory(
    memory: Vec<(&'static str, serde_json::Value)>,
) -> (Arc<dyn KernelHandle>, String, String) {
    let (_state, kh, local_path, raw_secret) = output_safety_kernel_state(memory);
    (kh, local_path, raw_secret)
}

fn assert_public_safe(output: &str, local_path: &str, raw_secret: &str) {
    assert!(!output.contains(local_path), "local path leaked: {output}");
    assert!(!output.contains(raw_secret), "secret leaked: {output}");
}

#[test]
fn learning_and_proposal_lists_are_public_safe() {
    let (kh, local_path, raw_secret) = output_safety_kernel();

    let learning = tool_learning_review_list(&serde_json::json!({"limit": 5}), Some(&kh))
        .expect("learning list should serialize");
    assert_public_safe(&learning, &local_path, &raw_secret);
    assert!(learning.contains("<local-path>"));
    assert!(learning.contains("<secret>"));
    assert!(!learning.contains("agent_id") && !learning.contains("written_write_id"));

    let proposals = tool_skill_proposal_list(&serde_json::json!({"limit": 5}), Some(&kh))
        .expect("proposal list should serialize");
    assert_public_safe(&proposals, &local_path, &raw_secret);
    assert!(proposals.contains("<local-path>"));
    assert!(proposals.contains("<secret>"));
    assert!(!proposals.contains("pattern_hash") && !proposals.contains("source_agent_id"));

    let review = tool_self_improvement_review(&serde_json::json!({"limit": 5}), Some(&kh))
        .expect("self improvement review should serialize");
    assert_public_safe(&review, &local_path, &raw_secret);
    assert!(review.contains("<local-path>"));
    assert!(review.contains("<secret>"));
}

#[test]
fn learning_and_proposal_list_limits_are_bounded() {
    let (state, kh, _local_path, _raw_secret) = output_safety_kernel_state(Vec::new());

    tool_learning_review_list(&serde_json::json!({"limit": 5000}), Some(&kh))
        .expect("learning list should serialize");
    assert_eq!(*state.learning_limits.lock().unwrap(), vec![50]);

    tool_skill_proposal_list(&serde_json::json!({"limit": 0}), Some(&kh))
        .expect("proposal list should serialize");

    tool_skill_proposal_list(&serde_json::json!({}), Some(&kh))
        .expect("default proposal list should serialize");
    assert_eq!(*state.proposal_limits.lock().unwrap(), vec![1, 50]);
}

#[tokio::test]
async fn decisions_are_public_safe_and_agent_skill_approval_is_blocked() {
    let (state, kh, local_path, raw_secret) = output_safety_kernel_state(Vec::new());

    let learning_err = tool_learning_review_decide(
        &serde_json::json!({"id": "learn-1", "approve": true}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect_err("tool calls must not self-approve learning review commits either");
    assert_public_safe(&learning_err, &local_path, &raw_secret);
    assert!(learning_err.contains("human/API/channel approval"));
    assert!(state.learning_decisions.lock().unwrap().is_empty());

    let learning = tool_learning_review_decide(
        &serde_json::json!({"id": "learn-1", "approve": false}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect("learning decision rejection should serialize");
    assert_public_safe(&learning, &local_path, &raw_secret);
    assert!(
        !learning.contains("<local-path>") && !learning.contains("<secret>"),
        "learning decision output should be a logical projection: {learning}"
    );
    let learning_json: serde_json::Value = serde_json::from_str(&learning).unwrap();
    assert_eq!(learning_json["status"], "denied");
    assert_eq!(learning_json["id"], "learn-1");
    assert_eq!(
        *state.learning_decisions.lock().unwrap(),
        vec![("learn-1".to_string(), false)]
    );

    let proposal_err = tool_skill_proposal_decide(
        &serde_json::json!({"id": "proposal", "approve": true}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect_err("tool calls must not approve generated skills");
    assert_public_safe(&proposal_err, &local_path, &raw_secret);
    assert!(proposal_err.contains("human/API/channel approval"));
    assert!(state.proposal_decisions.lock().unwrap().is_empty());

    let proposal = tool_skill_proposal_decide(
        &serde_json::json!({"id": "proposal", "approve": false}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect("skill proposal rejection should serialize");
    assert_public_safe(&proposal, &local_path, &raw_secret);
    assert!(!proposal.contains("written_path"));
    assert!(!proposal.contains("generated_path") && !proposal.contains("\"metadata\""));
    assert!(!proposal.contains("<local-path>") && !proposal.contains("<secret>"));
    let json: serde_json::Value = serde_json::from_str(&proposal).unwrap();
    assert_eq!(json["status"], "denied");
    assert_eq!(json["id"], "proposal-1");
    assert_eq!(
        *state.proposal_decisions.lock().unwrap(),
        vec![("proposal-1".to_string(), false)]
    );
}

#[tokio::test]
async fn skill_proposal_decide_prefix_scan_is_bounded() {
    let (state, kh, _local_path, _raw_secret) = output_safety_kernel_state(Vec::new());

    tool_skill_proposal_decide(
        &serde_json::json!({"id": "proposal", "approve": false}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect("skill proposal decision should serialize");

    assert_eq!(*state.proposal_limits.lock().unwrap(), vec![50]);
}

#[tokio::test]
async fn review_decision_ids_are_validated_without_echoing_raw_input() {
    let (kh, local_path, raw_secret) = output_safety_kernel();

    let learning_err = tool_learning_review_decide(
        &serde_json::json!({"id": local_path, "approve": true}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect_err("local path id should be rejected before kernel access");
    assert!(
        !learning_err.contains(&local_path),
        "local path echoed in error: {learning_err}"
    );

    let proposal_err = tool_skill_proposal_decide(
        &serde_json::json!({"id": raw_secret, "approve": true}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect_err("secret-looking id should be rejected before kernel access");
    assert!(
        !proposal_err.contains(&raw_secret),
        "secret echoed in error: {proposal_err}"
    );

    let system_bug_err = tool_system_bug_update(
        &serde_json::json!({"id": local_path, "status": "fixed"}),
        Some(&kh),
    )
    .expect_err("local path system bug id should be rejected before lookup");
    assert!(
        !system_bug_err.contains(&local_path),
        "local path echoed in error: {system_bug_err}"
    );

    let refinement_err = tool_skill_refinement_update(
        &serde_json::json!({"id": raw_secret, "status": "applied"}),
        Some(&kh),
    )
    .expect_err("secret-looking refinement id should be rejected before lookup");
    assert!(
        !refinement_err.contains(&raw_secret),
        "secret echoed in error: {refinement_err}"
    );
}

#[test]
fn legacy_system_bug_registry_outputs_are_public_safe() {
    let legacy_bugs = serde_json::json!([{
        "id": "bug-legacy-1",
        "title": "Legacy bug with private data",
        "description": "/Users/example/private/repro.log exposed a token",
        "category": "security",
        "severity": "high",
        "status": "open",
        "evidence": "AIzaSyC3_abcdefghij1234567890ABCDEFGhij",
        "suggested_fix": "Inspect /Users/example/private/fix.md",
        "source": "legacy_import",
        "created_at": "2026-05-18T00:00:00Z",
        "updated_at": "2026-05-18T00:00:00Z",
        "notes": [{
            "at": "2026-05-18T00:00:00Z",
            "note": "Seen in /Users/example/private/debug.log"
        }]
    }]);
    let (kh, local_path, raw_secret) =
        output_safety_kernel_with_memory(vec![(SYSTEM_BUGS_KEY, legacy_bugs)]);

    let listed = tool_system_bug_list(&serde_json::json!({"limit": 10}), Some(&kh))
        .expect("legacy system bug list should serialize");
    assert_public_safe(&listed, &local_path, &raw_secret);
    assert!(
        !listed.contains("/Users/example/"),
        "legacy path leaked in list: {listed}"
    );
    assert!(listed.contains("<local-path>"));
    assert!(listed.contains("<secret>"));

    let updated = tool_system_bug_update(
        &serde_json::json!({"id": "bug-legacy", "status": "investigating"}),
        Some(&kh),
    )
    .expect("legacy system bug update should serialize");
    assert_public_safe(&updated, &local_path, &raw_secret);
    assert!(
        !updated.contains("/Users/example/"),
        "legacy path leaked in update: {updated}"
    );
    assert!(updated.contains("<local-path>"));
    assert!(updated.contains("<secret>"));

    let stored = kh
        .memory_recall(SYSTEM_BUGS_KEY)
        .unwrap()
        .unwrap()
        .to_string();
    assert_public_safe(&stored, &local_path, &raw_secret);
    assert!(
        !stored.contains("/Users/example/"),
        "legacy path leaked in storage: {stored}"
    );
}

#[test]
fn legacy_skill_refinement_registry_outputs_are_public_safe() {
    let legacy_refinements = serde_json::json!([{
        "id": "refine-legacy-1",
        "skill": "legacy-refinement",
        "finding": "Workflow came from /Users/example/private/session.txt",
        "suggested_change": "Never embed AIzaSyC3_abcdefghij1234567890ABCDEFGhij",
        "evidence": "Trace at /Users/example/private/trace.log",
        "current_version": "0.1.0",
        "proposed_version": "0.2.0",
        "risk": "medium",
        "status": "pending",
        "source": "legacy_import",
        "origin_channel": "cli",
        "snapshot": {
            "path": "/Users/example/.captain/skills/legacy-refinement",
            "snapshot_id": "AIzaSyC3_abcdefghij1234567890ABCDEFGhij",
            "created_at": "2026-05-18T00:00:00Z",
            "kind": "directory",
            "reason": "before /Users/example/private/edit"
        },
        "created_at": "2026-05-18T00:00:00Z",
        "updated_at": "2026-05-18T00:00:00Z",
        "notes": [{
            "at": "2026-05-18T00:00:00Z",
            "note": "Seen in /Users/example/private/debug.log"
        }]
    }]);
    let (kh, local_path, raw_secret) =
        output_safety_kernel_with_memory(vec![(SKILL_REFINEMENTS_KEY, legacy_refinements)]);

    let listed = tool_skill_refinement_list(&serde_json::json!({"limit": 10}), Some(&kh))
        .expect("legacy refinement list should serialize");
    assert_public_safe(&listed, &local_path, &raw_secret);
    assert!(
        !listed.contains("/Users/example/"),
        "legacy path leaked in list: {listed}"
    );
    assert!(listed.contains("<local-path>"));
    assert!(listed.contains("<secret>"));

    let decided = tool_skill_refinement_decide(
        &serde_json::json!({"id": "refine-legacy", "approve": true}),
        Some(&kh),
        None,
    )
    .expect_err("legacy refinement approval should be blocked from tool calls");
    assert_public_safe(&decided, &local_path, &raw_secret);
    assert!(
        !decided.contains("/Users/example/"),
        "legacy path leaked in decision: {decided}"
    );
    assert!(!decided.contains("<local-path>"));
    assert!(!decided.contains("<secret>"));
    assert!(
        !decided.contains("snapshot_id"),
        "snapshot id should remain internal: {decided}"
    );
}

#[test]
fn skill_refinement_restore_missing_skill_error_is_public_safe() {
    let leaked_skill = "/Users/example/private/legacy-skill";
    let legacy_refinements = serde_json::json!([{
        "id": "legacy-restore-1",
        "skill": leaked_skill,
        "finding": "legacy item",
        "suggested_change": "restore if needed",
        "risk": "medium",
        "status": "approved",
        "snapshot": {
            "snapshot_id": "legacy-snapshot-id",
            "created_at": "2026-05-18T00:00:00Z",
            "kind": "directory",
            "reason": "before-refinement-proposal"
        },
        "created_at": "2026-05-18T00:00:00Z",
        "updated_at": "2026-05-18T00:00:00Z",
        "notes": []
    }]);
    let (kh, _local_path, _raw_secret) =
        output_safety_kernel_with_memory(vec![(SKILL_REFINEMENTS_KEY, legacy_refinements)]);
    let skills_root = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(skills_root.path().to_path_buf());
    registry.load_all().unwrap();

    let error = tool_skill_refinement_restore(
        &serde_json::json!({"id": "legacy-restore"}),
        Some(&kh),
        Some(&registry),
    )
    .expect_err("missing legacy skill should fail before restore");
    assert!(
        !error.contains(leaked_skill),
        "legacy skill path leaked in error: {error}"
    );
    assert!(
        !error.contains("/Users/example/"),
        "legacy local path leaked in error: {error}"
    );
    assert!(error.contains("Skill is not available in registry"));
}
