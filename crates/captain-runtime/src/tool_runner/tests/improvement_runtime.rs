use super::*;

#[test]
fn self_improvement_review_is_deferred_and_read_only() {
    assert!(!crate::core_tools::CORE_TOOLS.contains(&"self_improvement_review"));
    let tools = builtin_tool_definitions();
    let def = tools
        .iter()
        .find(|t| t.name == "self_improvement_review")
        .expect("self_improvement_review must be registered");
    assert!(def.description.contains("Read-only"));
    assert!(def.description.contains("approbation"));
}

#[tokio::test]
async fn self_improvement_review_requires_kernel() {
    let res = execute_tool(
        "test-id",
        "self_improvement_review",
        &serde_json::json!({"limit": 3}),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(res.is_error);
    assert!(res.content.contains("Kernel handle not available"));
}

struct SystemBugStubKernel {
    bugs: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait::async_trait]
impl KernelHandle for SystemBugStubKernel {
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
    fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        if key == SYSTEM_BUGS_KEY {
            *self.bugs.lock().unwrap() = Some(value);
        }
        Ok(())
    }
    fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        if key == SYSTEM_BUGS_KEY {
            Ok(self.bugs.lock().unwrap().clone())
        } else {
            Ok(None)
        }
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

#[test]
fn system_bug_tools_are_registered() {
    let names: Vec<String> = builtin_tool_definitions()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for name in ["system_bug_report", "system_bug_list", "system_bug_update"] {
        assert!(names.contains(&name.to_string()), "{name} missing");
    }
}

#[test]
fn system_bug_report_list_update_roundtrip() {
    let kh: Arc<dyn KernelHandle> = Arc::new(SystemBugStubKernel {
        bugs: std::sync::Mutex::new(None),
    });
    let created = tool_system_bug_report(
        &serde_json::json!({
            "title": "cron update missing",
            "description": "Captain had to cancel and recreate crons to edit them.",
            "category": "scheduler",
            "severity": "medium",
            "evidence": "User requested an in-place update tool.",
            "source": "user_report"
        }),
        Some(&kh),
    )
    .expect("bug report should store");
    let created_json: serde_json::Value = serde_json::from_str(&created).unwrap();
    let id = created_json["bug"]["id"].as_str().unwrap();

    let listed = tool_system_bug_list(&serde_json::json!({"category":"scheduler"}), Some(&kh))
        .expect("bug list should read");
    let listed_json: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed_json["count"], 1);

    let updated = tool_system_bug_update(
        &serde_json::json!({
            "id": &id[..8],
            "status": "fixed",
            "note": "Implemented cron_update."
        }),
        Some(&kh),
    )
    .expect("bug update should patch");
    let updated_json: serde_json::Value = serde_json::from_str(&updated).unwrap();
    assert_eq!(updated_json["bug"]["status"], "fixed");
    assert_eq!(updated_json["bug"]["notes"].as_array().unwrap().len(), 1);
}

struct SkillRefinementStubKernel {
    refinements: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait::async_trait]
impl KernelHandle for SkillRefinementStubKernel {
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
    fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        if key == SKILL_REFINEMENTS_KEY {
            *self.refinements.lock().unwrap() = Some(value);
        }
        Ok(())
    }
    fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        if key == SKILL_REFINEMENTS_KEY {
            Ok(self.refinements.lock().unwrap().clone())
        } else {
            Ok(None)
        }
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

#[test]
fn skill_refinement_tools_are_registered() {
    let names: Vec<String> = builtin_tool_definitions()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for name in [
        "skill_refinement_propose",
        "skill_refinement_list",
        "skill_refinement_decide",
        "skill_refinement_update",
        "skill_refinement_restore",
    ] {
        assert!(names.contains(&name.to_string()), "{name} missing");
    }
}

#[test]
fn skill_refinement_schema_exposes_restored_status() {
    let tools = builtin_tool_definitions();
    for name in ["skill_refinement_list", "skill_refinement_update"] {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} missing"));
        let statuses = tool.input_schema["properties"]["status"]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("{name} status enum missing"));
        assert!(
            statuses
                .iter()
                .any(|value| value.as_str() == Some("restored")),
            "{name} schema must expose restored status"
        );
    }
}

