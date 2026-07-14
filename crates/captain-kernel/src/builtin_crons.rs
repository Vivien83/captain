//! Built-in cron jobs wired at kernel boot (v3.11e).
//!
//! Each builtin here is *idempotent*: on boot, we look up the job by
//! name, add it if absent, and do nothing if it already exists. That
//! way the daemon can restart hundreds of times without spawning a
//! stack of duplicates, and a user who deletes a builtin gets it back
//! on the next boot (opt-out is via config flag, not deletion).

use crate::CaptainKernel;
use captain_types::agent::AgentId;
use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use chrono::Utc;
use std::sync::Arc;
use tracing::{info, warn};

/// Name used to deduplicate the weekly reflection cron across boots.
pub const WEEKLY_REFLECTION_NAME: &str = "weekly_reflection";

/// Name used to deduplicate the skill curator cron across boots.
pub const SKILL_CURATOR_NAME: &str = "skill_curator";

/// The message the cron sends to Captain when it fires.
///
/// The prompt asks for a structured report the scheduler dumps to the
/// user's last active channel (Telegram by default). Sections mirror
/// the v3.11e spec: done this week / blocked / next week.
const WEEKLY_REFLECTION_PROMPT: &str = "\
Genere un rapport hebdomadaire pour mes projets actifs. Pour chaque projet :\n\
1. Utilise `memory_recall` et `mcp_mempalace_mempalace_search` pour recuperer\n\
   les activites de la semaine ecoulee.\n\
2. Structure la reponse en trois sections : `## Cette semaine` (tasks\n\
   terminees, milestones franchis), `## Bloque` (tasks status=blocked ou\n\
   milestones missed), `## Semaine prochaine` (prochaines actions prioritaires).\n\
3. Reste concis : un projet = 3-5 lignes max par section.\n\
\n\
Si aucun projet n'est actif, reponds juste \"Aucun projet actif cette semaine.\"";

/// Daily background pass to keep the skill library healthy. Modeled after
/// Curator-style maintenance: rather than letting skills accumulate
/// silently, Captain reviews them periodically and proposes consolidations
/// or deprecations through the existing `skill_refinement_propose` rail.
///
/// The prompt is silent on success (no message sent) and only writes a
/// report to disk so the user is never spammed by an empty curator pass.
const SKILL_CURATOR_PROMPT: &str = "\
Tu es en pass de curation des skills (background, non interactif). \
Objectif : garder la skill library propre, sans rien casser silencieusement.\n\
\n\
1. Liste les skills installes (`skill_list` ou equivalent) et recupere leur \
metadata d'usage (last_used_at, success_count, failure_count, version courante).\n\
2. Identifie les candidats a curer :\n\
   - Skills jamais utilises depuis plus de 30 jours.\n\
   - Skills avec failure_rate > 50 % sur les 10 derniers usages.\n\
   - Doublons semantiques (deux skills qui repondent au meme intent).\n\
3. Pour chaque candidat, propose une action via `skill_refinement_propose` \
avec un `risk` adequat et un `proposed_version` decrivant la consolidation, \
la deprecation ou le merge. Ne modifie jamais un skill sans passer par le rail \
d'approbation.\n\
4. Ecris un rapport synthetique du run dans `~/.captain/data/curator-reports/<YYYY-MM-DD>.md` \
(via `file_write`) avec : nombre de skills inspectes, candidats trouves, propositions \
emises (ids), skips. Si aucun candidat, ecris quand meme une ligne \"<date> : aucun candidat\".\n\
5. Ne `channel_send` jamais ce rapport en automatique. L'utilisateur le \
consultera via `self_improvement_review` ou le file_read s'il en a besoin.\n\
\n\
Garde-fous : pas de spam, pas de modif silencieuse, tout passe par \
`skill_refinement_propose` + approbation utilisateur. Si tu n'es pas sur, \
log et abstiens-toi.";

/// Ensure every builtin cron is registered.
///
/// Called once from `start_background_agents`. Failures per-cron are
/// logged at `warn` and do not abort the boot sequence — a broken
/// builtin should never take the daemon down.
///
/// ## Opt-out
/// Builtin crons are idempotently re-created if missing, so a delete
/// won't stick across restarts. To durably disable a builtin, keep the
/// job and flip `enabled = false` via the API / `cron_update` tool —
/// that preserves the entry so `builtin_exists` finds it and skips the
/// re-create.
pub fn ensure_all(kernel: &Arc<CaptainKernel>) {
    if let Err(err) = ensure_weekly_reflection(kernel) {
        warn!("failed to ensure weekly_reflection builtin cron: {err}");
    }
    if let Err(err) = ensure_skill_curator(kernel) {
        warn!("failed to ensure skill_curator builtin cron: {err}");
    }
}

