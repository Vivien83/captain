//! Task-local stdout/stderr streaming for long-running tools.

/// Ambient per-tool streaming context.
#[derive(Debug, Clone)]
pub struct ToolStreamCtx {
    pub tool_use_id: String,
    pub tx: tokio::sync::mpsc::Sender<crate::llm_driver::StreamEvent>,
}

tokio::task_local! {
    pub static TOOL_STREAM: Option<ToolStreamCtx>;
}

/// Emit a chunk if a streaming context is active; no-op otherwise.
pub(crate) fn emit_tool_chunk(stream: &'static str, chunk: &str) {
    if chunk.is_empty() {
        return;
    }
    let owned = chunk.to_string();
    let _ = TOOL_STREAM.try_with(|ctx| {
        if let Some(ctx) = ctx.as_ref() {
            let tx = ctx.tx.clone();
            let id = ctx.tool_use_id.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(crate::llm_driver::StreamEvent::ToolOutputDelta {
                        tool_use_id: id,
                        stream,
                        chunk: owned,
                    })
                    .await;
            });
        }
    });
}
