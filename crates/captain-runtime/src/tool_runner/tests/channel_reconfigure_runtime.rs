use super::*;

/// Stub kernel for channel_reconfigure tests: captures publish calls and
/// hands back a configurable `home_dir` so the tool can read a fixture
/// `config.toml` we control.
struct ChannelReconfigureStub {
    home: std::path::PathBuf,
    published: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl KernelHandle for ChannelReconfigureStub {
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
    fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
        Ok(())
    }
    fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
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
    fn home_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.home.clone())
    }
    fn publish_integration_configured(&self, name: &str) {
        self.published.lock().unwrap().push(name.to_string());
    }
}

fn write_fixture_config(home: &std::path::Path, body: &str) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(home.join("config.toml"), body).unwrap();
}

#[test]
fn channel_reconfigure_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"channel_reconfigure"),
        "channel_reconfigure must be exposed as a builtin tool"
    );
    let def = tools
        .iter()
        .find(|t| t.name == "channel_reconfigure")
        .unwrap();
    assert!(
        def.description.contains("SPONTANÉMENT") || def.description.contains("spontanément"),
        "must contain proactive guidance"
    );
    let required = def.input_schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("channel")));
}

#[tokio::test]
async fn channel_reconfigure_rejects_missing_channel_param() {
    let home = std::env::temp_dir().join("captain_test_chan_recfg_missing");
    let _ = std::fs::remove_dir_all(&home);
    write_fixture_config(&home, "[channels.telegram]\nbot_token_env = \"T\"\n");
    let kh: Arc<dyn KernelHandle> = Arc::new(ChannelReconfigureStub {
        home: home.clone(),
        published: std::sync::Mutex::new(Vec::new()),
    });
    let res = tool_channel_reconfigure(&serde_json::json!({}), Some(&kh)).await;
    assert!(res.is_err(), "missing channel must error");
    assert!(res.unwrap_err().contains("channel"));
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn channel_reconfigure_rejects_unknown_channel() {
    let home = std::env::temp_dir().join("captain_test_chan_recfg_unknown");
    let _ = std::fs::remove_dir_all(&home);
    write_fixture_config(&home, "[channels.telegram]\nbot_token_env = \"T\"\n");
    let stub = Arc::new(ChannelReconfigureStub {
        home: home.clone(),
        published: std::sync::Mutex::new(Vec::new()),
    });
    let kh: Arc<dyn KernelHandle> = stub.clone();
    let res =
        tool_channel_reconfigure(&serde_json::json!({ "channel": "discord" }), Some(&kh)).await;
    assert!(res.is_err(), "unknown channel must error");
    let err = res.unwrap_err();
    assert!(
        err.contains("discord"),
        "err must mention the bad name: {err}"
    );
    assert!(
        stub.published.lock().unwrap().is_empty(),
        "no event must be published when validation fails"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn channel_reconfigure_rejects_configured_frozen_channel() {
    let home = std::env::temp_dir().join("captain_test_chan_recfg_frozen");
    let _ = std::fs::remove_dir_all(&home);
    write_fixture_config(
        &home,
        "[channels.telegram]\nbot_token_env = \"T\"\n[channels.slack]\nbot_token_env = \"S\"\n",
    );
    let stub = Arc::new(ChannelReconfigureStub {
        home: home.clone(),
        published: std::sync::Mutex::new(Vec::new()),
    });
    let kh: Arc<dyn KernelHandle> = stub.clone();
    let res = tool_channel_reconfigure(&serde_json::json!({ "channel": "slack" }), Some(&kh)).await;
    let err = res.expect_err("configured non-core channels must stay frozen");
    assert!(err.contains("slack"), "err must mention channel: {err}");
    assert!(
        err.contains("telegram, discord, signal, email"),
        "err must name active channels: {err}"
    );
    assert!(err.contains("frozen"), "err must explain the freeze: {err}");
    assert!(
        stub.published.lock().unwrap().is_empty(),
        "no event must be published when validation fails"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn channel_reconfigure_publishes_event_for_known_channel() {
    let home = std::env::temp_dir().join("captain_test_chan_recfg_known");
    let _ = std::fs::remove_dir_all(&home);
    write_fixture_config(
        &home,
        "[channels.telegram]\nbot_token_env = \"T\"\n[channels.discord]\nbot_token_env = \"D\"\n",
    );
    let stub = Arc::new(ChannelReconfigureStub {
        home: home.clone(),
        published: std::sync::Mutex::new(Vec::new()),
    });
    let kh: Arc<dyn KernelHandle> = stub.clone();
    let res =
        tool_channel_reconfigure(&serde_json::json!({ "channel": "telegram" }), Some(&kh)).await;
    assert!(res.is_ok(), "valid channel must succeed: {res:?}");
    let body = res.unwrap();
    assert!(body.contains("telegram"), "response must echo channel name");
    let published = stub.published.lock().unwrap().clone();
    assert_eq!(published, vec!["telegram".to_string()]);
    let _ = std::fs::remove_dir_all(&home);
}
