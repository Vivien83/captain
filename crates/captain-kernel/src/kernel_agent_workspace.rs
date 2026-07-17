use crate::error::{KernelError, KernelResult};
use captain_types::agent::AgentManifest;
use captain_types::error::CaptainError;
use std::path::Path;

/// Create workspace directory structure for an agent.
pub(crate) fn ensure_workspace(workspace: &Path) -> KernelResult<()> {
    captain_types::durable_fs::create_dir_all(workspace).map_err(|e| {
        KernelError::Captain(CaptainError::Internal(format!(
            "Failed to create workspace {}: {e}",
            workspace.display()
        )))
    })?;
    for subdir in &["data", "output", "sessions", "skills", "logs", "memory"] {
        captain_types::durable_fs::create_dir_all(&workspace.join(subdir)).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to create workspace dir {}/{subdir}: {e}",
                workspace.display()
            )))
        })?;
    }
    // Write agent metadata file (best-effort)
    let meta = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "workspace": workspace.display().to_string(),
    });
    let serialized = serde_json::to_string_pretty(&meta).unwrap_or_default();
    let _ = captain_types::durable_fs::atomic_write(
        &workspace.join("AGENT.json"),
        serialized.as_bytes(),
    );
    Ok(())
}

/// Generate lean workspace identity files for an agent (SOUL.md, IDENTITY.md).
///
/// `USER.md` is intentionally global-only (`~/.captain/USER.md`). Workspace
/// copies caused agent-specific placeholders to shadow the real user profile.
/// Uses `create_new` to never overwrite existing files (preserves user edits).
pub(crate) fn generate_identity_files(workspace: &Path, manifest: &AgentManifest) {
    let soul_content = format!(
        "# Soul\n\
         You are {}. {}\n\
         Be genuinely helpful. Have opinions. Be resourceful before asking.\n\
         Treat user data with respect \u{2014} you are a guest in their life.\n",
        manifest.name,
        if manifest.description.is_empty() {
            "You are a helpful AI agent."
        } else {
            &manifest.description
        }
    );

    let identity_content = format!(
        "---\n\
         name: {name}\n\
         archetype: assistant\n\
         vibe: helpful\n\
         emoji:\n\
         avatar_url:\n\
         greeting_style: warm\n\
         color:\n\
         ---\n\
         # Identity\n\
         <!-- Visual identity and personality at a glance. Edit these fields freely. -->\n",
        name = manifest.name
    );

    let files: &[(&str, &str)] = &[
        ("SOUL.md", &soul_content),
        ("IDENTITY.md", &identity_content),
    ];

    // Conditionally generate HEARTBEAT.md for autonomous agents.
    let heartbeat_content = if manifest.autonomous.is_some() {
        Some(
            "# Heartbeat Checklist\n\
             <!-- Proactive reminders to check during heartbeat cycles -->\n\n\
             ## Every Heartbeat\n\
             - [ ] Check for pending tasks or messages\n\
             - [ ] Review memory for stale items\n\n\
             ## Daily\n\
             - [ ] Summarize today's activity for the user\n\n\
             ## Weekly\n\
             - [ ] Archive old sessions and clean up memory\n"
                .to_string(),
        )
    } else {
        None
    };

    for (filename, content) in files {
        let _ =
            captain_types::durable_fs::create_new(&workspace.join(filename), content.as_bytes());
    }

    // Write HEARTBEAT.md for autonomous agents.
    if let Some(ref hb) = heartbeat_content {
        let _ =
            captain_types::durable_fs::create_new(&workspace.join("HEARTBEAT.md"), hb.as_bytes());
    }
}

