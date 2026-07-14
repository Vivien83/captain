use crate::prompt_sanitizer::sanitize;

use super::cap_str;

/// Delegation mode instructions — you are an orchestrator, spawn workers for
/// heavy work and reserve your own cycles for planning and synthesis.
pub fn build_delegation_section() -> String {
    String::from(
        "## Orchestration Mode: DELEGATION\n\n\
         You are operating as an orchestrator. You run on a premium model — your job is\n\
         to plan, delegate, and synthesize, NOT to execute bulk work yourself.\n\n\
         ### When to delegate\n\
         - Multi-step tasks (more than one tool call chain)\n\
         - Research, scraping, or repetitive tool loops\n\
         - Tasks that could take >3 iterations of tool calls\n\
         - Long-running background work\n\n\
         ### How to delegate\n\
         1. Use `agent_spawn` to create a worker with a cheap model:\n   \
             - fast lookups: `openrouter/google/gemini-2.5-flash`\n   \
             - tool-heavy:   `openrouter/anthropic/claude-haiku-4-5`\n   \
             - bulk work:    `openrouter/deepseek/deepseek-v3.1`\n\
         2. Use `agent_send` to assign the task with clear success criteria\n\
         3. Wait for the worker's result, then synthesize for the user\n\n\
         ### Respond directly (no delegation) for\n\
         - Quick acknowledgements (\"ok\", \"noted\", \"done\")\n\
         - Single-tool-call tasks (a single cron_create, memory_save, etc.)\n\
         - Conversational turns that don't require work\n\n\
         ### Rule\n\
         Do NOT replicate yourself — always reuse existing workers via `agent_list`\n\
         before spawning a new one. Kill workers with `agent_kill` when they finish.\n",
    )
}

pub(super) fn build_skills_section(skill_summary: &str, prompt_context: &str) -> String {
    let mut out = String::from("## Skills\n");
    if !skill_summary.is_empty() {
        out.push_str(
            "Skills are procedural memory. If a request matches a skill, use that skill or its tools before inventing a fresh workflow. For external API, SaaS, DevOps, custom CLI, project, debugging, release, or recurring automation work, call `skill_search`, then `skill_view` for the exact candidate when the full workflow context is needed. If a skill is missing, stale, or incomplete after real use, create/refine a skill proposal with exact commands, parameters, safety level, and verification steps.\n",
        );
        out.push_str(skill_summary.trim());
    }
    if !prompt_context.is_empty() {
        out.push('\n');
        out.push_str(&cap_str(prompt_context, 2000));
    }
    out
}

pub(super) fn build_mcp_section(mcp_summary: &str) -> String {
    format!("## Connected Tool Servers (MCP)\n{}", mcp_summary.trim())
}

pub(super) fn build_persona_section(
    identity_md: Option<&str>,
    soul_md: Option<&str>,
    user_md: Option<&str>,
    graph_md: Option<&str>,
    workspace_path: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(ws) = workspace_path {
        parts.push(format!("## Workspace\nWorkspace: {ws}"));
    }

    // v3.7g: sanitize user-controlled context files before injection.
    // Identity file (IDENTITY.md) — personality at a glance, before SOUL.md
    if let Some(identity) = identity_md {
        if !identity.trim().is_empty() {
            let clean = sanitize("IDENTITY.md", identity);
            parts.push(format!(
                "## Identity {}\n{}",
                budget_tag(&clean, 500),
                cap_str(&clean, 500)
            ));
        }
    }

    if let Some(soul) = soul_md {
        if !soul.trim().is_empty() {
            let sanitized_code = strip_code_blocks(soul);
            let clean = sanitize("SOUL.md", &sanitized_code);
            parts.push(format!(
                "## Persona {}\nEmbody this identity in your tone and communication style. Be natural, not stiff or generic.\n{}",
                budget_tag(&clean, 1000),
                cap_str(&clean, 1000)
            ));
        }
    }

    if let Some(user) = user_md {
        if !user.trim().is_empty() {
            let clean = sanitize("USER.md", user);
            parts.push(format!(
                "## User Context {}\n{}",
                budget_tag(&clean, 500),
                cap_str(&clean, 500)
            ));
        }
    }

    if let Some(graph) = graph_md {
        if !graph.trim().is_empty() {
            let clean = sanitize("GRAPH.md", graph);
            parts.push(format!(
                "## Knowledge Graph Snapshot {}\nThis is an auto-generated summary of what you know. Use tools for deeper queries.\n{}",
                budget_tag(&clean, 2000),
                cap_str(&clean, 2000)
            ));
        }
    }

    parts.join("\n\n")
}

