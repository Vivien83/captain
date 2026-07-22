//! Telegram Rich rendering for the shared Skill Learning V2 proposal card.

use captain_types::workflow_learning::{
    ProposalCard, ProposalCardAction, ProposalCardKind, ProposalCardRisk, ProposalInstallMode,
    ProposalIsolatedTestStatus, ProposalOperatorOutcome, ProposalOperatorResolution,
    ProposalRefinementCaptureResolution, WorkflowLifecycleCard, WorkflowLifecycleEvent,
};

pub fn format_workflow_learning_card(card: &ProposalCard, language: &str) -> String {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    let labels = Labels::new(french);
    let steps = render_steps(card, labels.none);
    let validation = render_validation(card, french);
    let authority = render_authority(card, labels.none);
    let limitations = render_limitations(card, labels.none);
    let isolated_test = render_isolated_test(card, french);
    let evidence = if french {
        format!(
            "{} executions · {} tours · {} sessions",
            card.evidence.occurrences,
            card.evidence.distinct_turns,
            card.evidence.distinct_sessions
        )
    } else {
        format!(
            "{} runs · {} turns · {} sessions",
            card.evidence.occurrences,
            card.evidence.distinct_turns,
            card.evidence.distinct_sessions
        )
    };

    format!(
        "🧩 **{title}**\n\n\
         **{name}** · {kind}\n\
         {purpose}\n\n\
         **{trigger_label}**\n{trigger}\n\n\
         **{evidence_label}**\n{evidence}\n\n\
         **{steps_label}**\n{steps}\n\n\
         **{validation_label}**\n{validation}\n\n\
         **{authority_label}**\n{authority}\n\n\
         **{risk_label}** {risk}\n\
         **{benefit_label}** {benefit}\n\n\
         **{limitations_label}**\n{limitations}\n\n\
         {isolated_test}\
         **{revision_label}** `{revision}`\n\
         **{recommendation_label}** {recommendation}",
        title = labels.title,
        name = markdown_safe(&card.name),
        kind = kind_label(card.kind, french),
        purpose = markdown_safe(&card.purpose),
        trigger_label = labels.trigger,
        trigger = markdown_safe(&card.trigger),
        evidence_label = labels.evidence,
        steps_label = labels.steps,
        validation_label = labels.validation,
        authority_label = labels.authority,
        risk_label = labels.risk,
        risk = risk_label(card.risk, french),
        benefit_label = labels.benefit,
        benefit = markdown_safe(&card.expected_benefit),
        limitations_label = labels.limitations,
        isolated_test = isolated_test,
        revision_label = labels.revision,
        revision = card.revision_sha256,
        recommendation_label = labels.recommendation,
        recommendation = action_label(card.recommended_action, french),
    )
}

pub fn build_workflow_learning_keyboard(card: &ProposalCard, language: &str) -> serde_json::Value {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    let has = |action| card.available_actions.contains(&action);
    let mut rows = Vec::<Vec<serde_json::Value>>::new();

    if has(card.recommended_action) {
        rows.push(vec![button(
            card.recommended_action,
            &card.lookup_token,
            card.decision_version,
            french,
            true,
            false,
        )]);
    }
    let secondary_primary = [ProposalCardAction::Activate, ProposalCardAction::Test]
        .into_iter()
        .filter(|action| *action != card.recommended_action && has(*action))
        .map(|action| {
            button(
                action,
                &card.lookup_token,
                card.decision_version,
                french,
                false,
                action == ProposalCardAction::Test
                    && card.isolated_test.as_ref().is_some_and(|test| {
                        matches!(
                            test.status,
                            ProposalIsolatedTestStatus::Passed | ProposalIsolatedTestStatus::Failed
                        )
                    }),
            )
        })
        .collect::<Vec<_>>();
    if !secondary_primary.is_empty() {
        rows.push(secondary_primary);
    }
    let details = [ProposalCardAction::Details, ProposalCardAction::Edit]
        .into_iter()
        .filter(|action| has(*action))
        .map(|action| {
            button(
                action,
                &card.lookup_token,
                card.decision_version,
                french,
                false,
                false,
            )
        })
        .collect::<Vec<_>>();
    if !details.is_empty() {
        rows.push(details);
    }
    let defer = [ProposalCardAction::Later, ProposalCardAction::Ignore]
        .into_iter()
        .filter(|action| has(*action))
        .map(|action| {
            button(
                action,
                &card.lookup_token,
                card.decision_version,
                french,
                false,
                false,
            )
        })
        .collect::<Vec<_>>();
    if !defer.is_empty() {
        rows.push(defer);
    }
    serde_json::json!({"inline_keyboard": rows})
}