/// Register the weekly reflection cron if not already present.
///
/// Returns `Ok(())` when the job exists (created or preserved) and
/// `Err` only on unrecoverable failure (no Captain agent, cron
/// rejection).
pub fn ensure_weekly_reflection(kernel: &Arc<CaptainKernel>) -> Result<(), String> {
    let Some(captain_id) = find_captain_agent_id(kernel) else {
        // No captain registered yet — the boot sequence creates agents
        // asynchronously on first boot. We log a debug-level note and
        // let the next kernel start retry.
        tracing::debug!("weekly_reflection: captain agent not found yet, skipping this boot");
        return Ok(());
    };

    if builtin_exists(kernel, captain_id, WEEKLY_REFLECTION_NAME) {
        tracing::debug!("weekly_reflection cron already registered");
        return Ok(());
    }

    let job = CronJob {
        id: CronJobId(uuid::Uuid::new_v4()),
        agent_id: captain_id,
        name: WEEKLY_REFLECTION_NAME.to_string(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 20 * * 0".to_string(),
            tz: Some("Europe/Paris".to_string()),
        },
        action: CronAction::AgentTurn {
            message: WEEKLY_REFLECTION_PROMPT.to_string(),
            model_override: None,
            timeout_secs: Some(300),
        },
        delivery: CronDelivery::LastChannel,
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
    };

    kernel
        .cron_scheduler
        .add_job(job, false)
        .map_err(|e| e.to_string())?;
    if let Err(err) = kernel.cron_scheduler.persist() {
        warn!("weekly_reflection: persist failed: {err}");
    }
    info!(
        agent = %captain_id,
        "weekly_reflection cron registered (Sunday 20:00 Europe/Paris)"
    );
    Ok(())
}

/// Register the skill curator cron if not already present.
///
/// Daily pass at 03:00 Europe/Paris (low-traffic window). Idempotent on
/// boot; survives daemon restarts and only runs once per day per agent.
pub fn ensure_skill_curator(kernel: &Arc<CaptainKernel>) -> Result<(), String> {
    let Some(captain_id) = find_captain_agent_id(kernel) else {
        tracing::debug!("skill_curator: captain agent not found yet, skipping this boot");
        return Ok(());
    };

    if builtin_exists(kernel, captain_id, SKILL_CURATOR_NAME) {
        tracing::debug!("skill_curator cron already registered");
        return Ok(());
    }

    let job = CronJob {
        id: CronJobId(uuid::Uuid::new_v4()),
        agent_id: captain_id,
        name: SKILL_CURATOR_NAME.to_string(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 3 * * *".to_string(),
            tz: Some("Europe/Paris".to_string()),
        },
        action: CronAction::AgentTurn {
            message: SKILL_CURATOR_PROMPT.to_string(),
            model_override: None,
            timeout_secs: Some(600),
        },
        delivery: CronDelivery::None,
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
    };

    kernel
        .cron_scheduler
        .add_job(job, false)
        .map_err(|e| e.to_string())?;
    if let Err(err) = kernel.cron_scheduler.persist() {
        warn!("skill_curator: persist failed: {err}");
    }
    info!(
        agent = %captain_id,
        "skill_curator cron registered (daily 03:00 Europe/Paris, no-delivery)"
    );
    Ok(())
}

/// Scan the registry for an agent named "captain" — the owner of
/// project-wide builtin crons.
fn find_captain_agent_id(kernel: &Arc<CaptainKernel>) -> Option<AgentId> {
    kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name.eq_ignore_ascii_case("captain"))
        .map(|entry| entry.id)
}

/// Check whether a cron with the given name is already registered for
/// this agent.
fn builtin_exists(kernel: &Arc<CaptainKernel>, agent_id: AgentId, name: &str) -> bool {
    kernel
        .cron_scheduler
        .list_jobs(agent_id)
        .into_iter()
        .any(|j| j.name == name)
}
