use crate::agent_loop_completion::{
    decide_end_turn_response, EndTurnDecision, EndTurnDecisionInput,
};
use crate::agent_loop_finish::{finish_silent_turn, finish_successful_turn, SuccessfulTurnInput};
use crate::agent_loop_messages::assistant_message_for_response;
use crate::agent_loop_phase::{notify_delivery_verification_phase, LoopPhase, PhaseCallback};
use crate::agent_loop_result::AgentLoopResult;
use crate::agent_loop_tool_flow::capability_search_nudge;
use crate::agent_loop_tool_record::ToolCallRecord;
use crate::embedding::EmbeddingDriver;
use crate::llm_driver::CompletionResponse;
use crate::work_verification::{
    evaluate_tool_receipts, verification_identifier_digest, VerificationDisposition,
    WorkVerificationReport, MAX_VERIFICATION_CORRECTION_ROUNDS,
};
use captain_memory::session::Session;
use captain_memory::work_verification_progress::{
    DurableVerificationGap, WorkVerificationLease, WorkVerificationProgress, WorkVerificationState,
    WORK_VERIFICATION_SCHEMA_VERSION,
};
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::message::{Message, TokenUsage};
use captain_types::tool::ToolDefinition;

pub(crate) struct EndTurnInput<'a> {
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) user_message: &'a str,
    pub(crate) response: &'a CompletionResponse,
    pub(crate) total_usage: &'a TokenUsage,
    pub(crate) messages: &'a mut Vec<Message>,
    pub(crate) iteration: u32,
    pub(crate) any_tools_executed: bool,
    pub(crate) capability_denial_watchdog_used: &'a mut bool,
    pub(crate) verification_correction_rounds: &'a mut u8,
    pub(crate) verification_operation: &'a mut Option<WorkVerificationLease>,
    pub(crate) max_iterations: u32,
    pub(crate) visible_tools: &'a [ToolDefinition],
    pub(crate) streaming: bool,
    pub(crate) phantom_action_watchdog: bool,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) agent_id_str: &'a str,
    pub(crate) tool_calls_recorded: &'a [ToolCallRecord],
}

