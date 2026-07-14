use crate::i18n::{self, Lang};

pub(crate) enum StatusSnapshot<'a> {
    Daemon {
        base_url: &'a str,
        agent_name: Option<&'a str>,
    },
    InProcess {
        agent_count: usize,
        agent_name: Option<&'a str>,
    },
    Disconnected,
}

pub(crate) fn status_message(snapshot: StatusSnapshot<'_>, lang: Lang) -> String {
    let mut lines = Vec::new();
    match snapshot {
        StatusSnapshot::Daemon {
            base_url,
            agent_name,
        } => {
            lines.push(i18n::t("status.mode_daemon", lang).replace("{url}", base_url));
            if let Some(name) = agent_name {
                lines.push(i18n::t("status.current_agent", lang).replace("{name}", name));
            }
        }
        StatusSnapshot::InProcess {
            agent_count,
            agent_name,
        } => {
            lines.push(i18n::t("status.mode_inproc", lang).to_string());
            lines.push(
                i18n::t("status.agent_count", lang).replace("{count}", &agent_count.to_string()),
            );
            if let Some(name) = agent_name {
                lines.push(i18n::t("status.current_agent", lang).replace("{name}", name));
            }
        }
        StatusSnapshot::Disconnected => {
            lines.push(i18n::t("status.mode_disconnected", lang).to_string());
        }
    }
    lines.join("\n")
}

pub(crate) fn daemon_session_lines(body: &serde_json::Value, limit: usize) -> Vec<String> {
    crate::tui::session_runtime::session_values(body)
        .iter()
        .take(limit)
        .map(|session| {
            let label = session["label"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Session sans titre");
            let id = session["session_id"]
                .as_str()
                .or_else(|| session["id"].as_str())
                .unwrap_or("?");
            let short_id = id.get(..8).unwrap_or(id);
            let messages = session["message_count"].as_u64().unwrap_or(0);
            let updated = session["updated_at"]
                .as_str()
                .or_else(|| session["last_active"].as_str())
                .or_else(|| session["created"].as_str())
                .or_else(|| session["created_at"].as_str())
                .unwrap_or("");
            format!("{label} [{short_id}] \u{2014} {messages} msg \u{2014} {updated}")
        })
        .collect()
}

pub(crate) fn daemon_agent_lines(body: &serde_json::Value) -> Vec<String> {
    let Some(agents) = body.as_array() else {
        return Vec::new();
    };

    agents
        .iter()
        .map(|agent| {
            format!(
                "{} [{}] {}",
                agent["name"].as_str().unwrap_or("?"),
                agent["state"].as_str().unwrap_or("?"),
                agent["model_name"].as_str().unwrap_or("?"),
            )
        })
        .collect()
}

pub(crate) fn inprocess_agent_line(name: &str, state: &str, provider: &str, model: &str) -> String {
    format!("{name} [{state}] {provider}/{model}")
}

pub(crate) fn list_message(lines: Vec<String>, empty_message: &str) -> String {
    if lines.is_empty() {
        empty_message.to_string()
    } else {
        lines.join("\n")
    }
}

pub(crate) fn sessions_not_connected_message(lang: Lang) -> &'static str {
    i18n::t("sessions.not_connected", lang)
}

pub(crate) fn sessions_list_message(lines: Vec<String>, lang: Lang) -> String {
    list_message(lines, i18n::t("sessions.empty", lang))
}

pub(crate) fn agents_list_message(lines: Vec<String>, lang: Lang) -> String {
    list_message(lines, i18n::t("agents.empty", lang))
}

#[cfg(test)]
#[path = "slash_info/tests.rs"]
mod tests;
