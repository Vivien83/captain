use crate::project_update_input::normalize_project_slug;

const ACTIVE_PROJECT_AGENT_ID_LIMIT: usize = 120;

pub(crate) const ACTIVE_PROJECT_AGENT_ID_ERROR: &str = "agent_id is invalid";
pub(crate) const ACTIVE_PROJECT_NOT_FOUND_ERROR: &str = "project not found";

pub(crate) fn normalize_active_project_agent_id(agent_id: &str) -> Result<String, &'static str> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty()
        || agent_id.len() > ACTIVE_PROJECT_AGENT_ID_LIMIT
        || !agent_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ACTIVE_PROJECT_AGENT_ID_ERROR);
    }
    Ok(agent_id.to_string())
}

pub(crate) fn normalize_active_project_slug(slug: String) -> Result<String, &'static str> {
    normalize_project_slug(slug)
}