pub fn format_workflow_isolated_test_result(
    card: &ProposalCard,
    passed: bool,
    language: &str,
) -> String {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    let checks = render_isolated_test_checks(card, french);
    if passed && french {
        format!(
            "🧪 **Test isole reussi**\n\n\
             **{}** est valide dans les registres prives de Captain.\n\
             Aucun skill, CapSpec ou scheduler actif n'a ete modifie.\n\n\
             **Controles**\n{}\n\n\
             Revision exacte : `{}`\n\
             **Prochaine action recommandee :** {}",
            markdown_safe(&card.name),
            checks,
            card.revision_sha256,
            action_label(card.recommended_action, true),
        )
    } else if passed {
        format!(
            "🧪 **Isolated test passed**\n\n\
             **{}** passed Captain's private registries.\n\
             No active skill, CapSpec, or scheduler was modified.\n\n\
             **Checks**\n{}\n\n\
             Exact revision: `{}`\n\
             **Recommended next action:** {}",
            markdown_safe(&card.name),
            checks,
            card.revision_sha256,
            action_label(card.recommended_action, false),
        )
    } else if french {
        format!(
            "🧪 **Test isole echoue**\n\n\
             **{}** n'a pas franchi tous les controles natifs.\n\
             Aucun etat actif n'a ete modifie. Corrige la proposition ou relance le test.\n\n\
             **Controles**\n{}\n\n\
             Revision exacte : `{}`",
            markdown_safe(&card.name),
            checks,
            card.revision_sha256,
        )
    } else {
        format!(
            "🧪 **Isolated test failed**\n\n\
             **{}** did not pass every native check.\n\
             No active state was modified. Edit the proposal or run the test again.\n\n\
             **Checks**\n{}\n\n\
             Exact revision: `{}`",
            markdown_safe(&card.name),
            checks,
            card.revision_sha256,
        )
    }
}

