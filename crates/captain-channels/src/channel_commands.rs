//! Canonical user-facing command catalogue shared by channel parsing and UX.

use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandLanguage {
    English,
    French,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandSection {
    General,
    Session,
    Info,
    Automation,
    Review,
    Monitoring,
    Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandDiscoverability {
    Active,
    Compatibility,
}

impl CommandSection {
    fn label(self, language: CommandLanguage) -> &'static str {
        match (self, language) {
            (Self::General, CommandLanguage::English) => "General",
            (Self::Session, CommandLanguage::English) => "Session",
            (Self::Info, CommandLanguage::English) => "Info",
            (Self::Automation, CommandLanguage::English) => "Automation",
            (Self::Review, CommandLanguage::English) => "Review",
            (Self::Monitoring, CommandLanguage::English) => "Monitoring",
            (Self::Channel, CommandLanguage::English) => "Channel",
            (Self::General, CommandLanguage::French) => "Général",
            (Self::Session, CommandLanguage::French) => "Session",
            (Self::Info, CommandLanguage::French) => "Informations",
            (Self::Automation, CommandLanguage::French) => "Automatisation",
            (Self::Review, CommandLanguage::French) => "Validation",
            (Self::Monitoring, CommandLanguage::French) => "Supervision",
            (Self::Channel, CommandLanguage::French) => "Canal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelCommandSpec {
    pub(crate) name: &'static str,
    pub(crate) usage: &'static str,
    pub(crate) section: CommandSection,
    pub(crate) discoverability: CommandDiscoverability,
    description_en: &'static str,
    description_fr: &'static str,
}

impl ChannelCommandSpec {
    pub(crate) fn description(self, language: CommandLanguage) -> &'static str {
        match language {
            CommandLanguage::English => self.description_en,
            CommandLanguage::French => self.description_fr,
        }
    }
}

const fn command(
    name: &'static str,
    usage: &'static str,
    section: CommandSection,
    description_en: &'static str,
    description_fr: &'static str,
) -> ChannelCommandSpec {
    ChannelCommandSpec {
        name,
        usage,
        section,
        discoverability: CommandDiscoverability::Active,
        description_en,
        description_fr,
    }
}

const fn compatibility_command(
    name: &'static str,
    usage: &'static str,
    section: CommandSection,
    description_en: &'static str,
    description_fr: &'static str,
) -> ChannelCommandSpec {
    ChannelCommandSpec {
        name,
        usage,
        section,
        discoverability: CommandDiscoverability::Compatibility,
        description_en,
        description_fr,
    }
}

pub(crate) const USER_CHANNEL_COMMANDS: &[ChannelCommandSpec] = &[
    command(
        "start",
        "start",
        CommandSection::General,
        "Show the welcome message",
        "Afficher le message de bienvenue",
    ),
    command(
        "help",
        "help",
        CommandSection::General,
        "Show every available command",
        "Afficher toutes les commandes disponibles",
    ),
    command(
        "agents",
        "agents",
        CommandSection::General,
        "List running agents",
        "Lister les agents actifs",
    ),
    command(
        "agent",
        "agent <name>",
        CommandSection::General,
        "Select an agent by name",
        "Choisir un agent par son nom",
    ),
    command(
        "new",
        "new",
        CommandSection::Session,
        "Start a new persistent session",
        "Démarrer une nouvelle session persistante",
    ),
    command(
        "clear",
        "clear",
        CommandSection::Session,
        "Alias for starting a new session",
        "Alias pour démarrer une nouvelle session",
    ),
    command(
        "compact",
        "compact",
        CommandSection::Session,
        "Compact the current model context",
        "Compacter le contexte courant du modèle",
    ),
    command(
        "model",
        "model [name]",
        CommandSection::Session,
        "Inspect or select the agent model",
        "Voir ou choisir le modèle de l'agent",
    ),
    command(
        "stop",
        "stop",
        CommandSection::Session,
        "Cancel the current agent run",
        "Annuler l'exécution en cours",
    ),
    command(
        "usage",
        "usage",
        CommandSection::Session,
        "Show session usage and provider quota",
        "Voir l'usage de session et le quota provider",
    ),
    command(
        "reasoning",
        "reasoning [auto|level]",
        CommandSection::Session,
        "Inspect or select model reasoning",
        "Voir ou choisir le niveau de réflexion",
    ),
    command(
        "think",
        "think",
        CommandSection::Session,
        "Explain the terminal-only thinking toggle",
        "Expliquer l'affichage terminal des réflexions",
    ),
    command(
        "models",
        "models",
        CommandSection::Info,
        "List available AI models",
        "Lister les modèles IA disponibles",
    ),
    command(
        "providers",
        "providers",
        CommandSection::Info,
        "Show configured model providers",
        "Voir les providers de modèles configurés",
    ),
    command(
        "skills",
        "skills",
        CommandSection::Info,
        "List installed skills",
        "Lister les skills installés",
    ),
    compatibility_command(
        "hands",
        "hands",
        CommandSection::Info,
        "List compatible hands",
        "Lister les hands compatibles",
    ),
    command(
        "status",
        "status",
        CommandSection::Info,
        "Show system status",
        "Afficher l'état du système",
    ),
    command(
        "health",
        "health",
        CommandSection::Info,
        "Show daemon health",
        "Afficher la santé du daemon",
    ),
    command(
        "version",
        "version",
        CommandSection::Info,
        "Show daemon version and paths",
        "Afficher la version et les chemins du daemon",
    ),
    command(
        "config",
        "config",
        CommandSection::Info,
        "Show exact config (owner only)",
        "Afficher la configuration exacte (propriétaire)",
    ),
    command(
        "reload",
        "reload",
        CommandSection::Info,
        "Reload config (owner only)",
        "Recharger la configuration (propriétaire)",
    ),
    command(
        "restart",
        "restart",
        CommandSection::Info,
        "Restart the daemon (owner only)",
        "Redémarrer le daemon (propriétaire)",
    ),
    command(
        "shutdown",
        "shutdown confirm",
        CommandSection::Info,
        "Stop the daemon (owner only)",
        "Arrêter le daemon (propriétaire)",
    ),
    command(
        "workflows",
        "workflows",
        CommandSection::Automation,
        "List workflows",
        "Lister les workflows",
    ),
    command(
        "workflow",
        "workflow run <name> [input]",
        CommandSection::Automation,
        "Run a workflow",
        "Exécuter un workflow",
    ),
    command(
        "triggers",
        "triggers",
        CommandSection::Automation,
        "List event triggers",
        "Lister les déclencheurs d'événements",
    ),
    command(
        "trigger",
        "trigger <add|del> ...",
        CommandSection::Automation,
        "Create or remove a trigger",
        "Créer ou supprimer un déclencheur",
    ),
    command(
        "schedules",
        "schedules",
        CommandSection::Automation,
        "List scheduled jobs",
        "Lister les tâches planifiées",
    ),
    command(
        "schedule",
        "schedule <add|del|run> ...",
        CommandSection::Automation,
        "Manage scheduled jobs",
        "Gérer les tâches planifiées",
    ),
    command(
        "approvals",
        "approvals",
        CommandSection::Review,
        "List pending approvals and rules",
        "Lister les validations et règles en attente",
    ),
    command(
        "approve",
        "approve <id>",
        CommandSection::Review,
        "Approve one request once",
        "Valider une demande une fois",
    ),
    command(
        "approve_session",
        "approve_session <id>",
        CommandSection::Review,
        "Approve the exact action for this session",
        "Valider l'action exacte pour cette session",
    ),
    command(
        "approve_always",
        "approve_always <id>",
        CommandSection::Review,
        "Persist an exact-action allow rule",
        "Mémoriser une autorisation pour l'action exacte",
    ),
    command(
        "reject",
        "reject <id> [reason]",
        CommandSection::Review,
        "Reject one request",
        "Refuser une demande",
    ),
    command(
        "reject_session",
        "reject_session <id> [reason]",
        CommandSection::Review,
        "Reject the exact action for this session",
        "Refuser l'action exacte pour cette session",
    ),
    command(
        "reject_always",
        "reject_always <id> <reason>",
        CommandSection::Review,
        "Persist an exact-action deny rule",
        "Mémoriser un refus pour l'action exacte",
    ),
    command(
        "approval_rule_revoke",
        "approval_rule_revoke <id>",
        CommandSection::Review,
        "Revoke a durable approval rule",
        "Révoquer une règle de validation persistante",
    ),
    command(
        "learning",
        "learning",
        CommandSection::Review,
        "Show the live Learning engine",
        "Afficher le moteur Learning actif",
    ),
    command(
        "learnings",
        "learnings",
        CommandSection::Review,
        "List pending learning candidates",
        "Lister les apprentissages en attente",
    ),
    command(
        "learn_approve",
        "learn_approve <id>",
        CommandSection::Review,
        "Approve a learning candidate",
        "Valider un apprentissage",
    ),
    command(
        "learn_reject",
        "learn_reject <id>",
        CommandSection::Review,
        "Reject a learning candidate",
        "Refuser un apprentissage",
    ),
    command(
        "skill_refinements",
        "skill_refinements",
        CommandSection::Review,
        "List skill refinements",
        "Lister les améliorations de skills",
    ),
    command(
        "skill_refine_approve",
        "skill_refine_approve <id>",
        CommandSection::Review,
        "Approve a skill refinement",
        "Valider une amélioration de skill",
    ),
    command(
        "skill_refine_reject",
        "skill_refine_reject <id>",
        CommandSection::Review,
        "Reject a skill refinement",
        "Refuser une amélioration de skill",
    ),
    command(
        "project_answer",
        "project_answer <id> <answer>",
        CommandSection::Review,
        "Answer a pending project question",
        "Répondre à une question de projet",
    ),
    command(
        "budget",
        "budget",
        CommandSection::Monitoring,
        "Show Captain spending limits",
        "Afficher les limites de dépense Captain",
    ),
    compatibility_command(
        "peers",
        "peers",
        CommandSection::Monitoring,
        "Show compatible peer status",
        "Afficher l'état des peers compatibles",
    ),
    compatibility_command(
        "a2a",
        "a2a",
        CommandSection::Monitoring,
        "List compatible external agents",
        "Lister les agents externes compatibles",
    ),
    command(
        "sethome",
        "sethome [chat_id]",
        CommandSection::Channel,
        "Register this chat as the home channel",
        "Définir ce chat comme canal principal",
    ),
    command(
        "gethome",
        "gethome",
        CommandSection::Channel,
        "Show the current home channel",
        "Afficher le canal principal actuel",
    ),
];

pub(crate) fn command_spec(command_name: &str) -> Option<&'static ChannelCommandSpec> {
    let command_name = command_name.trim_start_matches('/');
    let command_name = command_name.split('@').next().unwrap_or(command_name);
    USER_CHANNEL_COMMANDS
        .iter()
        .find(|command| command.name == command_name)
}

pub(crate) fn is_user_channel_command(command_name: &str) -> bool {
    command_spec(command_name).is_some()
}

pub(crate) fn active_channel_commands() -> impl Iterator<Item = &'static ChannelCommandSpec> {
    USER_CHANNEL_COMMANDS
        .iter()
        .filter(|command| command.discoverability == CommandDiscoverability::Active)
}

