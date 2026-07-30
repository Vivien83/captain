use captain_types::{
    agent::AgentId,
    agent_api::{
        failed_egress_report, generate_agent_api_callback_secret, generate_agent_api_token,
        pending_egress_report, ready_egress_report, ready_existing_ingress_report,
        ready_ingress_report, skipped_ingress_report, AgentApiEgressProvisionReport,
        AgentApiSpawnProvisionReport, AgentApiSpawnProvisionRequest,
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
            let token_env = captain_types::agent_api::agent_api_token_env(&agent_id);
            if self.credential_is_externally_managed(&token_env) {
                if self
                    .resolve_credential(&token_env)
                    .is_some_and(|token| token.len() >= 32)
                {
                    ready_existing_ingress_report(&agent_id)
                } else {
                    actions.push(format!(
                        "{token_env} is externally managed but unavailable or too short; fix its mounted file."
                    ));
                    skipped_ingress_report(&agent_id)
                }
            } else {
                let token = generate_agent_api_token();
                self.handle_secret_write(&token_env, &token)
                    .map_err(|err| format!("Failed to write ingress token: {err}"))?;
                ready_ingress_report(&agent_id, token)
            }
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
        let url_env = captain_types::agent_api::agent_api_callback_url_env(agent_id);
        let secret_env = captain_types::agent_api::agent_api_callback_secret_env(agent_id);
        if self.credential_is_externally_managed(&url_env)
            || self.credential_is_externally_managed(&secret_env)
        {
            let external_url = self.resolve_credential(&url_env);
            let external_secret = self.resolve_credential(&secret_env);
            let ready = external_url
                .as_deref()
                .is_some_and(|url| validate_agent_api_callback_url(url).is_ok())
                && external_secret
                    .as_deref()
                    .is_some_and(|value| value.len() >= 16);
            if ready {
                return Ok(ready_egress_report(agent_id, None));
            }
            let issue =
                "externally managed callback URL or secret is unavailable or invalid".to_string();
            actions.push(format!("Fix egress callback configuration: {issue}"));
            return Ok(failed_egress_report(agent_id, issue));
        }

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

        self.handle_secret_write(&url_env, callback_url)
            .map_err(|err| format!("Failed to write callback URL: {err}"))?;
        self.handle_secret_write(&secret_env, &secret)
            .map_err(|err| format!("Failed to write callback secret: {err}"))?;

        Ok(ready_egress_report(
            agent_id,
            generated_secret.then_some(secret),
        ))
    }
}

#[cfg(test)]
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
    let serialized = lines.join("\n") + "\n";
    captain_types::durable_fs::atomic_write(path, serialized.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
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