pub fn format_workflow_lifecycle_card(lifecycle: &WorkflowLifecycleCard, language: &str) -> String {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    let kind = kind_label(lifecycle.kind, french);
    let target = lifecycle
        .target_locator
        .as_deref()
        .map(markdown_safe)
        .unwrap_or_else(|| {
            if french {
                "aucun effet externe"
            } else {
                "no external effect"
            }
            .into()
        });
    let occurred_at = render_utc_timestamp(lifecycle.occurred_at_unix_ms);
    let revision = &lifecycle.revision_sha256;
    let name = markdown_safe(&lifecycle.name);
    match (lifecycle.event, french) {
        (WorkflowLifecycleEvent::InstallationVerified, true) => format!(
            "🔎 **Installation verifiee**\n\n\
             **{name}** · {kind}\n\
             La revision exacte est installee et verifiee par le registre natif.\n\
             Le canary durable est maintenant en file d'attente.\n\n\
             **Cible** `{target}`\n\
             **Revision** `{revision}`\n\
             **Etat** `active_canary` · `{occurred_at}` UTC"
        ),
        (WorkflowLifecycleEvent::InstallationVerified, false) => format!(
            "🔎 **Installation verified**\n\n\
             **{name}** · {kind}\n\
             The exact revision is installed and verified by the native registry.\n\
             Its durable canary is now queued.\n\n\
             **Target** `{target}`\n\
             **Revision** `{revision}`\n\
             **State** `active_canary` · `{occurred_at}` UTC"
        ),
        (WorkflowLifecycleEvent::ActivationCompleted, true) => format!(
            "✅ **Fonction native active**\n\n\
             **{name}** · {kind}\n\
             Installation, registre et canary concordent avec la revision approuvee.\n\n\
             **Cible** `{target}`\n\
             **Revision** `{revision}`\n\
             **Etat** `active` · `{occurred_at}` UTC"
        ),
        (WorkflowLifecycleEvent::ActivationCompleted, false) => format!(
            "✅ **Native capability active**\n\n\
             **{name}** · {kind}\n\
             Installation, registry, and canary match the approved revision.\n\n\
             **Target** `{target}`\n\
             **Revision** `{revision}`\n\
             **State** `active` · `{occurred_at}` UTC"
        ),
        (WorkflowLifecycleEvent::ActivationFailed, true) => {
            let error = markdown_safe(
                lifecycle
                    .failure_message
                    .as_deref()
                    .unwrap_or("echec d'activation sans detail"),
            );
            let recovery = if lifecycle.rollback_job_id.is_some() {
                "Un rollback exact est planifie et sera verifie automatiquement."
            } else {
                "Aucun effet externe n'avait ete engage ; aucun rollback n'est necessaire."
            };
            format!(
                "⚠️ **Activation interrompue**\n\n\
                 **{name}** · {kind}\n\
                 {error}\n\n\
                 {recovery}\n\n\
                 **Cible** `{target}`\n\
                 **Revision** `{revision}`\n\
                 **Etat** `install_failed` · `{occurred_at}` UTC"
            )
        }
        (WorkflowLifecycleEvent::ActivationFailed, false) => {
            let error = markdown_safe(
                lifecycle
                    .failure_message
                    .as_deref()
                    .unwrap_or("activation failed without details"),
            );
            let recovery = if lifecycle.rollback_job_id.is_some() {
                "An exact rollback is scheduled and will be verified automatically."
            } else {
                "No external effect had started; no rollback is required."
            };
            format!(
                "⚠️ **Activation interrupted**\n\n\
                 **{name}** · {kind}\n\
                 {error}\n\n\
                 {recovery}\n\n\
                 **Target** `{target}`\n\
                 **Revision** `{revision}`\n\
                 **State** `install_failed` · `{occurred_at}` UTC"
            )
        }
        (WorkflowLifecycleEvent::RollbackCompleted, true) => format!(
            "↩️ **Rollback verifie**\n\n\
             **{name}** · {kind}\n\
             L'effet exact a ete retire ou la revision precedente restauree.\n\
             Le registre natif confirme l'etat final.\n\n\
             **Cible** `{target}`\n\
             **Revision retiree** `{revision}`\n\
             **Etat** `rolled_back` · `{occurred_at}` UTC"
        ),
        (WorkflowLifecycleEvent::RollbackCompleted, false) => format!(
            "↩️ **Rollback verified**\n\n\
             **{name}** · {kind}\n\
             The exact effect was removed or the previous revision restored.\n\
             The native registry confirms the final state.\n\n\
             **Target** `{target}`\n\
             **Removed revision** `{revision}`\n\
             **State** `rolled_back` · `{occurred_at}` UTC"
        ),
        (WorkflowLifecycleEvent::RollbackFailed, true) => format!(
            "🚨 **Rollback en echec**\n\n\
             **{name}** · {kind}\n\
             Captain n'a pas pu prouver le retour a un etat sur. Une intervention est requise.\n\n\
             **Cible** `{target}`\n\
             **Revision** `{revision}`\n\
             **Erreur** {}\n\
             `{occurred_at}` UTC",
            markdown_safe(
                lifecycle
                    .failure_message
                    .as_deref()
                    .unwrap_or("erreur inconnue")
            )
        ),
        (WorkflowLifecycleEvent::RollbackFailed, false) => format!(
            "🚨 **Rollback failed**\n\n\
             **{name}** · {kind}\n\
             Captain could not prove a safe final state. Operator action is required.\n\n\
             **Target** `{target}`\n\
             **Revision** `{revision}`\n\
             **Error** {}\n\
             `{occurred_at}` UTC",
            markdown_safe(
                lifecycle
                    .failure_message
                    .as_deref()
                    .unwrap_or("unknown error")
            )
        ),
    }
}

