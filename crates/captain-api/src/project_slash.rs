//! `/project` slash command handler (v3.11d).
//!
//! Called from the WebSocket frame parser before the message would
//! otherwise reach the LLM. The handler returns a short human-readable
//! reply the WS layer sends straight back as a `response` frame — no
//! LLM turn, no token burn.

use captain_runtime::active_project::{global as active_project_registry, SlashCommand};
use std::sync::Arc;

use crate::project_active_input::{
    normalize_active_project_agent_id, normalize_active_project_slug,
};
use crate::project_storage_error::safe_project_storage_error;

const ACTIVE_PROJECT_UNAVAILABLE: &str =
    "Active project selection is unavailable; restart Captain.";

pub fn handle(
    kernel: &Arc<captain_kernel::CaptainKernel>,
    agent_id: &str,
    cmd: SlashCommand,
) -> String {
    let agent_id = match normalize_active_project_agent_id(agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return error.to_string(),
    };
    match cmd {
        SlashCommand::None => String::new(),
        SlashCommand::List => render_list(kernel, &agent_id),
        SlashCommand::Clear => {
            if let Some(reg) = active_project_registry() {
                if reg.clear(&agent_id) {
                    "Active project cleared.".into()
                } else {
                    "No active project to clear.".into()
                }
            } else {
                ACTIVE_PROJECT_UNAVAILABLE.into()
            }
        }
        SlashCommand::Switch(slug) => switch(kernel, &agent_id, slug),
    }
}

fn render_list(kernel: &Arc<captain_kernel::CaptainKernel>, agent_id: &str) -> String {
    let projects = match kernel.memory.project_list(false) {
        Ok(p) => p,
        Err(e) => return safe_project_storage_error(&e.to_string()),
    };
    if projects.is_empty() {
        return "No projects yet. Use `project_create` to start one.".into();
    }
    let current = active_project_registry().and_then(|r| r.get(agent_id));
    let mut out = String::from("Projects:\n");
    for p in &projects {
        let marker = if current.as_deref() == Some(p.slug.as_str()) {
            "→ "
        } else {
            "  "
        };
        out.push_str(&format!(
            "{marker}{}  · {}  [{}]\n",
            p.slug,
            if p.name.is_empty() {
                "(no name)"
            } else {
                &p.name
            },
            p.status.as_str()
        ));
    }
    if current.is_some() {
        out.push_str("\n(→ = currently active — use `/project clear` to detach)");
    } else {
        out.push_str("\n(no active project — use `/project <slug>` to switch)");
    }
    out
}

fn switch(kernel: &Arc<captain_kernel::CaptainKernel>, agent_id: &str, slug: String) -> String {
    let slug = match normalize_active_project_slug(slug) {
        Ok(slug) => slug,
        Err(error) => return error.to_string(),
    };
    match kernel.memory.project_find_by_slug(&slug) {
        Ok(Some(p)) => {
            let Some(reg) = active_project_registry() else {
                return ACTIVE_PROJECT_UNAVAILABLE.into();
            };
            reg.set(agent_id.to_string(), p.slug.clone());
            format!(
                "Active project → {} ({}). Use `project_resume` for context or `/project clear` to detach.",
                p.slug, p.status.as_str()
            )
        }
        Ok(None) => "Project not found. Try `/project list`.".to_string(),
        Err(e) => safe_project_storage_error(&e.to_string()),
    }
}
