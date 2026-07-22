use super::*;

#[test]
fn test_captain_prompt_teaches_session_recall_and_workspace() {
    // After the resume + workspace_add + session_recall commits, Captain
    // ships with three new powers it must know about and use without
    // being asked. The prompt is the only place it learns them, so we
    // guard the four key markers as a regression net.
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("session_recall"),
        "prompt must mention session_recall by name"
    );
    assert!(
        prompt.contains("on avait dit") || prompt.contains("l'autre fois"),
        "prompt must teach the user-facing trigger phrases for cross-session recall"
    );
    assert!(
        prompt.contains("workspace_add"),
        "prompt must mention workspace_add by name"
    );
    assert!(
        prompt.contains("~/.captain/") || prompt.contains(".captain/"),
        "prompt must explain Captain's free access to ~/.captain/"
    );
}

#[test]
fn test_captain_prompt_teaches_memory_forget() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("memory_forget"),
        "prompt must name memory_forget so Sonnet knows retraction is possible"
    );
    assert!(
        prompt.contains("oublie") || prompt.contains("tu te trompes"),
        "prompt must teach the user-facing trigger phrases for retraction"
    );
}

#[test]
fn test_captain_prompt_answers_in_latest_user_language() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("langue du dernier message utilisateur"),
        "prompt must make Captain follow the user's current language"
    );
    assert!(
        prompt.contains("préférence utilisateur configurée")
            && prompt.contains("sans préférence")
            && prompt.contains("anglais"),
        "prompt must use configured language preferences and a neutral fallback"
    );
    assert!(
        !prompt.contains("français avec tutoiement"),
        "prompt must not embed an operator-specific language or tone"
    );
}

#[test]
fn test_captain_prompt_does_not_treat_playbook_as_canonical_self_audit_source() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        !prompt.contains("Lis PLAYBOOK.md au démarrage"),
        "prompt must not force file_read of workspace-local PLAYBOOK.md"
    );
    assert!(
        prompt.contains("Ne traite pas PLAYBOOK.md comme une source canonique"),
        "prompt should keep self-audits/comparisons on live state and native docs"
    );
    assert!(
        prompt.contains("captain_docs") && prompt.contains("capability_search"),
        "prompt should route tool examples through native docs/capability discovery"
    );
}

/// A.3 — once Captain has the `channel_reconfigure` tool (A.2) and the
/// per-channel reload primitive (A.1), it still won't reach for them
/// unless the prompt teaches the workflow: "edit config.toml, then
/// channel_reconfigure({channel})". Without these markers Sonnet
/// defaults to rebuilding the daemon, which disconnects every other
/// channel for several seconds.
#[test]
fn test_captain_prompt_teaches_channel_reconfigure() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("channel_reconfigure"),
        "prompt must name channel_reconfigure so Sonnet swaps adapters"
    );
    assert!(
        prompt.contains("config.toml"),
        "prompt must explain that Captain edits config.toml first"
    );
    assert!(
        prompt.contains("nouveau token")
            || prompt.contains("change le bot")
            || prompt.contains("rotation"),
        "prompt must teach when to reach for channel_reconfigure"
    );
}

/// TS.4/CR.1 — After TS.1/TS.2 collapsed the visible builtin surface
/// to CORE tools, Captain MUST be told that missing capabilities are
/// reachable via `capability_search` and builtin schemas via
/// `tool_search`. Without this prompt teaching, the
/// LLM keeps hallucinating "no access" on text_to_speech / secret_write
/// / browser_navigate even though the runtime is now ready to serve them.
#[test]
fn test_captain_prompt_teaches_tool_search_pattern() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("capability_search"),
        "prompt must name capability_search as the first capability resolver"
    );
    assert!(
        prompt.contains("tool_search"),
        "prompt must still name tool_search for exact deferred builtin schemas"
    );
    assert!(
        prompt.contains("CORE"),
        "prompt must explain that only CORE tools are visible by default"
    );
    assert!(
        prompt.contains("select:"),
        "prompt must teach the select:name1,name2 exact-lookup form"
    );
    assert!(
        prompt.contains("je n'ai pas accès"),
        "prompt must explicitly forbid the 'I don't have access' reflex \
             that motivated the whole TS.* refactor"
    );
    // TS.3 nomenclature update — the SECURITY block must reference
    // tool_allowlist, not the legacy allowed_tools wording.
    assert!(
        prompt.contains("tool_allowlist"),
        "prompt must use the TS.3 field name `tool_allowlist`"
    );
}

/// C.3 — Captain has a manual now (`captain_docs`, C.2). The prompt
/// must explicitly tell the LLM to use it BEFORE falling back to
/// `ask_user`, otherwise the agent's reflex stays "ask the human"
/// and the audit bodies are dead weight.
#[test]
fn test_captain_prompt_teaches_rtfm_via_captain_docs() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("captain_docs"),
        "prompt must name captain_docs so Sonnet reaches for it"
    );
    assert!(
        prompt.contains("RTFM") || prompt.contains("manuel"),
        "prompt must frame the doc-first reflex"
    );
    assert!(
        prompt.contains("ask_user") || prompt.contains("avant de demander"),
        "prompt must position captain_docs ahead of ask_user in the choice tree"
    );
}