pub fn format_workflow_learning_resolution(
    resolution: &ProposalOperatorResolution,
    language: &str,
) -> String {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    let replay = match (resolution.replayed, french) {
        (true, true) => "\n\n_Reponse rejouee sans dupliquer la decision._",
        (true, false) => "\n\n_Response replayed without duplicating the decision._",
        (false, _) => "",
    };
    let card = &resolution.card;
    match &resolution.outcome {
        ProposalOperatorOutcome::Details => format_workflow_learning_card(card, language),
        ProposalOperatorOutcome::EditRequested {
            request_id,
            expires_at_unix_ms,
        } => {
            let expires_at = render_utc_timestamp(*expires_at_unix_ms);
            let request = request_id.chars().take(18).collect::<String>();
            if french {
                format!(
                    "✏️ **Modification de proposition**\n\n\
                     **{}**\n\
                     Envoie maintenant les changements souhaites dans un nouveau message.\n\
                     Fenetre de reponse : jusqu'a `{}` UTC.\n\
                     Demande : `{}`\n\n\
                     La revision `{}` reste intacte tant qu'une nouvelle proposition n'a pas \
                     ete validee.{}",
                    markdown_safe(&card.name),
                    expires_at,
                    request,
                    card.revision_sha256,
                    replay,
                )
            } else {
                format!(
                    "✏️ **Edit proposal**\n\n\
                     **{}**\n\
                     Send the requested changes now in a new message.\n\
                     Reply window: until `{}` UTC.\n\
                     Request: `{}`\n\n\
                     Revision `{}` remains intact until a new proposal has been validated.{}",
                    markdown_safe(&card.name),
                    expires_at,
                    request,
                    card.revision_sha256,
                    replay,
                )
            }
        }
        ProposalOperatorOutcome::InstallQueued { mode } => {
            let mode = match (*mode, french) {
                (ProposalInstallMode::Activate, true) => "activation",
                (ProposalInstallMode::Test, true) => "test isole",
                (ProposalInstallMode::Activate, false) => "activation",
                (ProposalInstallMode::Test, false) => "isolated test",
            };
            if french {
                format!(
                    "✅ **Decision enregistree**\n\n\
                     **{}** · {} planifie\n\
                     Revision exacte : `{}`\n\
                     Captain notifiera le resultat apres verification.{}",
                    markdown_safe(&card.name),
                    mode,
                    card.revision_sha256,
                    replay,
                )
            } else {
                format!(
                    "✅ **Decision recorded**\n\n\
                     **{}** · {} queued\n\
                     Exact revision: `{}`\n\
                     Captain will report the result after verification.{}",
                    markdown_safe(&card.name),
                    mode,
                    card.revision_sha256,
                    replay,
                )
            }
        }
        ProposalOperatorOutcome::Snoozed { until_unix_ms } => {
            let until = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(*until_unix_ms)
                .map(|value| value.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| until_unix_ms.to_string());
            if french {
                format!(
                    "⏰ **Proposition reportee**\n\n\
                     **{}** reviendra apres `{}`.{}",
                    markdown_safe(&card.name),
                    until,
                    replay,
                )
            } else {
                format!(
                    "⏰ **Proposal snoozed**\n\n\
                     **{}** will return after `{}`.{}",
                    markdown_safe(&card.name),
                    until,
                    replay,
                )
            }
        }
        ProposalOperatorOutcome::Dismissed => {
            if french {
                format!(
                    "🚫 **Proposition ignoree**\n\n\
                     **{}** · revision `{}`\n\
                     Aucun changement n'a ete installe.{}",
                    markdown_safe(&card.name),
                    card.revision_sha256,
                    replay,
                )
            } else {
                format!(
                    "🚫 **Proposal ignored**\n\n\
                     **{}** · revision `{}`\n\
                     No change was installed.{}",
                    markdown_safe(&card.name),
                    card.revision_sha256,
                    replay,
                )
            }
        }
    }
}

pub fn format_workflow_refinement_capture(
    resolution: &ProposalRefinementCaptureResolution,
) -> String {
    let french = resolution
        .language
        .trim()
        .to_ascii_lowercase()
        .starts_with("fr");
    let replay = match (resolution.replayed, french) {
        (true, true) => "\n\n_Message deja recu : aucun second job n'a ete cree._",
        (true, false) => "\n\n_Message already received: no second job was created._",
        (false, _) => "",
    };
    let request = resolution.request_id.chars().take(18).collect::<String>();
    if french {
        format!(
            "🛠️ **Modification prise en compte**\n\n\
             La nouvelle revision est en cours de generation et de validation.\n\
             Demande : `{}`\n\
             Proposition parente : `{}`\n\
             Proposition candidate : `{}`\n\n\
             La version actuelle reste active et inchangee jusqu'a validation complete.{}",
            code_safe(&request),
            code_safe(&resolution.parent_proposal_id),
            code_safe(&resolution.child_proposal_id),
            replay,
        )
    } else {
        format!(
            "🛠️ **Edit accepted**\n\n\
             The new revision is being generated and validated.\n\
             Request: `{}`\n\
             Parent proposal: `{}`\n\
             Candidate proposal: `{}`\n\n\
             The current version remains active and unchanged until validation completes.{}",
            code_safe(&request),
            code_safe(&resolution.parent_proposal_id),
            code_safe(&resolution.child_proposal_id),
            replay,
        )
    }
}

pub fn format_workflow_refinement_capture_error(reason: &str, language: &str) -> String {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    if french {
        format!(
            "⚠️ **Modification non enregistree**\n\n{}\n\n\
             _La proposition actuelle reste intacte. Envoie une instruction corrigee tant que \
             la demande est active._",
            markdown_safe(reason)
        )
    } else {
        format!(
            "⚠️ **Edit not recorded**\n\n{}\n\n\
             _The current proposal remains unchanged. Send a corrected instruction while the \
             request is active._",
            markdown_safe(reason)
        )
    }
}

fn render_utc_timestamp(unix_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(unix_ms)
        .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| unix_ms.to_string())
}

pub fn format_workflow_learning_error(reason: &str, language: &str) -> String {
    let french = language.trim().to_ascii_lowercase().starts_with("fr");
    if french {
        format!(
            "⚠️ **Decision non appliquee**\n\n{}\n\n_La proposition et ses boutons restent inchanges._",
            markdown_safe(reason)
        )
    } else {
        format!(
            "⚠️ **Decision not applied**\n\n{}\n\n_The proposal and its buttons remain unchanged._",
            markdown_safe(reason)
        )
    }
}

