//! Task-local stdout/stderr streaming for long-running tools.

/// Ambient per-tool streaming context.
#[derive(Debug, Clone)]
pub struct ToolStreamCtx {
    pub tool_use_id: String,
    pub tx: tokio::sync::mpsc::Sender<crate::llm_driver::StreamEvent>,
    remaining_output_bytes: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ToolStreamCtx {
    pub fn new(
        tool_use_id: String,
        tx: tokio::sync::mpsc::Sender<crate::llm_driver::StreamEvent>,
    ) -> Self {
        Self {
            tool_use_id,
            tx,
            remaining_output_bytes: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
                crate::tool_run_output::DEFAULT_PER_RUN_CAP_BYTES,
            )),
        }
    }

    fn reserve_chunk<'a>(&self, chunk: &'a str) -> Option<&'a str> {
        use std::sync::atomic::Ordering;

        loop {
            let remaining = self.remaining_output_bytes.load(Ordering::Relaxed);
            if remaining == 0 {
                return None;
            }
            let accepted = crate::str_utils::safe_truncate_str(chunk, remaining as usize);
            if accepted.is_empty() {
                return None;
            }
            let next = remaining.saturating_sub(accepted.len() as u64);
            if self
                .remaining_output_bytes
                .compare_exchange_weak(remaining, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Some(accepted);
            }
        }
    }
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
            let Some(chunk) = ctx.reserve_chunk(&owned) else {
                return;
            };
            let owned = chunk.to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_forwarding_is_bounded_before_queueing() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let ctx = ToolStreamCtx::new("tool-use".to_string(), tx);
        let oversized = "x".repeat(crate::tool_run_output::DEFAULT_PER_RUN_CAP_BYTES as usize + 64);

        TOOL_STREAM
            .scope(Some(ctx), async {
                emit_tool_chunk("stdout", &oversized);
                emit_tool_chunk("stdout", "must-not-be-queued");
                tokio::task::yield_now().await;
            })
            .await;

        let crate::llm_driver::StreamEvent::ToolOutputDelta { chunk, .. } =
            rx.recv().await.unwrap()
        else {
            panic!("expected tool output delta");
        };
        assert_eq!(
            chunk.len(),
            crate::tool_run_output::DEFAULT_PER_RUN_CAP_BYTES as usize
        );
        assert!(rx.try_recv().is_err());
    }
}
