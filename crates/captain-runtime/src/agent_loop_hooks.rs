use captain_types::agent::{AgentManifest, HookEvent};
use captain_types::message::ContentBlock;
use captain_types::tool::{ToolCall, ToolResult};

pub(crate) fn before_tool_call_allows_execution(
    hooks: Option<&crate::hooks::HookRegistry>,
    manifest: &AgentManifest,
    caller_id: &str,
    tool_call: &ToolCall,
    tool_result_blocks: &mut Vec<ContentBlock>,
) -> bool {
    let Some(hook_reg) = hooks else {
        return true;
    };

    let ctx = crate::hooks::HookContext {
        agent_name: &manifest.name,
        agent_id: caller_id,
        event: HookEvent::BeforeToolCall,
        data: serde_json::json!({
            "tool_name": &tool_call.name,
            "input": &tool_call.input,
        }),
    };

    if let Err(reason) = hook_reg.fire(&ctx) {
        crate::workflow_learning_runtime::record_terminal_tool_attempt(
            tool_call,
            true,
            "hook_blocked",
        );
        push_hook_block_result(tool_result_blocks, tool_call, &reason);
        return false;
    }

    true
}

pub(crate) fn fire_after_tool_call_hook(
    hooks: Option<&crate::hooks::HookRegistry>,
    manifest: &AgentManifest,
    caller_id: &str,
    tool_call: &ToolCall,
    result: &ToolResult,
) {
    let Some(hook_reg) = hooks else {
        return;
    };

    let ctx = crate::hooks::HookContext {
        agent_name: &manifest.name,
        agent_id: caller_id,
        event: HookEvent::AfterToolCall,
        data: serde_json::json!({
            "tool_name": &tool_call.name,
            "result": &result.content,
            "is_error": result.is_error,
        }),
    };
    let _ = hook_reg.fire(&ctx);
}

fn push_hook_block_result(
    tool_result_blocks: &mut Vec<ContentBlock>,
    tool_call: &ToolCall,
    reason: &str,
) {
    tool_result_blocks.push(ContentBlock::ToolResult {
        tool_use_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        content: format!("Hook blocked tool '{}': {}", tool_call.name, reason),
        is_error: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookContext, HookHandler, HookRegistry};
    use std::sync::Arc;

    struct BlockHandler;

    impl HookHandler for BlockHandler {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            Err("not allowed".to_string())
        }
    }

    struct OkHandler;

    impl HookHandler for OkHandler {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            Ok(())
        }
    }

    struct CaptureHandler {
        calls: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl CaptureHandler {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<serde_json::Value> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl HookHandler for CaptureHandler {
        fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
            self.calls.lock().unwrap().push(ctx.data.clone());
            Ok(())
        }
    }

    fn test_tool_call() -> ToolCall {
        ToolCall {
            id: "call-1".to_string(),
            name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd":"pwd"}),
        }
    }

    fn test_tool_result(is_error: bool) -> ToolResult {
        ToolResult {
            tool_use_id: "call-1".to_string(),
            content: "done".to_string(),
            is_error,
            transient_content: Vec::new(),
        }
    }

    #[test]
    fn absent_hook_registry_allows_execution() {
        let manifest = AgentManifest::default();
        let tool_call = test_tool_call();
        let mut blocks = Vec::new();

        let allowed =
            before_tool_call_allows_execution(None, &manifest, "agent", &tool_call, &mut blocks);

        assert!(allowed);
        assert!(blocks.is_empty());
    }

    #[test]
    fn successful_before_tool_hook_allows_execution() {
        let mut manifest = AgentManifest::default();
        manifest.name = "captain".to_string();
        let registry = HookRegistry::new();
        registry.register(HookEvent::BeforeToolCall, Arc::new(OkHandler));
        let tool_call = test_tool_call();
        let mut blocks = Vec::new();

        let allowed = before_tool_call_allows_execution(
            Some(&registry),
            &manifest,
            "agent",
            &tool_call,
            &mut blocks,
        );

        assert!(allowed);
        assert!(blocks.is_empty());
    }

    #[test]
    fn blocking_before_tool_hook_adds_error_result() {
        let mut manifest = AgentManifest::default();
        manifest.name = "captain".to_string();
        let registry = HookRegistry::new();
        registry.register(HookEvent::BeforeToolCall, Arc::new(BlockHandler));
        let tool_call = test_tool_call();
        let mut blocks = Vec::new();

        let allowed = before_tool_call_allows_execution(
            Some(&registry),
            &manifest,
            "agent",
            &tool_call,
            &mut blocks,
        );

        assert!(!allowed);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                is_error: true,
            } if tool_use_id == "call-1"
                && tool_name == "shell_exec"
                && content == "Hook blocked tool 'shell_exec': not allowed"
        ));
    }

    #[test]
    fn absent_after_tool_hook_is_noop() {
        let manifest = AgentManifest::default();
        let tool_call = test_tool_call();
        let result = test_tool_result(false);

        fire_after_tool_call_hook(None, &manifest, "agent", &tool_call, &result);
    }

    #[test]
    fn after_tool_hook_receives_tool_result_payload() {
        let mut manifest = AgentManifest::default();
        manifest.name = "captain".to_string();
        let registry = HookRegistry::new();
        let capture = Arc::new(CaptureHandler::new());
        registry.register(HookEvent::AfterToolCall, capture.clone());
        let tool_call = test_tool_call();
        let result = test_tool_result(true);

        fire_after_tool_call_hook(Some(&registry), &manifest, "agent", &tool_call, &result);

        let calls = capture.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["tool_name"], "shell_exec");
        assert_eq!(calls[0]["result"], "done");
        assert_eq!(calls[0]["is_error"], true);
    }
}