fn button(
    action: ProposalCardAction,
    token: &str,
    decision_version: u64,
    french: bool,
    recommended: bool,
    retest: bool,
) -> serde_json::Value {
    let prefix = if recommended { "★ " } else { "" };
    let label = if retest {
        if french {
            "Retester"
        } else {
            "Test again"
        }
    } else {
        action_label(action, french)
    };
    serde_json::json!({
        "text": format!("{prefix}{label}"),
        "callback_data": format!("workflow:{}:{token}:{decision_version}", action.as_str()),
    })
}

fn render_isolated_test(card: &ProposalCard, french: bool) -> String {
    let Some(test) = &card.isolated_test else {
        return String::new();
    };
    let status = match (test.status, french) {
        (ProposalIsolatedTestStatus::Queued, true) => "⏳ En attente",
        (ProposalIsolatedTestStatus::Passed, true) => "✅ Reussi",
        (ProposalIsolatedTestStatus::Failed, true) => "❌ Echoue",
        (ProposalIsolatedTestStatus::Queued, false) => "⏳ Queued",
        (ProposalIsolatedTestStatus::Passed, false) => "✅ Passed",
        (ProposalIsolatedTestStatus::Failed, false) => "❌ Failed",
    };
    let title = if french {
        "Test isole"
    } else {
        "Isolated test"
    };
    let checks = render_isolated_test_checks(card, french);
    if checks.is_empty() {
        format!("**{title}** {status}\n\n")
    } else {
        format!("**{title}** {status}\n{checks}\n\n")
    }
}

