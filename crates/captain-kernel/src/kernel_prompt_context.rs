use std::path::Path;

use captain_memory::MemorySubstrate;
use captain_types::config::AssistantConfig;
use tracing::warn;

/// Read a workspace identity file with a size cap to prevent prompt stuffing.
/// Returns None if the file doesn't exist or is empty.
pub(super) fn read_identity_file(workspace: &Path, filename: &str) -> Option<String> {
    const MAX_IDENTITY_FILE_BYTES: usize = 32_768; // 32KB cap
    let path = workspace.join(filename);
    // Security: ensure path stays inside workspace
    match path.canonicalize() {
        Ok(canonical) => {
            if let Ok(ws_canonical) = workspace.canonicalize() {
                if !canonical.starts_with(&ws_canonical) {
                    return None; // path traversal attempt
                }
            }
        }
        Err(_) => return None, // file doesn't exist
    }
    let content = std::fs::read_to_string(&path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    if content.len() > MAX_IDENTITY_FILE_BYTES {
        Some(captain_types::truncate_str(&content, MAX_IDENTITY_FILE_BYTES).to_string())
    } else {
        Some(content)
    }
}

pub(super) fn workspace_prompt_file_has_product_content(filename: &str, content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }

    match filename {
        // Retired: MemPalace/canonical context/recent journal are the memory
        // sources. Injecting workspace MEMORY.md reintroduces stale snapshots.
        "MEMORY.md" | "BOOTSTRAP.md" | "PLAYBOOK.md" | "USER.md" => false,
        "TOOLS.md" => {
            !trimmed.contains("Agent-specific environment notes")
                && trimmed != "# Tools & Environment"
        }
        "AGENTS.md" => {
            let lower = trimmed.to_ascii_lowercase();
            let generated_agent_rules = lower.contains("# agent behavioral guidelines")
                && (lower.contains("## memory (mandatory)")
                    || lower.contains("## memory journal")
                    || lower.contains("update memory.md after significant actions"));
            !generated_agent_rules
        }
        _ => true,
    }
}

pub(super) fn read_workspace_prompt_file(workspace: &Path, filename: &str) -> Option<String> {
    read_identity_file(workspace, filename)
        .filter(|content| workspace_prompt_file_has_product_content(filename, content))
}

pub(super) fn filter_prompt_memory_context(
    content: Option<String>,
    retractions: &[captain_runtime::memory_retractions::MemoryRetraction],
) -> Option<String> {
    captain_runtime::memory_retractions::filter_optional_text(content, retractions)
}

pub(super) fn build_persistent_memory_capsule(
    memory: &MemorySubstrate,
    retractions: &[captain_runtime::memory_retractions::MemoryRetraction],
) -> Option<String> {
    let conn = memory.usage_conn();
    let guard = match conn.lock() {
        Ok(g) => g,
        Err(e) => {
            warn!(error = %e, "persistent memory capsule: sqlite lock failed");
            return None;
        }
    };
    let capsule = match captain_memory::memory_capsule::build_from_writes(
        &guard,
        captain_memory::memory_capsule::DEFAULT_MAX_ITEMS,
        captain_memory::memory_capsule::DEFAULT_MAX_CHARS,
    ) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "persistent memory capsule: build failed");
            None
        }
    };
    filter_prompt_memory_context(capsule, retractions)
}

pub(super) fn asks_for_error_diagnosis(text: &str) -> bool {
    let lower = text.to_lowercase();
    let asks_why = lower.contains("pourquoi")
        || lower.contains("why")
        || lower.contains("what happened")
        || lower.contains("que s'est")
        || lower.contains("qu'est-ce qui s'est");
    let mentions_error = lower.contains("erreur")
        || lower.contains("errors")
        || lower.contains("error")
        || lower.contains("échou")
        || lower.contains("echou")
        || lower.contains("failed")
        || lower.contains("fail")
        || lower.contains("bug")
        || lower.contains("probl");

    asks_why && mentions_error
}