pub(crate) async fn handle_end_turn_response(
    input: EndTurnInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    let EndTurnInput {
        manifest,
        user_message,
        response,
        total_usage,
        messages,
        iteration,
        any_tools_executed,
        capability_denial_watchdog_used,
        verification_correction_rounds,
        verification_operation,
        max_iterations,
        visible_tools,
        streaming,
        phantom_action_watchdog,
        session,
        memory,
        embedding_driver,
        on_phase,
        hooks,
        agent_id_str,
        tool_calls_recorded,
    } = input;

    match decide_end_turn_response(EndTurnDecisionInput {
        agent_name: &manifest.name,
        response,
        total_usage,
        messages_len: messages.len(),
        iteration,
        any_tools_executed,
        capability_denial_watchdog_used: *capability_denial_watchdog_used,
        visible_tools,
        streaming,
        phantom_action_watchdog,
    }) {
        EndTurnDecision::Silent { directives } => {
            let report = delivery_verification_report(
                tool_calls_recorded,
                any_tools_executed,
                *verification_correction_rounds,
                iteration,
                max_iterations,
            );
            begin_delivery_verification(
                memory,
                session,
                verification_operation,
                *verification_correction_rounds,
                &report,
                on_phase,
            )?;
            match report.disposition {
                VerificationDisposition::NeedsCorrection => {
                    let next_round = verification_correction_rounds.saturating_add(1);
                    record_delivery_verification(
                        memory,
                        session,
                        verification_operation,
                        WorkVerificationState::Correcting,
                        next_round,
                        &report,
                        on_phase,
                    )?;
                    enqueue_verification_correction(
                        messages,
                        response,
                        "[Silent completion withheld pending verification.]".to_string(),
                        verification_correction_rounds,
                        next_round,
                        &report,
                    );
                    Ok(None)
                }
                VerificationDisposition::Incomplete => {
                    let final_response = report.incomplete_notice();
                    let assistant_message =
                        assistant_message_for_response(response, final_response.clone());
                    let result = finish_successful_turn(SuccessfulTurnInput {
                        manifest,
                        user_message,
                        final_response,
                        assistant_message,
                        completed_iterations: iteration + 1,
                        session,
                        memory,
                        embedding_driver,
                        on_phase,
                        hooks,
                        agent_id_str,
                        total_usage: *total_usage,
                        tool_calls_recorded,
                        streaming,
                    })
                    .await?;
                    record_delivery_verification(
                        memory,
                        session,
                        verification_operation,
                        WorkVerificationState::Incomplete,
                        *verification_correction_rounds,
                        &report,
                        on_phase,
                    )?;
                    Ok(Some(result))
                }
                VerificationDisposition::NotRequired => finish_silent_turn(
                    session,
                    memory,
                    *total_usage,
                    iteration + 1,
                    directives,
                    tool_calls_recorded,
                )
                .await
                .map(Some),
                VerificationDisposition::Verified => {
                    let result = finish_silent_turn(
                        session,
                        memory,
                        *total_usage,
                        iteration + 1,
                        directives,
                        tool_calls_recorded,
                    )
                    .await?;
                    record_delivery_verification(
                        memory,
                        session,
                        verification_operation,
                        WorkVerificationState::Verified,
                        *verification_correction_rounds,
                        &report,
                        on_phase,
                    )?;
                    Ok(Some(result))
                }
            }
        }
        EndTurnDecision::RetryEmpty { silent_failure } => {
            if silent_failure {
                *messages = crate::session_repair::validate_and_repair(&*messages);
            }
            messages.push(Message::assistant("[no response]".to_string()));
            messages.push(Message::user("Please provide your response.".to_string()));
            Ok(None)
        }
        EndTurnDecision::RetryPhantom { text } => {
            messages.push(Message::assistant(text));
            messages.push(Message::user(
                "[System: You claimed to perform an action but did not call any tools. \
                 You must use the appropriate tool (e.g., channel_send, web_fetch, file_write) \
                 to actually perform the action. Do not claim completion without executing tools.]",
            ));
            Ok(None)
        }
        EndTurnDecision::RetryCapability { text } => {
            *capability_denial_watchdog_used = true;
            messages.push(Message::assistant(text));
            messages.push(capability_search_nudge());
            Ok(None)
        }
        EndTurnDecision::Complete { text } => {
            let report = delivery_verification_report(
                tool_calls_recorded,
                any_tools_executed,
                *verification_correction_rounds,
                iteration,
                max_iterations,
            );
            begin_delivery_verification(
                memory,
                session,
                verification_operation,
                *verification_correction_rounds,
                &report,
                on_phase,
            )?;
            if report.requires_correction() {
                let next_round = verification_correction_rounds.saturating_add(1);
                record_delivery_verification(
                    memory,
                    session,
                    verification_operation,
                    WorkVerificationState::Correcting,
                    next_round,
                    &report,
                    on_phase,
                )?;
                enqueue_verification_correction(
                    messages,
                    response,
                    text,
                    verification_correction_rounds,
                    next_round,
                    &report,
                );
                return Ok(None);
            }

            let final_response = if report.disposition == VerificationDisposition::Incomplete {
                incomplete_delivery_text(&report, &text)
            } else {
                text
            };
            let assistant_message =
                assistant_message_for_response(response, final_response.clone());
            let defer_done_phase = matches!(
                report.disposition,
                VerificationDisposition::Verified | VerificationDisposition::Incomplete
            );
            let result = finish_successful_turn(SuccessfulTurnInput {
                manifest,
                user_message,
                final_response,
                assistant_message,
                completed_iterations: iteration + 1,
                session,
                memory,
                embedding_driver,
                on_phase: if defer_done_phase { None } else { on_phase },
                hooks,
                agent_id_str,
                total_usage: *total_usage,
                tool_calls_recorded,
                streaming,
            })
            .await?;
            match report.disposition {
                VerificationDisposition::Verified => record_delivery_verification(
                    memory,
                    session,
                    verification_operation,
                    WorkVerificationState::Verified,
                    *verification_correction_rounds,
                    &report,
                    on_phase,
                )?,
                VerificationDisposition::Incomplete => record_delivery_verification(
                    memory,
                    session,
                    verification_operation,
                    WorkVerificationState::Incomplete,
                    *verification_correction_rounds,
                    &report,
                    on_phase,
                )?,
                VerificationDisposition::NotRequired | VerificationDisposition::NeedsCorrection => {
                }
            }
            if defer_done_phase {
                notify_done_phase(on_phase);
            }
            Ok(Some(result))
        }
    }
}

fn notify_done_phase(on_phase: Option<&PhaseCallback>) {
    if let Some(callback) = on_phase {
        callback(LoopPhase::Done);
    }
}

pub(crate) fn delivery_verification_report(
    tool_calls_recorded: &[ToolCallRecord],
    any_tools_executed: bool,
    correction_rounds: u8,
    iteration: u32,
    max_iterations: u32,
) -> WorkVerificationReport {
    let correction_rounds = if iteration.saturating_add(1) >= max_iterations {
        MAX_VERIFICATION_CORRECTION_ROUNDS
    } else {
        correction_rounds
    };
    evaluate_tool_receipts(tool_calls_recorded, any_tools_executed, correction_rounds)
}

pub(crate) fn incomplete_delivery_text(
    report: &WorkVerificationReport,
    candidate_text: &str,
) -> String {
    if candidate_text.trim().is_empty() {
        report.incomplete_notice()
    } else {
        format!(
            "{}\n\nUnverified draft from the agent:\n{}",
            report.incomplete_notice(),
            candidate_text
        )
    }
}