static CAPTAIN_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn create_file_backed_skill(root: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let skill_dir = root.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        format!(
            r#"
[skill]
name = "{name}"
version = "0.1.0"
description = "Path-safe refinement test skill"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "A test tool"
input_schema = {{ type = "object" }}
"#
        ),
    )
    .unwrap();
    std::fs::write(skill_dir.join("main.py"), body).unwrap();
    skill_dir
}

fn assert_refinement_output_is_path_safe(output: &str, roots: &[&Path]) {
    for field in [
        "skill_path",
        "snapshot_path",
        "restored_path",
        "snapshot_id",
    ] {
        assert!(!output.contains(field), "{field} leaked in {output}");
    }
    for root in roots {
        let root = root.to_string_lossy();
        assert!(
            !output.contains(root.as_ref()),
            "local root leaked in {output}"
        );
    }
}

fn assert_refinement_storage_has_no_local_paths(value: &serde_json::Value, roots: &[&Path]) {
    let value = value.to_string();
    for field in ["skill_path", "snapshot_path", "restored_path"] {
        assert!(!value.contains(field), "{field} leaked in {value}");
    }
    for root in roots {
        let root = root.to_string_lossy();
        assert!(
            !value.contains(root.as_ref()),
            "local root leaked in {value}"
        );
    }
}

