use crate::project_ask::normalize_project_ask_answer;
use crate::project_runtime_checkpoints::trim_runtime_text;
use chrono::Utc;

const USER_QUESTION_LIMIT: usize = 40;
const TIMELINE_LIMIT: usize = 120;
const NO_USER_QUESTIONS_CONTEXT: &str = "No project user questions have been recorded.";

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProjectAsk<'a> {
    pub ask_id: &'a str,
    pub run_id: &'a str,
    pub phase: &'a str,
    pub worker_id: &'a str,
    pub agent_id: &'a str,
    pub worker_role: &'a str,
    pub question: &'a str,
    pub options: Option<&'a [String]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeProjectAskAnswer {
    pub ask_id: String,
    pub phase: String,
    pub question: String,
    pub answer: String,
    pub was_pending: bool,
}

pub(crate) fn ensure_runtime_question_store(runtime: &mut serde_json::Value) {
    if !runtime
        .get("user_questions")
        .map(|value| value.is_array())
        .unwrap_or(false)
    {
        runtime["user_questions"] = serde_json::json!([]);
    }
}

pub(crate) fn record_runtime_project_ask(
    runtime: &mut serde_json::Value,
    ask: RuntimeProjectAsk<'_>,
) {
    ensure_runtime_question_store(runtime);
    let now = Utc::now().to_rfc3339();
    let options = ask
        .options
        .map(|items| serde_json::json!(items))
        .unwrap_or(serde_json::Value::Null);
    let record = serde_json::json!({
        "ask_id": ask.ask_id,
        "run_id": ask.run_id,
        "phase": ask.phase,
        "worker_id": ask.worker_id,
        "agent_id": ask.agent_id,
        "worker_role": ask.worker_role,
        "question": trim_runtime_text(ask.question, 1800),
        "options": options,
        "status": "pending",
        "delivery": "waiting_for_user",
        "created_at": now,
        "updated_at": now,
    });
    let Some(items) = runtime
        .get_mut("user_questions")
        .and_then(|value| value.as_array_mut())
    else {
        return;
    };
    if let Some(existing) = items
        .iter_mut()
        .find(|item| item.get("ask_id").and_then(|value| value.as_str()) == Some(ask.ask_id))
    {
        *existing = record;
        return;
    }
    items.push(record);
    if items.len() > USER_QUESTION_LIMIT {
        let drain = items.len() - USER_QUESTION_LIMIT;
        items.drain(0..drain);
    }
}

pub(crate) fn record_runtime_project_ask_event(
    runtime: &mut serde_json::Value,
    ask: RuntimeProjectAsk<'_>,
) {
    let ask_id = ask.ask_id.to_string();
    let run_id = ask.run_id.to_string();
    let phase = ask.phase.to_string();
    let worker_id = ask.worker_id.to_string();
    let agent_id = ask.agent_id.to_string();
    let role = ask.worker_role.to_string();
    let question = ask.question.to_string();
    let options = ask.options.map(|items| serde_json::json!(items));

    record_runtime_project_ask(runtime, ask);
    if !runtime
        .get("timeline")
        .map(|value| value.is_array())
        .unwrap_or(false)
    {
        runtime["timeline"] = serde_json::json!([]);
    }
    let event = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "ts": Utc::now().to_rfc3339(),
        "kind": "worker.ask_user",
        "title": format!("{role} needs user direction"),
        "detail": trim_runtime_text(&question, 1100),
        "actor": agent_id,
        "phase": phase,
        "status": "waiting_user",
        "data": {
            "run_id": run_id,
            "worker_id": worker_id,
            "agent_id": agent_id,
            "ask_id": ask_id,
            "options": options,
        },
    });
    if let Some(items) = runtime
        .get_mut("timeline")
        .and_then(|value| value.as_array_mut())
    {
        items.push(event);
        if items.len() > TIMELINE_LIMIT {
            let drain = items.len() - TIMELINE_LIMIT;
            items.drain(0..drain);
        }
    }
}

