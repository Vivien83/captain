use super::*;

struct SystemBugSafetyStubKernel {
    bugs: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait::async_trait]
impl KernelHandle for SystemBugSafetyStubKernel {
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
fn system_bug_registry_rejects_secrets_and_redacts_local_paths() {
    let kh: Arc<dyn KernelHandle> = Arc::new(SystemBugSafetyStubKernel {
        bugs: std::sync::Mutex::new(None),
    });
    let dir = tempfile::tempdir().unwrap();
    let local_path = dir.path().join("secrets.env").to_string_lossy().to_string();
    let raw_secret = "AIzaSyC3_abcdefghij1234567890ABCDEFGhij";

    let secret = tool_system_bug_report(
        &serde_json::json!({
            "title": "raw token should not persist",
            "description": "A tool returned a raw credential.",
            "category": "security",
            "severity": "high",
            "evidence": raw_secret
        }),
        Some(&kh),
    )
    .expect_err("raw secret evidence should be blocked");
    assert!(secret.contains("Security blocked"), "got: {secret}");
    assert!(
        !secret.contains(raw_secret),
        "secret echoed in error: {secret}"
    );

    let created = tool_system_bug_report(
        &serde_json::json!({
            "title": format!("bug observed in {local_path}"),
            "description": format!("The retry state referenced {local_path}."),
            "category": "tool",
            "severity": "medium",
            "evidence": format!("stderr pointed at {local_path}."),
            "suggested_fix": format!("Use a generic recovery note instead of {local_path}."),
            "source": format!("self review from {local_path}")
        }),
        Some(&kh),
    )
    .expect("path-bearing bug report should be redacted");
    assert!(
        !created.contains(&local_path),
        "path leaked in create: {created}"
    );
    assert!(
        created.contains("<local-path>"),
        "redaction missing: {created}"
    );
    let id = serde_json::from_str::<serde_json::Value>(&created).unwrap()["bug"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let updated = tool_system_bug_update(
        &serde_json::json!({
            "id": &id[..8],
            "note": format!("verified with {local_path}"),
            "suggested_fix": format!("do not publish {local_path}")
        }),
        Some(&kh),
    )
    .expect("path-bearing update should be redacted");
    assert!(
        !updated.contains(&local_path),
        "path leaked in update: {updated}"
    );

    let stored = kh
        .memory_recall(SYSTEM_BUGS_KEY)
        .unwrap()
        .unwrap()
        .to_string();
    assert!(
        !stored.contains(&local_path),
        "path leaked in storage: {stored}"
    );
}
