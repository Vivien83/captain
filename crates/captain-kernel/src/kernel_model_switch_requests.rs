use crate::error::{KernelError, KernelResult};
use crate::model_switch::{ModelSwitchPlan, ModelSwitchSessionStrategy};
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::model_switch_pending::{
    parse_pending_model_switch_choice, pending_model_switch_key, PendingModelSwitchChoice,
};
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;

use super::{shared_memory_agent_id, CaptainKernel};

impl CaptainKernel {
    pub(super) fn consume_pending_model_switch_choice(
        &self,
        agent_id: AgentId,
        user_message: &str,
    ) -> KernelResult<Option<AgentLoopResult>> {
        let Some(choice) = parse_pending_model_switch_choice(user_message) else {
            return Ok(None);
        };

        let shared_id = shared_memory_agent_id();
        let key = pending_model_switch_key(&agent_id.to_string());
        let Some(pending) = self
            .memory
            .structured_get(shared_id, &key)
            .map_err(KernelError::Captain)?
        else {
            return Ok(None);
        };

        if pending.get("status").and_then(|v| v.as_str()) != Some("pending") {
            return Ok(None);
        }

        if pending
            .get("expires_at_unix")
            .and_then(|v| v.as_i64())
            .map(|expires| expires < chrono::Utc::now().timestamp())
            .unwrap_or(false)
        {
            let _ = self.memory.structured_delete(shared_id, &key);
            return Ok(Some(Self::empty_agent_loop_result(
                "Le choix de switch a expiré. Relance le changement de modèle et je le referai proprement.".to_string(),
            )));
        }

        if choice == PendingModelSwitchChoice::Cancel {
            let _ = self.memory.structured_delete(shared_id, &key);
            return Ok(Some(Self::empty_agent_loop_result(
                "Switch annulé. Je garde le modèle actuel.".to_string(),
            )));
        }

        let model = pending
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::Internal(
                    "Pending model switch is missing target model".to_string(),
                ))
            })?;
        let provider = pending.get("provider").and_then(|v| v.as_str());
        let strategy = choice
            .as_session_strategy()
            .and_then(|s| s.parse::<ModelSwitchSessionStrategy>().ok())
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::Internal(
                    "Invalid pending model switch choice".to_string(),
                ))
            })?;

        let result = self.apply_model_switch(agent_id, model, provider, strategy)?;
        let _ = self.memory.structured_delete(shared_id, &key);

        let session_label = match strategy {
            ModelSwitchSessionStrategy::NewSession => "nouvelle session démarrée",
            ModelSwitchSessionStrategy::CompactSession => {
                "contexte compacté puis nouvelle session démarrée"
            }
        };
        let response = format!(
            "✅ Switché sur **{} / {}** — {}.",
            result.plan.target_provider, result.plan.target_model, session_label
        );
        Ok(Some(Self::empty_agent_loop_result(response)))
    }

    pub(super) fn handle_direct_model_switch_request(
        &self,
        agent_id: AgentId,
        user_message: &str,
    ) -> KernelResult<Option<AgentLoopResult>> {
        let Some((provider, model)) = self.detect_direct_model_switch_target(user_message) else {
            return Ok(None);
        };

        let plan = self.plan_model_switch(agent_id, &model, Some(&provider))?;
        if !plan.can_apply {
            let issues = if plan.blocking_issues.is_empty() {
                "préflight refusé sans détail".to_string()
            } else {
                plan.blocking_issues.join("; ")
            };
            return Ok(Some(Self::empty_agent_loop_result(format!(
                "Je ne peux pas appliquer ce switch pour l'instant : {issues}"
            ))));
        }

        if !plan.provider_changed && !plan.model_changed {
            return Ok(Some(Self::empty_agent_loop_result(format!(
                "C'est déjà le modèle actif : **{} / {}**.",
                plan.target_provider, plan.target_model
            ))));
        }

        if let Some(choice) = parse_pending_model_switch_choice(user_message) {
            if choice == PendingModelSwitchChoice::Cancel {
                return Ok(Some(Self::empty_agent_loop_result(
                    "Switch annulé. Je garde le modèle actuel.".to_string(),
                )));
            }
            let strategy = choice
                .as_session_strategy()
                .and_then(|s| s.parse::<ModelSwitchSessionStrategy>().ok())
                .ok_or_else(|| {
                    KernelError::Captain(CaptainError::Internal(
                        "Invalid direct model switch choice".to_string(),
                    ))
                })?;
            let applied = self.apply_model_switch(
                agent_id,
                &plan.target_model,
                Some(&plan.target_provider),
                strategy,
            )?;
            let session_label = match strategy {
                ModelSwitchSessionStrategy::NewSession => "nouvelle session démarrée",
                ModelSwitchSessionStrategy::CompactSession => {
                    "contexte compacté puis nouvelle session démarrée"
                }
            };
            return Ok(Some(Self::empty_agent_loop_result(format!(
                "✅ Switché sur **{} / {}** — {}.",
                applied.plan.target_provider, applied.plan.target_model, session_label
            ))));
        }

        self.store_pending_model_switch_plan(agent_id, &plan)?;
        Ok(Some(Self::empty_agent_loop_result(format!(
            "✅ Switch sécurisé prêt vers **{} / {}**.\nRéponds simplement `Nouvelle session` ou `Résumé compact`.",
            plan.target_provider, plan.target_model
        ))))
    }

    fn store_pending_model_switch_plan(
        &self,
        agent_id: AgentId,
        plan: &ModelSwitchPlan,
    ) -> KernelResult<()> {
        let key = pending_model_switch_key(&agent_id.to_string());
        self.memory
            .structured_set(
                shared_memory_agent_id(),
                &key,
                serde_json::json!({
                    "status": "pending",
                    "agent_id": agent_id.to_string(),
                    "provider": plan.target_provider.as_str(),
                    "model": plan.target_model.as_str(),
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "expires_at_unix": chrono::Utc::now().timestamp() + 900,
                    "source": "kernel_direct_model_switch"
                }),
            )
            .map_err(KernelError::Captain)
    }

    pub(super) fn detect_direct_model_switch_target(
        &self,
        user_message: &str,
    ) -> Option<(String, String)> {
        let normalized = normalize_model_switch_text(user_message);
        let padded = format!(" {normalized} ");
        let has_switch_intent = [
            " switch ",
            " bascule ",
            " basculer ",
            " changer ",
            " change ",
            " remets ",
            " remet ",
            " remettre ",
            " mets ",
            " mettre ",
            " repasse ",
            " modele par defaut ",
            " model par defaut ",
            " provider par defaut ",
        ]
        .iter()
        .any(|needle| padded.contains(needle));
        if !has_switch_intent {
            return None;
        }

        let provider_hint = if padded.contains(" anthropic ") {
            Some("anthropic")
        } else if padded.contains(" codex ") || padded.contains(" gpt 5 5 ") {
            Some("codex")
        } else if padded.contains(" openai ") {
            Some("openai")
        } else {
            None
        };

        if provider_hint == Some("anthropic")
            && padded.contains(" sonnet ")
            && (padded.contains(" 4 6 ") || padded.contains(" 46 "))
        {
            return Some(("anthropic".to_string(), "claude-sonnet-4-6".to_string()));
        }
        if provider_hint == Some("codex") && padded.contains(" 5 5 ") {
            return Some(("codex".to_string(), "gpt-5.5".to_string()));
        }
        if provider_hint == Some("codex") && padded.contains(" 5 4 mini ") {
            return Some(("codex".to_string(), "gpt-5.4-mini".to_string()));
        }
        if provider_hint == Some("codex") && padded.contains(" 5 4 ") {
            return Some(("codex".to_string(), "gpt-5.4".to_string()));
        }
        if provider_hint == Some("codex") && padded.contains(" 5 3 ") && padded.contains(" spark ")
        {
            return Some(("codex".to_string(), "gpt-5.3-codex-spark".to_string()));
        }
        if provider_hint == Some("codex") && padded.contains(" 5 3 ") {
            return Some(("codex".to_string(), "gpt-5.3-codex".to_string()));
        }

        let catalog = self.model_catalog.read().ok()?;
        for entry in catalog.list_models() {
            if provider_hint.is_some_and(|provider| provider != entry.provider) {
                continue;
            }
            let entry_id = normalize_model_switch_text(&entry.id);
            let display_name = normalize_model_switch_text(&entry.display_name);
            if !entry_id.is_empty()
                && (padded.contains(&format!(" {entry_id} "))
                    || padded.contains(&format!(" {display_name} ")))
            {
                return Some((entry.provider.clone(), entry.id.clone()));
            }
        }

        if padded.contains(" sonnet ") {
            let entry = catalog.find_model("sonnet")?;
            return Some((entry.provider.clone(), entry.id.clone()));
        }
        None
    }
}

fn normalize_model_switch_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars().flat_map(char::to_lowercase) {
        let mapped = match ch {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c if c.is_ascii_alphanumeric() => c,
            _ => ' ',
        };
        out.push(mapped);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