/// C.4 — Phase B added invisible-by-default constraints that Captain
/// will trip over silently if it doesn't know about them: subprocess
/// env_clear (B.1/B.2), per-skill env_inject (B.3), allowed_tools
/// enforcement (B.4), api_key on non-loopback bind (B.5), per-agent
/// Chrome user-data-dir (B.7), and the RBAC inversion that turns an
/// empty allowed_users into "deny all" (B.8). The prompt must list
/// the user-visible failure modes in one place so the LLM doesn't
/// generate code that quietly breaks.
#[test]
fn test_captain_prompt_teaches_phase_b_security_constraints() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("env_clear") || prompt.contains("env clear"),
        "prompt must explain that subprocess env is stripped"
    );
    assert!(
        prompt.contains("env_inject"),
        "prompt must surface the per-skill env_inject contract"
    );
    assert!(
        prompt.contains("Security blocked") && prompt.contains("secret_write"),
        "prompt must teach recovery from literal-secret sink guards"
    );
    assert!(
        // TS.3 renamed the field to tool_allowlist; accept either form
        // so the C.4 test stays useful as nomenclature evolves.
        prompt.contains("allowed_tools") || prompt.contains("tool_allowlist"),
        "prompt must mention the capability-allowlist enforcement"
    );
    assert!(
        prompt.contains("allowed_users") && prompt.contains("[\"*\"]"),
        "prompt must teach the empty=deny / wildcard=allow channel RBAC"
    );
    assert!(
        prompt.contains("loopback") || prompt.contains("non-loopback"),
        "prompt must mention the api_key requirement on non-loopback bind"
    );
}

#[test]
fn test_captain_prompt_routes_repeatable_workflows_through_learning_v2() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("EXTENSIBILITÉ"),
        "CAPTAIN_SYSTEM_PROMPT missing EXTENSIBILITÉ section"
    );
    assert!(
        prompt.contains("scaffold_skill"),
        "CAPTAIN_SYSTEM_PROMPT must name scaffold_skill"
    );
    assert!(
        prompt.contains("Learning V2") && prompt.contains("workflow_learning_list"),
        "repeatable workflows must route through the native Learning V2 control plane"
    );
    assert!(
        prompt.contains("ne contourne jamais") && prompt.contains("demande manuelle explicite"),
        "scaffold_skill must not bypass staging, tests, canary, rollback, or operator approval"
    );
    assert!(
        prompt.contains("skill_refinement_propose"),
        "existing skills must retain the separate refinement approval path"
    );
    assert!(
        prompt.contains("Auto-amélioration contrôlée"),
        "must describe the controlled self-improvement gate"
    );
    assert!(
        prompt.contains("self_improvement_review"),
        "must name the self-improvement review tool"
    );
    assert!(
        prompt.contains("attends l'action humaine exacte"),
        "critical self-improvements must require an exact human action"
    );
}

#[test]
fn test_captain_prompt_teaches_proactive_memory_save() {
    // Captain alignment: primary model must invoke memory_save itself when
    // the user asks to memorize something. The previous wording told the
    // LLM "you don't need to call memory tools, the system writes in the
    // background", which silenced Phase-1 entirely.
    let prompt = CAPTAIN_SYSTEM_PROMPT;

    assert!(
        prompt.contains("memory_save"),
        "CAPTAIN_SYSTEM_PROMPT must mention memory_save by name"
    );
    assert!(
        prompt.contains("SPONTANÉMENT") || prompt.contains("spontanément"),
        "memory section must use proactive guidance"
    );
    assert!(
        prompt.contains("faits déclaratifs")
            && prompt.contains("les workflows et modes opératoires vont dans les skills"),
        "prompt must keep memory declarative and route procedures to skills"
    );
    // Negative guard: drop the old phrasing that told the LLM not to call
    // memory tools at all — that wording dissuaded Sonnet from acting.
    assert!(
        !prompt.contains("PAS besoin d'appeler les outils mempalace"),
        "the old anti-incentive phrasing must be removed"
    );
}

#[test]
fn test_captain_prompt_forbids_memory_showcase() {
    let prompt = CAPTAIN_SYSTEM_PROMPT;
    assert!(
        prompt.contains("contexte silencieux") || prompt.contains("Use memory silently"),
        "prompt must teach memory as silent context"
    );
    assert!(
        prompt.contains("Ne liste jamais des détails personnels")
            || prompt.contains("Do not list or name personal memories"),
        "prompt must forbid unsolicited personal-memory recitation"
    );
    assert!(
        prompt.contains("memory_forget first")
            || prompt.contains("call memory_forget first")
            || (prompt.contains("memory_forget") && prompt.contains("d'abord")),
        "prompt must teach forget/correction before confirmation"
    );
}

#[test]
fn test_captain_prompt_teaches_remote_access_and_search_first() {
    // Sonnet was regressing on SSH workflows: instead of resolving a
    // familiar server name
    // via secret_read/config_read and running ssh_exec, it asked the user
    // for the IP. The system prompt must explicitly teach the resolution
    // path before falling back to ask_user.
    let prompt = CAPTAIN_SYSTEM_PROMPT;

    assert!(
        prompt.contains("ACCÈS DISTANTS"),
        "CAPTAIN_SYSTEM_PROMPT missing ACCÈS DISTANTS section"
    );
    assert!(
        prompt.contains("ssh_exec"),
        "CAPTAIN_SYSTEM_PROMPT missing ssh_exec guidance"
    );
    assert!(
        prompt.contains("secret_read"),
        "CAPTAIN_SYSTEM_PROMPT missing secret_read guidance"
    );
    assert!(
        prompt.contains("CHERCHE AVANT DE DEMANDER"),
        "CAPTAIN_SYSTEM_PROMPT missing CHERCHE AVANT DE DEMANDER section"
    );
    assert!(
        prompt.contains("absente du CORE visible") && prompt.contains("capability_search"),
        "CAPTAIN_SYSTEM_PROMPT must require capability_search when domain capability is not visible"
    );
    assert!(
        prompt.contains("knowledge_query"),
        "CAPTAIN_SYSTEM_PROMPT missing knowledge_query as resolution path"
    );
}