pub(crate) fn begin_delivery_verification(
    memory: &MemorySubstrate,
    session: &Session,
    operation: &mut Option<WorkVerificationLease>,
    correction_round: u8,
    report: &WorkVerificationReport,
    on_phase: Option<&PhaseCallback>,
) -> CaptainResult<()> {
    if report.disposition == VerificationDisposition::NotRequired {
        return Ok(());
    }
    record_delivery_verification(
        memory,
        session,
        operation,
        WorkVerificationState::Verifying,
        correction_round,
        report,
        on_phase,
    )
}

pub(crate) fn record_delivery_verification(
    memory: &MemorySubstrate,
    session: &Session,
    operation: &mut Option<WorkVerificationLease>,
    state: WorkVerificationState,
    correction_round: u8,
    report: &WorkVerificationReport,
    on_phase: Option<&PhaseCallback>,
) -> CaptainResult<()> {
    let now_unix_ms = chrono::Utc::now().timestamp_millis();
    if operation.is_none() {
        if state != WorkVerificationState::Verifying {
            return Err(CaptainError::Memory(
                "verification terminal transition has no active lease".to_string(),
            ));
        }
        let progress = durable_verification_progress(
            uuid::Uuid::new_v4().to_string(),
            memory.runtime_instance_id().to_string(),
            session,
            state,
            correction_round,
            report,
            now_unix_ms,
            now_unix_ms,
        );
        *operation = Some(memory.start_work_verification_progress(progress)?);
        notify_delivery_verification_phase(on_phase, LoopPhase::Verifying);
        return Ok(());
    }

    let terminal = matches!(
        state,
        WorkVerificationState::Verified
            | WorkVerificationState::Incomplete
            | WorkVerificationState::Interrupted
    );
    {
        let Some(active) = operation.as_mut() else {
            return Err(CaptainError::Memory(
                "verification operation was not initialized".to_string(),
            ));
        };
        let current = active.progress();
        let progress = durable_verification_progress(
            current.operation_id.clone(),
            current.runtime_instance_id.clone(),
            session,
            state,
            correction_round,
            report,
            current.started_at_ms,
            now_unix_ms.max(current.started_at_ms),
        );
        active
            .record(progress)
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
    }
    if terminal {
        *operation = None;
    }
    notify_delivery_verification_phase(on_phase, loop_phase_for_verification_state(state));
    Ok(())
}

fn loop_phase_for_verification_state(state: WorkVerificationState) -> LoopPhase {
    match state {
        WorkVerificationState::Verifying => LoopPhase::Verifying,
        WorkVerificationState::Correcting => LoopPhase::Correcting,
        WorkVerificationState::Verified => LoopPhase::VerificationVerified,
        WorkVerificationState::Incomplete | WorkVerificationState::Interrupted => {
            LoopPhase::VerificationIncomplete
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn durable_verification_progress(
    operation_id: String,
    runtime_instance_id: String,
    session: &Session,
    state: WorkVerificationState,
    correction_round: u8,
    report: &WorkVerificationReport,
    started_at_ms: i64,
    updated_at_ms: i64,
) -> WorkVerificationProgress {
    WorkVerificationProgress {
        schema_version: WORK_VERIFICATION_SCHEMA_VERSION,
        operation_id,
        runtime_instance_id,
        agent_id: session.agent_id,
        session_id: session.id,
        state,
        correction_round,
        receipt_digests: report
            .observed_receipts
            .iter()
            .take(128)
            .map(|receipt| verification_identifier_digest(receipt))
            .collect(),
        gaps: report
            .gaps
            .iter()
            .take(32)
            .map(|gap| DurableVerificationGap {
                code: gap.code.to_string(),
                tool_name: gap.tool_name.chars().take(128).collect(),
                sequence: gap.sequence,
                scope_digest: gap.scope_hint.clone(),
            })
            .collect(),
        detail: verification_state_detail(state).to_string(),
        started_at_ms,
        updated_at_ms,
    }
}

fn verification_state_detail(state: WorkVerificationState) -> &'static str {
    match state {
        WorkVerificationState::Verifying => "Evaluating ordered evidence before delivery",
        WorkVerificationState::Correcting => "Targeted correction requested before delivery",
        WorkVerificationState::Verified => "Required post-condition evidence accepted",
        WorkVerificationState::Incomplete => {
            "Verification circuit breaker returned an incomplete delivery"
        }
        WorkVerificationState::Interrupted => {
            "Verification was interrupted before a terminal decision"
        }
    }
}

fn enqueue_verification_correction(
    messages: &mut Vec<Message>,
    response: &CompletionResponse,
    candidate_text: String,
    correction_rounds: &mut u8,
    next_round: u8,
    report: &WorkVerificationReport,
) {
    *correction_rounds = next_round;
    messages.push(assistant_message_for_response(response, candidate_text));
    messages.push(Message::user(report.correction_nudge()));
}

#[cfg(test)]
#[path = "agent_loop_end_turn_tests.rs"]
mod tests;