/// Domain-specific prompt templates for Fleet Managers.
pub(crate) fn manager_domain_template(name: &str, domain: &str) -> String {
    let lower = name.to_lowercase();
    let domain_lower = domain.to_lowercase();

    if lower.contains("research")
        || domain_lower.contains("recherche")
        || domain_lower.contains("research")
    {
        "GUIDE DOMAINE — RECHERCHE :\n\
         - Décompose la question en sous-recherches parallélisables.\n\
         - Spawn 1 worker par angle de recherche (max 3).\n\
         - Workers utilisent web_research_batch, web_fetch, et web_download+document_extract pour PDF/rapports.\n\
         - Agrège les résultats, élimine les doublons, vérifie les contradictions, synthétise.\n\
         - Cite toujours les sources réellement lues/extraites."
            .to_string()
    } else if lower.contains("trading")
        || lower.contains("finance")
        || domain_lower.contains("trading")
    {
        "GUIDE DOMAINE — TRADING/FINANCE :\n\
         - Worker 1 : collecte de données (prix, news, sentiment).\n\
         - Worker 2 : analyse technique/fondamentale.\n\
         - JAMAIS exécuter un trade sans confirmation Captain.\n\
         - Toujours vérifier les données avant analyse.\n\
         - Rapporte avec chiffres précis et niveau de confiance."
            .to_string()
    } else if lower.contains("ops")
        || lower.contains("devops")
        || domain_lower.contains("opération")
    {
        "GUIDE DOMAINE — OPÉRATIONS :\n\
         - Workers pour monitoring, alertes, maintenance.\n\
         - Utilise shell_exec pour diagnostics système.\n\
         - Log chaque action dans memory_store.\n\
         - Escalade vers Captain si action destructive nécessaire.\n\
         - Préfère les actions réversibles."
            .to_string()
    } else if lower.contains("content")
        || lower.contains("redac")
        || domain_lower.contains("contenu")
    {
        "GUIDE DOMAINE — CONTENU :\n\
         - Worker 1 : recherche/outline.\n\
         - Worker 2 : rédaction.\n\
         - Toi : review + correction finale.\n\
         - Respecte le ton et le style demandés.\n\
         - Vérifie les faits avant publication."
            .to_string()
    } else if lower.contains("security")
        || lower.contains("securite")
        || domain_lower.contains("sécurité")
    {
        "GUIDE DOMAINE — SÉCURITÉ :\n\
         - Worker scan : shell_exec pour audits (ports, configs).\n\
         - Worker analyse : interprétation des résultats.\n\
         - JAMAIS exploiter une vulnérabilité sans accord Captain.\n\
         - Classifie par sévérité (CRITICAL/HIGH/MEDIUM/LOW).\n\
         - Propose des remédiations concrètes."
            .to_string()
    } else {
        format!(
            "GUIDE DOMAINE — {domain} :\n\
             - Adapte ta stratégie au domaine.\n\
             - Spawn des workers spécialisés si la tâche est décomposable.\n\
             - Rapporte les résultats de manière structurée.\n\
             - Demande à Captain si tu as besoin de clarification."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::AutonomousConfig;

    fn manifest(name: &str, description: &str) -> AgentManifest {
        AgentManifest {
            name: name.to_string(),
            description: description.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn workspace_setup_creates_expected_directories_and_metadata() {
        let dir = tempfile::tempdir().unwrap();

        ensure_workspace(dir.path()).expect("workspace");

        for subdir in ["data", "output", "sessions", "skills", "logs", "memory"] {
            assert!(dir.path().join(subdir).is_dir(), "{subdir} should exist");
        }
        let metadata = std::fs::read_to_string(dir.path().join("AGENT.json")).unwrap();
        assert!(metadata.contains("\"workspace\""));
        assert!(metadata.contains(&dir.path().display().to_string()));
    }

    #[test]
    fn identity_files_are_lean_and_preserve_existing_edits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "custom soul").unwrap();

        generate_identity_files(dir.path(), &manifest("captain", "Principal assistant"));

        assert_eq!(
            std::fs::read_to_string(dir.path().join("SOUL.md")).unwrap(),
            "custom soul"
        );
        assert!(dir.path().join("IDENTITY.md").exists());
        for retired in [
            "USER.md",
            "MEMORY.md",
            "BOOTSTRAP.md",
            "PLAYBOOK.md",
            "TOOLS.md",
            "AGENTS.md",
        ] {
            assert!(
                !dir.path().join(retired).exists(),
                "{retired} should not be generated by default"
            );
        }
    }

    #[test]
    fn autonomous_identity_generation_adds_heartbeat_checklist() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = manifest("ops", "");
        manifest.autonomous = Some(AutonomousConfig::default());

        generate_identity_files(dir.path(), &manifest);

        let heartbeat = std::fs::read_to_string(dir.path().join("HEARTBEAT.md")).unwrap();
        assert!(heartbeat.contains("Every Heartbeat"));
        assert!(heartbeat.contains("Daily"));
    }

    #[test]
    fn manager_domain_template_selects_specialized_guides() {
        assert!(
            manager_domain_template("research manager", "").contains("GUIDE DOMAINE — RECHERCHE")
        );
        assert!(manager_domain_template("ops manager", "").contains("GUIDE DOMAINE — OPÉRATIONS"));
        assert!(manager_domain_template("custom", "support").contains("GUIDE DOMAINE — support"));
    }
}