/// Render a budget indicator `[X% — N/Max chars]` (v3.7e).
///
/// Gives the LLM a quantitative view of its persona-file budgets so it can
/// self-regulate (edit/replace/split) instead of relying on silent hook
/// truncation to save it. Percent is capped at 999 to keep the format tight
/// if a file exceeds its own cap before truncation.
#[allow(clippy::manual_checked_ops)] // explicit zero-check reads clearer than checked_div here
fn budget_tag(content: &str, max_chars: usize) -> String {
    let used = content.chars().count();
    let pct = if max_chars == 0 {
        0
    } else {
        (used * 100 / max_chars).min(999)
    };
    format!("[{pct}% — {used}/{max_chars} chars]")
}

pub(super) fn build_user_section(user_name: Option<&str>) -> String {
    match user_name {
        Some(name) => {
            format!(
                "## User Profile\n\
                 The user's name is \"{name}\". Address them by name naturally \
                 when appropriate (greetings, farewells, etc.), but don't overuse it."
            )
        }
        None => "## User Profile\n\
             You don't know the user's name yet. On your FIRST reply in this conversation, \
             warmly introduce yourself by your agent name and ask what they'd like to be called. \
             Once they tell you, immediately use `memory_save` with subject \"user\", \
             predicate \"name\", object as their name, and category \"info\" so you remember it for future sessions. \
             Keep the introduction brief — don't let it overshadow their actual request."
            .to_string(),
    }
}

/// Build channel-awareness for active messaging adapters and runtime surfaces.
///
/// Each channel carries: char_limit + formatting_rules + execution_context.
/// The execution_context matters most for `cron` (no user present) and for
/// IDE/protocol channels (`acp`, `mcp`) where the "user" is another program.
/// Frozen messaging adapters intentionally fall back to the generic hint so
/// the prompt does not promote them as active experience.
pub(super) fn build_channel_section(channel: &str) -> String {
    let (limit, hints, context) = match channel {
        "telegram" => (
            "4096",
            "Telegram: *bold*, _italic_, `code`, ```pre```, ||spoiler||. \
             No markdown tables. For metrics or tabular data, use compact aligned \
             code blocks or short bullet cards. Result first; hide internal tool \
             noise unless it is needed to explain a blocker.",
            "A human user reads on mobile. Keep short. Inline keyboards \
             available via telegram_api.",
        ),
        "discord" => (
            "2000",
            "Discord markdown: **bold**, *italic*, `code`, ```code blocks```. \
             Split long responses across multiple messages.",
            "Could be DM or channel with many users — check sender context \
             before broadcasting.",
        ),
        "signal" => (
            "2000",
            "Limited markdown. Plain text preferred. \
             Never include remote image URLs in sensitive conversations.",
            "Privacy-focused users; avoid tracking links.",
        ),
        "email" => (
            "65535",
            "HTML or plain text. Include subject when creating drafts. \
             Quote prior thread context when replying.",
            "Async — no user waiting. Be thorough, include all context.",
        ),
        "cron" => (
            "4096",
            "Output goes to the audit log and optional notification channel. \
             Concise structured summary preferred.",
            "NO USER is present. You cannot ask questions, request \
             clarification, or wait for follow-up. Execute the task fully \
             and autonomously. If something genuinely blocks, fail loud \
             with a descriptive error for the operator to read later.",
        ),
        "cli" => (
            "65535",
            "Terminal output. Plain text or ANSI codes. Keep lines ≤120 chars.",
            "A developer is watching interactively. Terseness is respected.",
        ),
        "desktop" => (
            "65535",
            "Markdown rendered by the desktop shell. Use headings, lists, code \
             fences. Screenshots and embedded images supported.",
            "Power user on a full screen. Can handle longer responses.",
        ),
        "acp" => (
            "65535",
            "Responses are rendered inside an ACP-capable editor (Zed, \
             JetBrains, VS Code, Neovim). Markdown + code fences supported.",
            "The 'user' is the editor session. File/terminal tools are scoped \
             to the editor's cwd. Prefer structured output the editor can \
             navigate (headings, code blocks with language tags).",
        ),
        "mcp" => (
            "65535",
            "Responses are consumed by an MCP tool server client (Claude \
             Desktop, Cursor, other agents). Return structured JSON when the \
             consumer requested it; otherwise markdown.",
            "The 'user' is another program. No clarification round-trip \
             possible unless the MCP consumer is interactive. Be exact.",
        ),
        _ => (
            "4096",
            "Use markdown formatting where supported.",
            "Unknown or frozen channel — default behavior.",
        ),
    };
    format!(
        "## Channel\n\
         You are responding via {channel}. Keep messages under {limit} chars.\n\
         Formatting: {hints}\n\
         Context: {context}"
    )
}