#[test]
fn skill_refinement_roundtrip_tracks_version_and_applied_state() {
    let kh: Arc<dyn KernelHandle> = Arc::new(SkillRefinementStubKernel {
        refinements: std::sync::Mutex::new(None),
    });
    let proposed = tool_skill_refinement_propose(
        &serde_json::json!({
            "skill": "mcp-installer",
            "finding": "Context7 required the same install reasoning as another MCP server.",
            "suggested_change": "Document project-type detection and env mapping.",
            "current_version": "0.1.0",
            "proposed_version": "0.2.0",
            "risk": "medium"
        }),
        Some(&kh),
        None,
    )
    .expect("refinement proposal should store");
    let proposed_json: serde_json::Value = serde_json::from_str(&proposed).unwrap();
    let id = proposed_json["refinement"]["id"].as_str().unwrap();
    assert_eq!(proposed_json["refinement"]["proposed_version"], "0.2.0");

    let blocked = tool_skill_refinement_decide(
        &serde_json::json!({"id": &id[..8], "approve": true}),
        Some(&kh),
        Some("captain"),
    )
    .expect_err("tool calls must not approve skill refinements");
    assert!(
        blocked.contains("human/API/channel approval"),
        "approval guard should point to the external review surface: {blocked}"
    );
    let pending = tool_skill_refinement_list(
        &serde_json::json!({"id": &id[..8], "status": "pending"}),
        Some(&kh),
    )
    .expect("blocked approval should leave the refinement pending");
    let pending_json: serde_json::Value = serde_json::from_str(&pending).unwrap();
    assert_eq!(pending_json["count"], 1);

    let blocked_without_agent = tool_skill_refinement_decide(
        &serde_json::json!({"id": &id[..8], "approve": true}),
        Some(&kh),
        None,
    )
    .expect_err("ambiguous tool calls without an agent id must not approve skill refinements");
    assert!(
        blocked_without_agent.contains("human/API/channel approval"),
        "approval guard should not depend on caller id: {blocked_without_agent}"
    );

    let applied = tool_skill_refinement_update(
        &serde_json::json!({
            "id": &id[..8],
            "status": "applied",
            "note": "Patched and tested."
        }),
        Some(&kh),
    )
    .expect("update should mark applied");
    let applied_json: serde_json::Value = serde_json::from_str(&applied).unwrap();
    assert_eq!(applied_json["refinement"]["status"], "applied");
    assert_eq!(
        applied_json["refinement"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn skill_refinement_outputs_hide_paths_while_restore_uses_internal_snapshot() {
    let _lock = CAPTAIN_HOME_LOCK.lock().unwrap();
    let captain_home = tempfile::tempdir().unwrap();
    let _env = EnvGuard::set_path("CAPTAIN_HOME", captain_home.path());
    let skills_root = tempfile::tempdir().unwrap();
    let original = "print('original')\n";
    let skill_dir = create_file_backed_skill(skills_root.path(), "path-safe-skill", original);
    let mut registry = SkillRegistry::new(skills_root.path().to_path_buf());
    registry.load_all().unwrap();
    let kh: Arc<dyn KernelHandle> = Arc::new(SkillRefinementStubKernel {
        refinements: std::sync::Mutex::new(None),
    });
    let roots = [captain_home.path(), skills_root.path()];

    let proposed = tool_skill_refinement_propose(
        &serde_json::json!({
            "skill": "path-safe-skill",
            "finding": format!("The skill needs a tighter precondition after reading {}.", skill_dir.join("main.py").display()),
            "suggested_change": format!("Add the precondition without hard-coding {}.", skill_dir.join("main.py").display()),
            "evidence": format!("Failure evidence came from {}.", skill_dir.join("main.py").display()),
            "current_version": format!("observed in {}", skill_dir.join("main.py").display()),
            "proposed_version": format!("patch from {}", skill_dir.join("main.py").display()),
            "source": format!("reviewed via {}", skill_dir.join("main.py").display()),
            "channel": format!("cli at {}", captain_home.path().display()),
            "risk": "low"
        }),
        Some(&kh),
        Some(&registry),
    )
    .expect("file-backed proposal should snapshot");
    assert_refinement_output_is_path_safe(&proposed, &roots);
    let proposed_json: serde_json::Value = serde_json::from_str(&proposed).unwrap();
    let id = proposed_json["refinement"]["id"].as_str().unwrap();
    assert_eq!(proposed_json["refinement"]["snapshot"]["available"], true);
    assert_refinement_storage_has_no_local_paths(
        &kh.memory_recall(SKILL_REFINEMENTS_KEY).unwrap().unwrap(),
        &roots,
    );

    let snapshot = crate::tools::skill_refinement_ops::skill_refinement_snapshot(&kh, 10).unwrap();
    assert_refinement_output_is_path_safe(&snapshot.to_string(), &roots);

    let decision_err = tool_skill_refinement_decide(
        &serde_json::json!({"id": &id[..8], "approve": true}),
        Some(&kh),
        None,
    )
    .expect_err("tool approval should be blocked even without caller agent id");
    assert_refinement_output_is_path_safe(&decision_err, &roots);

    let updated = tool_skill_refinement_update(
        &serde_json::json!({
            "id": &id[..8],
            "status": "applied",
            "note": format!("patched after inspecting {}", skill_dir.join("main.py").display())
        }),
        Some(&kh),
    )
    .expect("update should keep output path-safe");
    assert_refinement_output_is_path_safe(&updated, &roots);
    assert_refinement_storage_has_no_local_paths(
        &kh.memory_recall(SKILL_REFINEMENTS_KEY).unwrap().unwrap(),
        &roots,
    );

    let listed = tool_skill_refinement_list(&serde_json::json!({"limit": 10}), Some(&kh))
        .expect("list should keep output path-safe");
    assert_refinement_output_is_path_safe(&listed, &roots);

    std::fs::write(skill_dir.join("main.py"), "print('mutated')\n").unwrap();
    let restored = tool_skill_refinement_restore(
        &serde_json::json!({
            "id": &id[..8],
            "note": format!("Rollback after failed patch in {}.", skill_dir.join("main.py").display())
        }),
        Some(&kh),
        Some(&registry),
    )
    .expect("restore should use the internal snapshot");
    assert_refinement_output_is_path_safe(&restored, &roots);
    let restored_json: serde_json::Value = serde_json::from_str(&restored).unwrap();
    assert_eq!(
        restored_json["refinement"]["restore_backup"]["available"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(skill_dir.join("main.py")).unwrap(),
        original
    );
    assert_refinement_storage_has_no_local_paths(
        &kh.memory_recall(SKILL_REFINEMENTS_KEY).unwrap().unwrap(),
        &roots,
    );
}