fn render_isolated_test_checks(card: &ProposalCard, french: bool) -> String {
    let Some(test) = &card.isolated_test else {
        return String::new();
    };
    if test.checks.is_empty() {
        return if french {
            "Aucun resultat disponible.".to_string()
        } else {
            "No result available.".to_string()
        };
    }
    test.checks
        .iter()
        .take(8)
        .map(|check| {
            format!(
                "{} `{}` · {}",
                if check.passed { "✓" } else { "✗" },
                code_safe(&check.code),
                markdown_safe(&check.detail)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_steps(card: &ProposalCard, none: &str) -> String {
    if card.steps.is_empty() {
        return none.to_string();
    }
    let mut lines = card
        .steps
        .iter()
        .take(8)
        .enumerate()
        .map(|(index, step)| {
            format!(
                "{}. `{}` · {}",
                index + 1,
                code_safe(&step.tool_name),
                markdown_safe(&step.role)
            )
        })
        .collect::<Vec<_>>();
    if card.steps.len() > 8 {
        lines.push(format!("+{}", card.steps.len() - 8));
    }
    lines.join("\n")
}

fn render_validation(card: &ProposalCard, french: bool) -> String {
    card.validation
        .iter()
        .map(|fact| {
            let marker = if fact.passed { "✓" } else { "!" };
            format!("{marker} {}", validation_label(&fact.code, french))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_authority(card: &ProposalCard, none: &str) -> String {
    if card.required_authority.is_empty() {
        return none.to_string();
    }
    card.required_authority
        .iter()
        .take(12)
        .map(|authority| format!("`{}`", code_safe(authority)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn render_limitations(card: &ProposalCard, none: &str) -> String {
    if card.validation_limitations.is_empty() {
        return none.to_string();
    }
    card.validation_limitations
        .iter()
        .take(4)
        .map(|limitation| format!("- {}", markdown_safe(limitation)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn validation_label(code: &str, french: bool) -> String {
    let label = match (code, french) {
        ("whole_response_schema", true) => "Schema structure valide",
        ("native_artifact_parser", true) => "Artefact relu par le parseur natif",
        ("secret_scan", true) => "Aucun secret detecte",
        ("path_and_identifier_policy", true) => "Chemins et identifiants conformes",
        ("immutable_staging_hashes", true) => "Revision stagee et hashes verifies",
        ("whole_response_schema", false) => "Structured schema valid",
        ("native_artifact_parser", false) => "Native artifact parser passed",
        ("secret_scan", false) => "No secret detected",
        ("path_and_identifier_policy", false) => "Paths and identifiers compliant",
        ("immutable_staging_hashes", false) => "Staged revision and hashes verified",
        _ => return markdown_safe(code),
    };
    label.to_string()
}

fn kind_label(kind: ProposalCardKind, french: bool) -> &'static str {
    match (kind, french) {
        (ProposalCardKind::Skill, _) => "Skill",
        (ProposalCardKind::Capspec, _) => "CapSpec",
        (ProposalCardKind::Automation, true) => "Automatisation",
        (ProposalCardKind::Automation, false) => "Automation",
        (ProposalCardKind::Refinement, true) => "Amelioration",
        (ProposalCardKind::Refinement, false) => "Refinement",
    }
}

fn risk_label(risk: ProposalCardRisk, french: bool) -> &'static str {
    match (risk, french) {
        (ProposalCardRisk::ReadOnly, true) => "lecture seule",
        (ProposalCardRisk::Mutation, true) => "mutation controlee",
        (ProposalCardRisk::Unknown, true) => "autorite a confirmer",
        (ProposalCardRisk::ReadOnly, false) => "read-only",
        (ProposalCardRisk::Mutation, false) => "controlled mutation",
        (ProposalCardRisk::Unknown, false) => "authority needs review",
    }
}

fn action_label(action: ProposalCardAction, french: bool) -> &'static str {
    match (action, french) {
        (ProposalCardAction::Activate, true) => "Activer",
        (ProposalCardAction::Test, true) => "Tester d'abord",
        (ProposalCardAction::Details, true) => "Details",
        (ProposalCardAction::Edit, true) => "Modifier",
        (ProposalCardAction::Later, true) => "Plus tard",
        (ProposalCardAction::Ignore, true) => "Ignorer",
        (ProposalCardAction::Activate, false) => "Activate",
        (ProposalCardAction::Test, false) => "Test first",
        (ProposalCardAction::Details, false) => "Details",
        (ProposalCardAction::Edit, false) => "Edit",
        (ProposalCardAction::Later, false) => "Later",
        (ProposalCardAction::Ignore, false) => "Ignore",
    }
}

fn markdown_safe(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn code_safe(value: &str) -> String {
    value.replace('`', "'").replace(['\r', '\n'], " ")
}

struct Labels {
    title: &'static str,
    trigger: &'static str,
    evidence: &'static str,
    steps: &'static str,
    validation: &'static str,
    authority: &'static str,
    risk: &'static str,
    benefit: &'static str,
    limitations: &'static str,
    revision: &'static str,
    recommendation: &'static str,
    none: &'static str,
}

impl Labels {
    fn new(french: bool) -> Self {
        if french {
            Self {
                title: "Nouvelle capacite proposee",
                trigger: "Quand Captain l'utilisera",
                evidence: "Preuves observees",
                steps: "Workflow compact",
                validation: "Validations",
                authority: "Autorites requises",
                risk: "Risque :",
                benefit: "Benefice :",
                limitations: "Limites connues",
                revision: "Revision",
                recommendation: "Action recommandee :",
                none: "Aucun",
            }
        } else {
            Self {
                title: "New capability proposal",
                trigger: "When Captain will use it",
                evidence: "Observed evidence",
                steps: "Compact workflow",
                validation: "Validation",
                authority: "Required authority",
                risk: "Risk:",
                benefit: "Benefit:",
                limitations: "Known limitations",
                revision: "Revision",
                recommendation: "Recommended action:",
                none: "None",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use captain_types::workflow_learning::{
        ProposalCardEvidence, ProposalCardModel, ProposalCardState, ProposalCardStep,
        ProposalCardValidationFact, ProposalIsolatedTest, ProposalIsolatedTestCheck,
        PROPOSAL_CARD_SCHEMA_VERSION, WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
    };

    use super::*;
    use crate::telegram_callbacks::parse_workflow_learning_callback;

    fn card(risk: ProposalCardRisk) -> ProposalCard {
        let recommended_action = if risk == ProposalCardRisk::ReadOnly {
            ProposalCardAction::Activate
        } else {
            ProposalCardAction::Test
        };
        ProposalCard {
            schema_version: PROPOSAL_CARD_SCHEMA_VERSION,
            proposal_id: "proposal-1".to_string(),
            lookup_token: "00000000000000000000".to_string(),
            decision_version: 4,
            revision_sha256: "a".repeat(64),
            state: ProposalCardState::Proposed,
            kind: ProposalCardKind::Skill,
            name: "sourced-research".to_string(),
            purpose: "Search *current* sources safely.".to_string(),
            trigger: "A current answer needs sources.".to_string(),
            evidence: ProposalCardEvidence {
                occurrences: 3,
                distinct_turns: 3,
                distinct_sessions: 2,
                explicit_reuse_request: false,
            },
            steps: vec![ProposalCardStep {
                index: 0,
                tool_name: "web_search".to_string(),
                role: "research".to_string(),
                dependencies: vec![],
            }],
            validation: vec![ProposalCardValidationFact {
                code: "secret_scan".to_string(),
                passed: true,
            }],
            validation_limitations: vec!["Review high-stakes claims.".to_string()],
            isolated_test: None,
            validated_by: ProposalCardModel {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            required_authority: vec!["web_search".to_string()],
            expected_benefit: "Repeatable sourced answers.".to_string(),
            risk,
            recommended_action,
            available_actions: vec![
                recommended_action,
                ProposalCardAction::Details,
                ProposalCardAction::Edit,
                ProposalCardAction::Later,
                ProposalCardAction::Ignore,
            ],
        }
    }

    #[test]
    fn french_rich_card_is_compact_safe_and_complete() {
        let rendered = format_workflow_learning_card(&card(ProposalCardRisk::ReadOnly), "fr-FR");
        assert!(rendered.contains("Nouvelle capacite proposee"));
        assert!(rendered.contains("Search \\*current\\* sources safely."));
        assert!(rendered.contains("3 executions · 3 tours · 2 sessions"));
        assert!(rendered.contains("✓ Aucun secret detecte"));
        assert!(rendered.contains("Action recommandee :** Activer"));
        assert!(rendered.contains(&format!("`{}`", "a".repeat(64))));
    }

    #[test]
    fn keyboard_marks_one_recommendation_and_callbacks_are_strict() {
        let keyboard = build_workflow_learning_keyboard(&card(ProposalCardRisk::Mutation), "fr");
        let rows = keyboard["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].as_array().unwrap().len(), 1);
        assert_eq!(rows[0][0]["text"], "★ Tester d'abord");
        assert_eq!(rows[1].as_array().unwrap().len(), 2);
        assert_eq!(rows[2].as_array().unwrap().len(), 2);

        let callback =
            parse_workflow_learning_callback(rows[0][0]["callback_data"].as_str().unwrap())
                .unwrap();
        assert_eq!(callback.action, ProposalCardAction::Test);
        assert_eq!(callback.token, "00000000000000000000");
        assert_eq!(callback.decision_version, 4);
        assert!(parse_workflow_learning_callback("workflow:test:short").is_none());
        assert!(
            parse_workflow_learning_callback("workflow:unknown:00000000000000000000:4").is_none()
        );
        assert!(parse_workflow_learning_callback("workflow:test:00000000000000000000:0").is_none());
        assert!(
            parse_workflow_learning_callback("workflow:test:00000000000000000000:4:extra")
                .is_none()
        );
    }

    #[test]
    fn english_card_and_terminal_state_keyboard_remain_coherent() {
        let mut terminal = card(ProposalCardRisk::Unknown);
        terminal.state = ProposalCardState::Dismissed;
        terminal.available_actions = vec![ProposalCardAction::Details];
        let rendered = format_workflow_learning_card(&terminal, "en");
        let keyboard = build_workflow_learning_keyboard(&terminal, "en");

        assert!(rendered.contains("New capability proposal"));
        assert!(rendered.contains("authority needs review"));
        let rows = keyboard["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0]["text"], "Details");
    }

    #[test]
    fn terminal_resolution_reports_exact_revision_and_replay() {
        let resolution = ProposalOperatorResolution {
            card: card(ProposalCardRisk::Mutation),
            outcome: ProposalOperatorOutcome::InstallQueued {
                mode: ProposalInstallMode::Test,
            },
            replayed: true,
            retire_keyboard: true,
        };

        let rendered = format_workflow_learning_resolution(&resolution, "fr");
        assert!(rendered.contains("Decision enregistree"));
        assert!(rendered.contains("test isole planifie"));
        assert!(rendered.contains(&format!("`{}`", "a".repeat(64))));
        assert!(rendered.contains("sans dupliquer la decision"));
    }

    #[test]
    fn passed_isolated_test_renders_a_rich_result_and_retest_control() {
        let mut tested = card(ProposalCardRisk::Mutation);
        tested.recommended_action = ProposalCardAction::Activate;
        tested.available_actions = vec![
            ProposalCardAction::Activate,
            ProposalCardAction::Test,
            ProposalCardAction::Details,
            ProposalCardAction::Edit,
        ];
        tested.isolated_test = Some(ProposalIsolatedTest {
            status: ProposalIsolatedTestStatus::Passed,
            revision_sha256: tested.revision_sha256.clone(),
            job_id: "test-job-1".to_string(),
            checks: vec![ProposalIsolatedTestCheck {
                code: "native_skill_registry".to_string(),
                passed: true,
                detail: "exact private artifact loaded".to_string(),
            }],
            completed_at_unix_ms: Some(3_000),
        });

        let rendered = format_workflow_isolated_test_result(&tested, true, "fr");
        assert!(rendered.contains("Test isole reussi"));
        assert!(rendered.contains("Aucun skill, CapSpec ou scheduler actif"));
        assert!(rendered.contains("native_skill_registry"));
        let keyboard = build_workflow_learning_keyboard(&tested, "fr");
        assert_eq!(keyboard["inline_keyboard"][0][0]["text"], "★ Activer");
        assert_eq!(keyboard["inline_keyboard"][1][0]["text"], "Retester");
    }

    #[test]
    fn details_and_edit_resolutions_keep_the_current_revision_explicit() {
        let proposal = card(ProposalCardRisk::ReadOnly);
        let details = ProposalOperatorResolution {
            card: proposal.clone(),
            outcome: ProposalOperatorOutcome::Details,
            replayed: false,
            retire_keyboard: false,
        };
        let edit = ProposalOperatorResolution {
            card: proposal,
            outcome: ProposalOperatorOutcome::EditRequested {
                request_id: "wr-0123456789abcdef".to_string(),
                expires_at_unix_ms: 1_750_000_000_000,
            },
            replayed: false,
            retire_keyboard: false,
        };

        assert_eq!(
            format_workflow_learning_resolution(&details, "en"),
            format_workflow_learning_card(&details.card, "en")
        );
        let rendered = format_workflow_learning_resolution(&edit, "en");
        assert!(rendered.contains("Edit proposal"));
        assert!(rendered.contains(&format!("`{}`", "a".repeat(64))));
        assert!(rendered.contains("remains intact"));
        assert!(rendered.contains("wr-0123456789abcd"));
    }

    #[test]
    fn refinement_capture_reports_the_child_without_claiming_installation() {
        let rendered = format_workflow_refinement_capture(&ProposalRefinementCaptureResolution {
            request_id: "wr-1".to_string(),
            parent_proposal_id: "parent-1".to_string(),
            child_proposal_id: "child-1".to_string(),
            language: "fr".to_string(),
            replayed: true,
        });

        assert!(rendered.contains("Modification prise en compte"));
        assert!(rendered.contains("`parent-1`"));
        assert!(rendered.contains("`child-1`"));
        assert!(rendered.contains("aucun second job"));
        assert!(!rendered.contains("installee"));
    }

    #[test]
    fn callback_error_is_escaped_and_preserves_the_card_contract() {
        let rendered = format_workflow_learning_error("stale *revision*", "en");
        assert!(rendered.contains("stale \\*revision\\*"));
        assert!(rendered.contains("buttons remain unchanged"));
    }

    #[test]
    fn activation_lifecycle_cards_are_explicit_safe_and_channel_neutral() {
        let base = WorkflowLifecycleCard {
            schema_version: WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
            event: WorkflowLifecycleEvent::ActivationCompleted,
            proposal_id: "proposal-1".to_string(),
            revision_sha256: "b".repeat(64),
            decision_version: 7,
            state: ProposalCardState::Active,
            kind: ProposalCardKind::Capspec,
            name: "deploy-*verified*".to_string(),
            lifecycle_job_id: "canary-1".to_string(),
            continuation_job_id: None,
            target_locator: Some("capabilities/deploy-verified.capspec".to_string()),
            failure_code: None,
            failure_message: None,
            rollback_job_id: None,
            occurred_at_unix_ms: 1_750_000_000_000,
        };
        let active = format_workflow_lifecycle_card(&base, "fr");
        assert!(active.contains("Fonction native active"));
        assert!(active.contains("deploy-\\*verified\\*"));
        assert!(active.contains("`active`"));

        let failed = WorkflowLifecycleCard {
            event: WorkflowLifecycleEvent::ActivationFailed,
            state: ProposalCardState::InstallFailed,
            lifecycle_job_id: "install-1".to_string(),
            failure_code: Some("workflow_activation_failed".to_string()),
            failure_message: Some("registry *mismatch*".to_string()),
            rollback_job_id: Some("rollback-1".to_string()),
            ..base.clone()
        };
        let failed_text = format_workflow_lifecycle_card(&failed, "en");
        assert!(failed_text.contains("Activation interrupted"));
        assert!(failed_text.contains("registry \\*mismatch\\*"));
        assert!(failed_text.contains("exact rollback is scheduled"));

        let rollback_failed = WorkflowLifecycleCard {
            event: WorkflowLifecycleEvent::RollbackFailed,
            failure_code: Some("workflow_rollback_failed".to_string()),
            failure_message: Some("target still present".to_string()),
            rollback_job_id: None,
            ..failed
        };
        let alert = format_workflow_lifecycle_card(&rollback_failed, "fr");
        assert!(alert.contains("Rollback en echec"));
        assert!(alert.contains("intervention est requise"));
    }
}
