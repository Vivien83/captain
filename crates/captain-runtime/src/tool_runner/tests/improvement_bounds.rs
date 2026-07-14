use super::*;

struct OvereagerReviewKernel;

#[async_trait::async_trait]
impl KernelHandle for OvereagerReviewKernel {
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

    fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn learning_review_list(&self, _limit: usize) -> Result<serde_json::Value, String> {
        Ok(review_items("learn", 55))
    }

    fn skill_proposal_list(&self, _limit: usize) -> Result<serde_json::Value, String> {
        Ok(review_items("proposal", 55))
    }

    async fn skill_proposal_decide(
        &self,
        proposal_id: &str,
        approve: bool,
        _decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "id": proposal_id,
            "status": if approve { "approved" } else { "denied" }
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

fn review_items(prefix: &str, count: usize) -> serde_json::Value {
    serde_json::Value::Array(
        (0..count)
            .map(|idx| {
                serde_json::json!({
                    "id": format!("{prefix}-{idx}"),
                    "subject": format!("{prefix} item {idx}")
                })
            })
            .collect(),
    )
}

#[test]
fn direct_review_lists_truncate_overeager_kernel_output() {
    let kh: Arc<dyn KernelHandle> = Arc::new(OvereagerReviewKernel);

    let learning = tool_learning_review_list(&serde_json::json!({"limit": 2}), Some(&kh))
        .expect("learning review should serialize");
    let learning_json: serde_json::Value = serde_json::from_str(&learning).unwrap();
    assert_eq!(learning_json.as_array().unwrap().len(), 2);
    assert_eq!(learning_json[1]["id"], "learn-1");

    let proposals = tool_skill_proposal_list(&serde_json::json!({"limit": 3}), Some(&kh))
        .expect("skill proposals should serialize");
    let proposals_json: serde_json::Value = serde_json::from_str(&proposals).unwrap();
    assert_eq!(proposals_json.as_array().unwrap().len(), 3);
    assert_eq!(proposals_json[2]["id"], "proposal-2");
}

#[tokio::test]
async fn self_review_and_proposal_prefix_resolution_use_bounded_output_window() {
    let kh: Arc<dyn KernelHandle> = Arc::new(OvereagerReviewKernel);

    let review = tool_self_improvement_review(&serde_json::json!({"limit": 2}), Some(&kh))
        .expect("self improvement review should serialize");
    let review_json: serde_json::Value = serde_json::from_str(&review).unwrap();
    assert_eq!(
        review_json["pending"]["learning_review"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(review_json["pending"]["skill_proposals"]["count"], 2);

    let err = tool_skill_proposal_decide(
        &serde_json::json!({"id": "proposal-54", "approve": true}),
        Some(&kh),
        Some("captain"),
    )
    .await
    .expect_err("prefix outside the bounded review window should not resolve");
    assert_eq!(err, "Skill proposal id not found");
}