fn classify_recent_tool_error(content: &str) -> String {
    let lower = content.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
        "timeout".to_string()
    } else if lower.contains("rate limit") || lower.contains("429") {
        "rate_limit".to_string()
    } else if lower.contains("unauthorized")
        || lower.contains("authentication")
        || lower.contains("api key")
        || lower.contains("billing")
        || lower.contains("quota")
    {
        "auth_or_billing".to_string()
    } else if lower.contains("permission") || lower.contains("approval") || lower.contains("denied")
    {
        "permission_or_approval".to_string()
    } else if lower.contains("context")
        || lower.contains("too many tokens")
        || lower.contains("token limit")
    {
        "context_limit".to_string()
    } else if lower.contains("max iterations") {
        "max_iterations".to_string()
    } else if lower.contains("not found") {
        "not_found".to_string()
    } else {
        let first_line = content
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let mut preview = captain_runtime::str_utils::safe_truncate_str(first_line.trim(), 180);
        if preview.contains('{') || preview.contains('}') {
            preview = "raw_error_hidden_json_payload";
        }
        format!("other:{preview}")
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn recent_session_diagnostic_context(session: &captain_memory::session::Session) -> String {
    use captain_types::message::{ContentBlock, MessageContent};

    let recent_window = 28;
    let start = session.messages.len().saturating_sub(recent_window);
    let mut failures = Vec::new();
    let mut successes = Vec::new();
    let mut requested_tools = Vec::new();

    for msg in &session.messages[start..] {
        let MessageContent::Blocks(blocks) = &msg.content else {
            continue;
        };
        for block in blocks {
            match block {
                ContentBlock::ToolUse { name, .. } => {
                    push_unique(&mut requested_tools, name.clone());
                }
                ContentBlock::ToolResult {
                    tool_name,
                    content,
                    is_error,
                    ..
                } => {
                    let name = if tool_name.trim().is_empty() {
                        "unknown_tool"
                    } else {
                        tool_name.as_str()
                    };
                    if *is_error {
                        push_unique(
                            &mut failures,
                            format!("{name}: {}", classify_recent_tool_error(content)),
                        );
                    } else {
                        push_unique(&mut successes, name.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    let mut lines = vec![format!(
        "- Messages recents inspectes: {} sur {}.",
        session.messages.len().saturating_sub(start),
        session.messages.len()
    )];

    if failures.is_empty() {
        lines.push("- Echecs outil recents visibles: aucun.".to_string());
    } else {
        lines.push(format!(
            "- Echecs outil recents visibles: {}.",
            failures.join("; ")
        ));
    }

    if !successes.is_empty() {
        let tail: Vec<&str> = successes.iter().rev().take(8).map(String::as_str).collect();
        lines.push(format!(
            "- Derniers outils termines avec succes: {}.",
            tail.into_iter().rev().collect::<Vec<_>>().join(", ")
        ));
    } else if !requested_tools.is_empty() {
        let tail: Vec<&str> = requested_tools
            .iter()
            .rev()
            .take(8)
            .map(String::as_str)
            .collect();
        lines.push(format!(
            "- Derniers outils demandes: {}.",
            tail.into_iter().rev().collect::<Vec<_>>().join(", ")
        ));
    }

    lines.join("\n")
}

pub(super) fn append_turn_diagnostic_context(
    canonical_context_msg: Option<String>,
    session: &captain_memory::session::Session,
    current_user_message: &str,
) -> Option<String> {
    if !asks_for_error_diagnosis(current_user_message) {
        return canonical_context_msg;
    }

    let diagnostic = format!(
        "[Contexte diagnostic automatique - ce n'est pas une nouvelle demande utilisateur]\n\
         Utilise uniquement ces faits pour expliquer les erreurs recentes. \
         Si aucun echec n'est liste, dis clairement qu'aucun echec outil recent n'est visible au lieu d'inventer une cause.\n\n{}",
        recent_session_diagnostic_context(session)
    );

    match canonical_context_msg {
        Some(mut existing) if !existing.trim().is_empty() => {
            existing.push_str("\n\n");
            existing.push_str(&diagnostic);
            Some(existing)
        }
        _ => Some(diagnostic),
    }
}

pub(super) fn system_prompt_with_runtime_update(base: &str, notice: Option<String>) -> String {
    match notice {
        Some(notice) if !notice.trim().is_empty() => format!("{base}\n\n{notice}"),
        _ => base.to_string(),
    }
}

fn prompt_safe_single_line(value: &str, fallback: &str, max_chars: usize) -> String {
    let sanitized: String = value
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn assistant_style_guidance(style: &str) -> (&'static str, &'static str) {
    match style.trim().to_ascii_lowercase().as_str() {
        "balanced" | "natural" | "naturel" => (
            "balanced",
            "Warm, concise, proactive, and precise. Prefer useful action over long explanations.",
        ),
        "concise" | "court" => (
            "concise",
            "Answer directly in short paragraphs. Avoid recap unless it prevents ambiguity.",
        ),
        "professional" | "formal" | "formel" | "poli" => (
            "professional",
            "Use polished, precise language. Stay courteous, structured, and calm.",
        ),
        "developer" | "dev" | "engineer" => (
            "developer",
            "Be implementation-oriented: name files, commands, tradeoffs, and verification steps when relevant.",
        ),
        "friendly" | "pote" | "compagnon" | "companion" => (
            "friendly",
            "Be relaxed, human, and encouraging while staying efficient and technically honest.",
        ),
        "classic" | "assistant" | "neutral" | "neutre" => (
            "classic",
            "Use a neutral assistant tone: clear, helpful, and unobtrusive.",
        ),
        _ => (
            "custom",
            "Follow the user's configured custom style label while staying helpful, secure, and precise.",
        ),
    }
}

pub(super) fn assistant_style_context(
    assistant: &AssistantConfig,
    workspace_style_md: Option<String>,
) -> Option<String> {
    let display_name = prompt_safe_single_line(&assistant.display_name, "Captain", 64);
    let raw_style = prompt_safe_single_line(&assistant.style, "balanced", 48);
    let (style_id, guidance) = assistant_style_guidance(&raw_style);

    let mut sections = vec![format!(
        "Assistant identity\n- User-facing name: {display_name}\n- Internal routing slug: captain (keep this internal; do not rename tools, agents, or config paths unless asked).\n\nConfigured style: {style_id} ({raw_style})\n{guidance}"
    )];

    if let Some(style_md) = workspace_style_md.filter(|s| !s.trim().is_empty()) {
        sections.push(format!("Workspace STYLE.md\n{}", style_md.trim()));
    }

    Some(sections.join("\n\n"))
}

/// Compile-time copy of the agent-facing runtime changelog. Embedded so the
/// fingerprint can detect a changelog edit even when the binary's mtime/size
/// happens to round-trip identically (e.g. reproducible builds).
const RUNTIME_CHANGELOG_DOC: &str =
    include_str!("../../../docs/captain-tools/runtime-changelog.md");

pub(super) fn runtime_binary_fingerprint() -> String {
    let version = captain_types::version::captain_version();
    let changelog_hash = blake3::hash(RUNTIME_CHANGELOG_DOC.as_bytes()).to_hex();
    let mut parts = vec![
        format!("version={version}"),
        format!("changelog={}", &changelog_hash.as_str()[..16]),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(meta) = std::fs::metadata(exe) {
            parts.push(format!("size={}", meta.len()));
            if let Ok(modified) = meta.modified() {
                if let Ok(delta) = modified.duration_since(std::time::UNIX_EPOCH) {
                    parts.push(format!("mtime={}", delta.as_secs()));
                }
            }
        }
    }
    parts.join(";")
}

/// Extract learned rules from FEEDBACK.jsonl.
/// Focuses on negative feedback with corrections — those are the most actionable.
/// Returns a compact bullet list (max ~500 chars) for injection into the system prompt.
pub(super) fn extract_feedback_rules(feedback_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(feedback_path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    let mut rules: Vec<String> = Vec::new();
    let mut total_len = 0;
    // Process newest first (most relevant corrections)
    for line in content.lines().rev() {
        if total_len > 400 {
            break;
        }
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let rating = entry.get("rating").and_then(|v| v.as_i64()).unwrap_or(0);
            if rating >= 0 {
                continue; // Only learn from negative feedback
            }
            // Prefer corrections (user wrote what they expected)
            if let Some(correction) = entry.get("correction").and_then(|v| v.as_str()) {
                if !correction.trim().is_empty() {
                    let rule = format!("- {}", correction.trim());
                    total_len += rule.len();
                    rules.push(rule);
                    continue;
                }
            }
            // Fallback: extract context from user prompt + agent response
            if let Some(prompt) = entry.get("user_prompt").and_then(|v| v.as_str()) {
                if !prompt.trim().is_empty() {
                    let truncated = if prompt.len() > 80 {
                        &prompt[..80]
                    } else {
                        prompt
                    };
                    let rule = format!(
                        "- When asked \"{}\": response was disliked",
                        truncated.trim()
                    );
                    total_len += rule.len();
                    rules.push(rule);
                }
            }
        }
    }
    if rules.is_empty() {
        return None;
    }
    // Number rules for clarity
    let numbered: Vec<String> = rules
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {}", i + 1, r.trim_start_matches("- ")))
        .collect();
    Some(numbered.join("\n"))
}