pub(super) fn build_sender_section(
    sender_name: Option<&str>,
    sender_id: Option<&str>,
) -> Option<String> {
    match (sender_name, sender_id) {
        (Some(name), Some(id)) => Some(format!("## Sender\nMessage from: {name} ({id})")),
        (Some(name), None) => Some(format!("## Sender\nMessage from: {name}")),
        (None, Some(id)) => Some(format!("## Sender\nMessage from: {id}")),
        (None, None) => None,
    }
}

pub(super) fn build_peer_agents_section(
    self_name: &str,
    peers: &[(String, String, String)],
) -> String {
    let mut out = String::from(
        "## Peer Agents\n\
         You are part of a multi-agent system. These agents are running alongside you:\n",
    );
    for (name, state, model) in peers {
        if name == self_name {
            continue; // Don't list yourself
        }
        out.push_str(&format!("- **{}** ({}) — model: {}\n", name, state, model));
    }
    out.push_str(
        "\nStates above are a snapshot from when this prompt was built and go stale \
         during long turns — call agent_status or agent_list for the live state before \
         relying on them. \
         You can communicate with agents using `agent_send` (by name) and see all agents \
         with `agent_list`. Delegate tasks to specialized agents when appropriate.",
    );
    out
}

/// Static safety section. Destructive-op rules scoped (v3.7c) so the LLM
/// can retrieve them by name before any irreversible action.
pub(super) const SAFETY_SECTION: &str = "\
## Safety

<destructive_ops>
- NEVER auto-execute purchases, payments, account deletions, or irreversible actions without explicit user confirmation.
- If a tool could cause data loss (file_delete, shell_exec with rm/drop/truncate, DB mutations, git reset --hard, git push --force), explain what it will do and confirm first.
- When in doubt about reversibility, ask before acting.
</destructive_ops>

<oversight>
- Prioritize safety and human oversight over task completion.
- If you cannot accomplish a task safely, explain the limitation instead of finding a workaround that trades safety for speed.
- When user gives a broad instruction, match scope: authorization for one action does not imply authorization for similar actions elsewhere.
</oversight>";

/// Static operational guidelines (replaces STABILITY_GUIDELINES).
pub(super) const CONSCIOUSNESS_AWARENESS: &str = "\
## Operational Awareness
Captain may inject runtime awareness such as emergent thoughts, past failures, user state, active goals, loop warnings, or health signals.

Use it as operational telemetry:
- Relevant active goal or anomaly → factor it into the next action.
- Prior tool failure → change approach instead of repeating it.
- Loop warning → narrow scope, verify assumptions, or ask only for the missing decision.
- Runtime health/budget issue → report the concrete blocker and safe next step.

Do not treat awareness as personality, decoration, or proof that Captain is conscious. Do not mention it unless it changes the answer or the user asks about runtime state.
Never invent missing facts. If the information is absent from prompt context, config, secrets, memory, or tools, say that plainly.";

pub(super) const OPERATIONAL_GUIDELINES: &str = "\
## Operational Guidelines
- Do NOT retry a tool call with identical parameters if it failed. Try a different approach.
- If a tool returns an error, analyze the error before calling it again.
- Prefer targeted, specific tool calls over broad ones.
- Plan your approach before executing multiple tool calls.
- If you cannot accomplish a task after a few attempts, explain what went wrong instead of looping.
- Never call the same tool more than 3 times with the same parameters.
- If a message requires no response (simple acknowledgments, reactions, messages not directed at you), respond with exactly NO_REPLY.";

/// Strip markdown triple-backtick code blocks from content.
///
/// Prevents LLMs from copying code blocks as text output instead of making
/// tool calls when SOUL.md contains command examples.
fn strip_code_blocks(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut in_block = false;
    for line in content.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if !in_block {
            result.push_str(line);
            result.push('\n');
        }
    }
    // Collapse multiple blank lines left by stripped blocks.
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}