pub(crate) fn mark_runtime_project_ask_answered(
    runtime: &mut serde_json::Value,
    ask_id_or_prefix: &str,
    answer: &str,
    delivery: &str,
) -> Result<RuntimeProjectAskAnswer, String> {
    ensure_runtime_question_store(runtime);
    let index = resolve_runtime_question_index(runtime, ask_id_or_prefix)?;
    let item = runtime_question_mut(runtime, index)?;
    let options = runtime_question_options(item);
    let was_pending = runtime_question_was_pending(item);
    let status = runtime_question_status(item);
    let ask_id = runtime_question_field(item, "ask_id", ask_id_or_prefix);
    if !was_pending {
        return Err(format!(
            "Project question [{}] is already {status}.",
            short_id(&ask_id)
        ));
    }
    let normalized = normalize_project_ask_answer(answer, options.as_deref())?;
    let phase = runtime_question_field(item, "phase", "");
    let question = runtime_question_field(item, "question", "");
    write_runtime_question_answer(item, &normalized, delivery);

    Ok(RuntimeProjectAskAnswer {
        ask_id,
        phase,
        question,
        answer: normalized,
        was_pending,
    })
}

fn runtime_question_mut(
    runtime: &mut serde_json::Value,
    index: usize,
) -> Result<&mut serde_json::Value, String> {
    let Some(items) = runtime
        .get_mut("user_questions")
        .and_then(|value| value.as_array_mut())
    else {
        return Err("Project runtime has no user question store.".to_string());
    };
    items
        .get_mut(index)
        .ok_or_else(|| "Project runtime question disappeared while answering.".to_string())
}

fn runtime_question_options(item: &serde_json::Value) -> Option<Vec<String>> {
    item.get("options")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
}

fn runtime_question_was_pending(item: &serde_json::Value) -> bool {
    item.get("status").and_then(|value| value.as_str()) == Some("pending")
}

fn runtime_question_status(item: &serde_json::Value) -> String {
    runtime_question_field(item, "status", "pending")
}

fn runtime_question_field(item: &serde_json::Value, field: &str, fallback: &str) -> String {
    item.get(field)
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
        .to_string()
}

fn write_runtime_question_answer(item: &mut serde_json::Value, normalized: &str, delivery: &str) {
    let now = Utc::now().to_rfc3339();
    if let Some(obj) = item.as_object_mut() {
        obj.insert("status".to_string(), serde_json::json!("answered"));
        obj.insert("answer".to_string(), serde_json::json!(normalized));
        obj.insert("answered_at".to_string(), serde_json::json!(now.clone()));
        obj.insert("updated_at".to_string(), serde_json::json!(now));
        obj.insert("delivery".to_string(), serde_json::json!(delivery));
    }
}

pub(crate) fn close_runtime_project_asks_for_run(runtime: &mut serde_json::Value, run_id: &str) {
    let now = Utc::now().to_rfc3339();
    if let Some(items) = runtime
        .get_mut("user_questions")
        .and_then(|value| value.as_array_mut())
    {
        for item in items {
            let same_run = item.get("run_id").and_then(|value| value.as_str()) == Some(run_id);
            let pending = item.get("status").and_then(|value| value.as_str()) == Some("pending");
            if same_run && pending {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("status".to_string(), serde_json::json!("closed"));
                    obj.insert("closed_at".to_string(), serde_json::json!(now.clone()));
                    obj.insert("updated_at".to_string(), serde_json::json!(now.clone()));
                    obj.insert("delivery".to_string(), serde_json::json!("run_completed"));
                }
            }
        }
    }
}

