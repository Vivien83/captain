//! `/reasoning` control shared by standalone and full TUI surfaces.

use captain_kernel::CaptainKernel;
use captain_types::agent::AgentId;
use captain_types::reasoning::{AgentReasoningStatus, ReasoningSelectionSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReasoningCommand {
    Show,
    Set(Option<String>),
}

pub(crate) fn command_for(command: &str, args: &str) -> Option<Result<ReasoningCommand, String>> {
    if command != "/reasoning" {
        return None;
    }
    let args = args.trim();
    if args.is_empty() {
        return Some(Ok(ReasoningCommand::Show));
    }
    if args.split_whitespace().count() != 1 {
        return Some(Err(
            "Usage: /reasoning [auto|niveau] (ex. low, high, xhigh, max, ultra)".to_string(),
        ));
    }
    if args.eq_ignore_ascii_case("auto") {
        Some(Ok(ReasoningCommand::Set(None)))
    } else {
        Some(Ok(ReasoningCommand::Set(Some(args.to_ascii_lowercase()))))
    }
}

pub(crate) fn run_daemon(
    base_url: &str,
    agent_id: &str,
    command: &ReasoningCommand,
) -> Result<AgentReasoningStatus, String> {
    let client = crate::daemon_client();
    let url = format!("{base_url}/api/agents/{agent_id}/reasoning");
    let response = match command {
        ReasoningCommand::Show => client.get(url).send(),
        ReasoningCommand::Set(effort) => client
            .put(url)
            .json(&serde_json::json!({"effort": effort}))
            .send(),
    }
    .map_err(|error| format!("Contrôle du raisonnement indisponible : {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response
            .json::<serde_json::Value>()
            .ok()
            .and_then(|body| body["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| status.to_string());
        return Err(format!("Raisonnement refusé : {detail}"));
    }
    response
        .json::<AgentReasoningStatus>()
        .map_err(|error| format!("Réponse raisonnement invalide : {error}"))
}

pub(crate) fn run_inprocess(
    kernel: &CaptainKernel,
    agent_id: AgentId,
    command: &ReasoningCommand,
) -> Result<AgentReasoningStatus, String> {
    match command {
        ReasoningCommand::Show => kernel.agent_reasoning_status(agent_id),
        ReasoningCommand::Set(effort) => {
            kernel.set_agent_reasoning_effort(agent_id, effort.as_deref())
        }
    }
    .map_err(|error| error.to_string())
}

pub(crate) fn status_message(status: &AgentReasoningStatus) -> String {
    let model = format!("{}/{}", status.provider, status.model);
    if !status.supported {
        return format!(
            "Raisonnement non configurable pour `{model}`. Captain laisse le provider décider."
        );
    }
    let configured = status
        .configured_effort
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "auto".to_string());
    let effective = status
        .effective_effort
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "provider".to_string());
    let source = match status.source {
        ReasoningSelectionSource::AgentOverride => "override agent",
        ReasoningSelectionSource::ModelDefault => "défaut du modèle",
        ReasoningSelectionSource::ProviderDefault => "défaut provider",
        ReasoningSelectionSource::Unsupported => "non configurable",
    };
    let options = if status.options.is_empty() {
        "aucun niveau explicite publié ; Auto laisse le provider décider".to_string()
    } else {
        status
            .options
            .iter()
            .map(|option| option.effort.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let ultra_note = if status
        .options
        .iter()
        .any(|option| option.effort.as_str() == "ultra")
    {
        "\nPour Codex, `ultra` utilise l'effort modèle `max` et active la délégation proactive uniquement sur l'agent racine."
    } else {
        ""
    };
    format!(
        "Raisonnement `{model}` : sélection `{configured}` → effectif `{effective}` ({source}).\nNiveaux disponibles : {options}.\n`auto` omet l'override et laisse le modèle choisir ; `none`, s'il est proposé, est un niveau explicite.{ultra_note}"
    )
}

#[cfg(test)]
#[path = "slash_reasoning/tests.rs"]
mod tests;
