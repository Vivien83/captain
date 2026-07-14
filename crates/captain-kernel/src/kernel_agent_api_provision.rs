use captain_types::{
    agent::AgentId,
    agent_api::{
        failed_egress_report, generate_agent_api_callback_secret, generate_agent_api_token,
        pending_egress_report, ready_egress_report, ready_ingress_report, skipped_ingress_report,
        AgentApiEgressProvisionReport, AgentApiSpawnProvisionReport, AgentApiSpawnProvisionRequest,
    },
};

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) async fn handle_provision_spawned_agent_api(
        &self,
        agent_id: &str,
        request: AgentApiSpawnProvisionRequest,
    ) -> Result<AgentApiSpawnProvisionReport, String> {
        let agent_id: AgentId = agent_id
            .parse()
            .map_err(|_| "Invalid agent ID for agent API provisioning".to_string())?;
        if self.registry.get(agent_id).is_none() {
            return Err("Agent not found for agent API provisioning".to_string());
        }

        let mut actions = Vec::new();
        let ingress = if request.provision_ingress_token {
            let token = generate_agent_api_token();
            let token_env = captain_types::agent_api::agent_api_token_env(&agent_id);
            write_secret_env_value(
                &self.config.home_dir.join("secrets.env"),
                &token_env,
                &token,
            )
            .map_err(|err| format!("Failed to write ingress token: {err}"))?;
            std::env::set_var(token_env, &token);
            ready_ingress_report(&agent_id, token)
        } else {
            actions.push(format!(
                "Rotate ingress token with {} before external callers use the agent.",
                captain_types::agent_api::agent_api_token_rotate_url(&agent_id)
            ));
            skipped_ingress_report(&agent_id)
        };

        let egress = self.provision_spawned_agent_egress(&agent_id, request, &mut actions)?;
        Ok(AgentApiSpawnProvisionReport::new(
            &agent_id, ingress, egress, actions,
        ))
    }

    fn provision_spawned_agent_egress(
        &self,
        agent_id: &AgentId,
        request: AgentApiSpawnProvisionRequest,
        actions: &mut Vec<String>,
    ) -> Result<AgentApiEgressProvisionReport, String> {
        let Some(callback_url) = request
            .egress_callback_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
        else {
            actions.push(format!(
                "Configure signed callback egress with {} before treating the agent API as fully in/out ready.",
                captain_types::agent_api::agent_api_egress_configure_url(agent_id)
            ));
            return Ok(pending_egress_report(agent_id));
        };

        if let Err(issue) = validate_agent_api_callback_url(callback_url) {
            actions.push(format!("Fix egress callback configuration: {issue}"));
            return Ok(failed_egress_report(agent_id, issue));
        }

        let (secret, generated_secret) = match request.egress_callback_secret {
            Some(secret) if !secret.trim().is_empty() => (secret.trim().to_string(), false),
            _ if request.generate_callback_secret => (generate_agent_api_callback_secret(), true),
            _ => {
                let issue = "callback_secret is required when generate_callback_secret is false"
                    .to_string();
                actions.push(format!("Fix egress callback configuration: {issue}"));
                return Ok(failed_egress_report(agent_id, issue));
            }
        };
        if secret.len() < 16 {
            let issue = "callback_secret must be at least 16 characters".to_string();
            actions.push(format!("Fix egress callback configuration: {issue}"));
            return Ok(failed_egress_report(agent_id, issue));
        }

        let url_env = captain_types::agent_api::agent_api_callback_url_env(agent_id);
        let secret_env = captain_types::agent_api::agent_api_callback_secret_env(agent_id);
        let secrets_path = self.config.home_dir.join("secrets.env");
        write_secret_env_value(&secrets_path, &url_env, callback_url)
            .map_err(|err| format!("Failed to write callback URL: {err}"))?;
        write_secret_env_value(&secrets_path, &secret_env, &secret)
            .map_err(|err| format!("Failed to write callback secret: {err}"))?;
        std::env::set_var(url_env, callback_url);
        std::env::set_var(secret_env, &secret);

        Ok(ready_egress_report(
            agent_id,
            generated_secret.then_some(secret),
        ))
    }
}

fn write_secret_env_value(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    validate_secret_env_entry(key, value)?;
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|line| line.to_string())
            .collect()
    } else {
        Vec::new()
    };
    lines.retain(|line| !line.starts_with(&format!("{key}=")));
    lines.push(format!("{key}={value}"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, lines.join("\n") + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn validate_secret_env_entry(key: &str, value: &str) -> Result<(), std::io::Error> {
    if key.is_empty()
        || key.contains('=')
        || key.contains('\n')
        || key.contains('\r')
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret key must be a plain environment variable name",
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret value must be a single line",
        ));
    }
    Ok(())
}

/// See `captain_types::ssrf_guard` — the shared SSRF check outbound event
/// webhooks, agent-API egress callbacks, and this provisioning-time check
/// all delegate to. This used to be an independent copy that (unlike its
/// captain-api sibling) let `metadata.google.internal` through when the
/// local-testing escape hatch was set — exactly the kind of divergence
/// three copies of a security check invite.
fn validate_agent_api_callback_url(url: &str) -> Result<(), String> {
    captain_types::ssrf_guard::validate_outbound_callback_url(
        url,
        local_agent_api_callbacks_allowed(),
    )
}

fn local_agent_api_callbacks_allowed() -> bool {
    std::env::var("CAPTAIN_AGENT_API_ALLOW_LOCAL_CALLBACKS")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_env_writer_rejects_injection() {
        let path = tempfile::tempdir().unwrap().path().join("secrets.env");

        let err = write_secret_env_value(&path, "TOKEN", "secret\nOTHER=value").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn callback_url_rejects_localhost_by_default() {
        assert!(validate_agent_api_callback_url("http://localhost:7777/hook").is_err());
    }
}
