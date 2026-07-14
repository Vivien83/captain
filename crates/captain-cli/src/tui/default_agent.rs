use serde_json::Value;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AgentPreference<'a> {
    pub(crate) id: Option<&'a str>,
    pub(crate) name: Option<&'a str>,
}

pub(crate) fn select_index<T>(
    agents: &[T],
    preference: AgentPreference<'_>,
    mut id_matches: impl FnMut(&T, &str) -> bool,
    mut name_of: impl FnMut(&T) -> Option<&str>,
) -> Option<usize> {
    if let Some(agent_id) = preference.id {
        if let Some(index) = agents.iter().position(|agent| id_matches(agent, agent_id)) {
            return Some(index);
        }
    }

    if let Some(agent_name) = preference.name {
        if let Some(index) = agents.iter().position(|agent| {
            name_of(agent)
                .map(|name| name.eq_ignore_ascii_case(agent_name))
                .unwrap_or(false)
        }) {
            return Some(index);
        }
    }

    agents
        .iter()
        .position(|agent| {
            name_of(agent)
                .map(|name| name.eq_ignore_ascii_case("captain"))
                .unwrap_or(false)
        })
        .or_else(|| (!agents.is_empty()).then_some(0))
}

pub(crate) fn daemon_agent_identity(agent: &Value) -> Option<(String, String)> {
    let id = agent.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return None;
    }

    let name = agent.get("name").and_then(Value::as_str).unwrap_or("agent");
    Some((id.to_string(), name.to_string()))
}

#[cfg(test)]
#[path = "default_agent/tests.rs"]
mod tests;
