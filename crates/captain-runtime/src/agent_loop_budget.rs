use crate::agent_loop::AgentLoopResult;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::message::{Message, TokenUsage};

tokio::task_local! {
    static TURN_TOKEN_BUDGET: Option<u64>;
}

pub async fn with_turn_token_budget<F, T>(budget_tokens: Option<u64>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    TURN_TOKEN_BUDGET.scope(budget_tokens, fut).await
}

pub fn current_turn_token_budget() -> Option<u64> {
    TURN_TOKEN_BUDGET.try_with(|budget| *budget).ok().flatten()
}

pub(crate) fn budget_blocks_next_tool_step(total_usage: &TokenUsage) -> Option<u64> {
    let budget = current_turn_token_budget()?;
    if budget > 0 && total_usage.total() >= budget {
        Some(budget)
    } else {
        None
    }
}

pub(crate) async fn finish_budget_limited_turn(
    session: &mut Session,
    memory: &MemorySubstrate,
    total_usage: TokenUsage,
    iterations: u32,
    budget_tokens: u64,
) -> CaptainResult<AgentLoopResult> {
    let response = format!(
        "Budget de delegation atteint ({used}/{budget_tokens} tokens). J'arrete ce worker avant de lancer d'autres outils.",
        used = total_usage.total()
    );
    session.messages.push(Message::assistant(response.clone()));
    memory
        .save_session_async(session)
        .await
        .map_err(|e| CaptainError::Memory(e.to_string()))?;
    Ok(AgentLoopResult {
        response,
        total_usage,
        iterations,
        cost_usd: None,
        silent: false,
        directives: Default::default(),
        tool_calls: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{MessageContent, Role};

    #[tokio::test]
    async fn turn_token_budget_blocks_next_tool_step_when_reached() {
        let usage = TokenUsage {
            input_tokens: 80,
            output_tokens: 20,
            ..Default::default()
        };
        let blocked =
            with_turn_token_budget(Some(100), async { budget_blocks_next_tool_step(&usage) }).await;
        assert_eq!(blocked, Some(100));
    }

    #[tokio::test]
    async fn turn_token_budget_allows_next_tool_step_under_budget() {
        let usage = TokenUsage {
            input_tokens: 70,
            output_tokens: 20,
            ..Default::default()
        };
        let blocked =
            with_turn_token_budget(Some(100), async { budget_blocks_next_tool_step(&usage) }).await;
        assert_eq!(blocked, None);
    }

    #[tokio::test]
    async fn turn_token_budget_zero_never_blocks_tool_step() {
        let usage = TokenUsage {
            input_tokens: 100,
            ..Default::default()
        };
        let blocked =
            with_turn_token_budget(Some(0), async { budget_blocks_next_tool_step(&usage) }).await;
        assert_eq!(blocked, None);
    }

    #[tokio::test]
    async fn turn_token_budget_absent_never_blocks_tool_step() {
        let usage = TokenUsage {
            input_tokens: 100,
            ..Default::default()
        };
        let blocked =
            with_turn_token_budget(None, async { budget_blocks_next_tool_step(&usage) }).await;
        assert_eq!(blocked, None);
    }

    #[tokio::test]
    async fn finish_budget_limited_turn_records_and_persists_stop_message() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let mut session = Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: captain_types::agent::AgentId::new(),
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let usage = TokenUsage {
            input_tokens: 80,
            output_tokens: 20,
            ..Default::default()
        };

        let result = finish_budget_limited_turn(&mut session, &memory, usage, 3, 100)
            .await
            .unwrap();

        assert_eq!(result.iterations, 3);
        assert_eq!(result.total_usage.total(), 100);
        assert!(result.tool_calls.is_empty());
        assert!(result.response.contains("Budget de delegation atteint"));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, Role::Assistant);
        assert!(matches!(
            &session.messages[0].content,
            MessageContent::Text(text) if text == &result.response
        ));

        let persisted = memory
            .get_session(session.id)
            .unwrap()
            .expect("budget stop session must be persisted");
        assert_eq!(persisted.messages.len(), 1);
        assert_eq!(persisted.messages[0].role, Role::Assistant);
        assert!(matches!(
            &persisted.messages[0].content,
            MessageContent::Text(text) if text == &result.response
        ));
    }
}