pub(crate) fn format_command_help(language: CommandLanguage) -> String {
    let mut output = match language {
        CommandLanguage::English => "Captain Bot Commands:\n".to_string(),
        CommandLanguage::French => "Commandes du bot Captain :\n".to_string(),
    };
    let mut current_section = None;
    for command in active_channel_commands() {
        if current_section != Some(command.section) {
            current_section = Some(command.section);
            let _ = write!(output, "\n{}:\n", command.section.label(language));
        }
        let _ = writeln!(
            output,
            "/{} - {}",
            command.usage,
            command.description(language)
        );
    }
    output.pop();
    output
}

pub(crate) fn format_command_subset(command_names: &[&str], language: CommandLanguage) -> String {
    command_names
        .iter()
        .filter_map(|name| command_spec(name))
        .map(|command| format!("/{} - {}", command.usage, command.description(language)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalogue_is_unique_and_valid_for_telegram_bot_commands() {
        assert!(USER_CHANNEL_COMMANDS.len() <= 100);
        let mut names = HashSet::new();
        for command in USER_CHANNEL_COMMANDS {
            assert!(names.insert(command.name), "duplicate {}", command.name);
            assert!((1..=32).contains(&command.name.chars().count()));
            assert!(command
                .name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'));
            for language in [CommandLanguage::English, CommandLanguage::French] {
                assert!((1..=256).contains(&command.description(language).chars().count()));
            }
            assert!(
                command.usage == command.name
                    || command.usage.starts_with(&format!("{} ", command.name))
            );
        }
    }

    #[test]
    fn help_and_lookup_share_the_complete_catalogue() {
        let help = format_command_help(CommandLanguage::English);
        assert!(help.len() <= 4096, "Telegram help is {} bytes", help.len());
        for command in USER_CHANNEL_COMMANDS {
            let listed = help.contains(&format!("\n/{} - ", command.usage));
            assert_eq!(
                listed,
                command.discoverability == CommandDiscoverability::Active,
                "unexpected help visibility for /{}",
                command.name
            );
            assert!(is_user_channel_command(command.name));
            assert!(is_user_channel_command(&format!(
                "/{}@CaptainBot",
                command.name
            )));
        }
        assert_eq!(USER_CHANNEL_COMMANDS.len(), 50);
        assert_eq!(active_channel_commands().count(), 47);
    }

    #[test]
    fn compact_subset_uses_the_same_descriptions() {
        let text = format_command_subset(&["agents", "agent", "help"], CommandLanguage::English);
        assert!(text.contains("/agents - List running agents"));
        assert!(text.contains("/agent <name> - Select an agent by name"));
        assert!(text.contains("/help - Show every available command"));
    }
}