pub(crate) fn append_runtime_project_ask_answer_event(
    runtime: &mut serde_json::Value,
    answer: &RuntimeProjectAskAnswer,
    actor: &str,
) {
    if !runtime
        .get("timeline")
        .map(|value| value.is_array())
        .unwrap_or(false)
    {
        runtime["timeline"] = serde_json::json!([]);
    }
    let event = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "ts": Utc::now().to_rfc3339(),
        "kind": "worker.ask_user_answered",
        "title": "User answered project question",
        "detail": trim_runtime_text(&answer.answer, 900),
        "actor": actor,
        "phase": answer.phase,
        "status": "answered",
        "data": {
            "ask_id": answer.ask_id,
            "question": trim_runtime_text(&answer.question, 900),
            "was_pending": answer.was_pending,
        },
    });
    if let Some(items) = runtime
        .get_mut("timeline")
        .and_then(|value| value.as_array_mut())
    {
        items.push(event);
        if items.len() > TIMELINE_LIMIT {
            let drain = items.len() - TIMELINE_LIMIT;
            items.drain(0..drain);
        }
    }
}

pub(crate) fn runtime_user_questions_context(
    runtime: &serde_json::Value,
    current_phase: &str,
) -> String {
    let Some(items) = runtime
        .get("user_questions")
        .and_then(|value| value.as_array())
    else {
        return NO_USER_QUESTIONS_CONTEXT.to_string();
    };
    let mut lines = Vec::new();
    let mut current_pending = false;
    for item in items.iter().rev().take(8).rev() {
        if runtime_question_context_is_current_pending(item, current_phase) {
            current_pending = true;
        }
        lines.push(runtime_question_context_line(item));
    }
    if lines.is_empty() {
        return NO_USER_QUESTIONS_CONTEXT.to_string();
    }
    if current_pending {
        lines.insert(
            0,
            "Current phase has a pending user question. Do not guess; return STATUS: blocked until it is answered.".to_string(),
        );
    }
    lines.join("\n")
}

fn runtime_question_context_is_current_pending(
    item: &serde_json::Value,
    current_phase: &str,
) -> bool {
    let phase = item
        .get("phase")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let status = item
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("pending");
    phase == current_phase && status == "pending"
}

fn runtime_question_context_line(item: &serde_json::Value) -> String {
    let ask_id = item
        .get("ask_id")
        .and_then(|value| value.as_str())
        .map(short_id)
        .unwrap_or_else(|| "unknown".to_string());
    let phase = item
        .get("phase")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let status = item
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("pending");
    let question = item
        .get("question")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let mut line = format!(
        "- [{ask_id}] {phase}/{status}: {}",
        trim_runtime_text(question, 260)
    );
    if let Some(answer) = runtime_question_context_answer(item) {
        line.push_str(&answer);
    }
    if let Some(options) = runtime_question_context_options(item) {
        line.push_str(&options);
    }
    line
}

fn runtime_question_context_answer(item: &serde_json::Value) -> Option<String> {
    item.get("answer")
        .and_then(|value| value.as_str())
        .map(|answer| format!(" | answer: {}", trim_runtime_text(answer, 220)))
}

fn runtime_question_context_options(item: &serde_json::Value) -> Option<String> {
    let labels = item
        .get("options")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str())
        .take(4)
        .enumerate()
        .map(|(idx, option)| format!("{}. {}", idx + 1, trim_runtime_text(option, 80)))
        .collect::<Vec<_>>();
    (!labels.is_empty()).then(|| format!(" | options: {}", labels.join(" / ")))
}

fn resolve_runtime_question_index(
    runtime: &serde_json::Value,
    ask_id_or_prefix: &str,
) -> Result<usize, String> {
    let prefix = ask_id_or_prefix.trim();
    if prefix.is_empty() {
        return Err("Project question id is missing.".to_string());
    }
    let matches = runtime
        .get("user_questions")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .enumerate()
                .filter_map(|(idx, item)| {
                    item.get("ask_id")
                        .and_then(|value| value.as_str())
                        .filter(|ask_id| ask_id.starts_with(prefix))
                        .map(|_| idx)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    match matches.len() {
        0 => Err(format!("No recorded project question matches '{prefix}'.")),
        1 => Ok(matches[0]),
        n => Err(format!(
            "{n} recorded project questions match '{prefix}'. Use more characters."
        )),
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

#[cfg(test)]
#[path = "project_runtime_asks_tests.rs"]
mod project_runtime_asks_tests;
