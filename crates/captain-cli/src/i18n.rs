//! Lightweight CLI i18n.
//!
//! Reads the top-level `language` key from `~/.captain/config.toml` with a
//! hand-rolled scan (no full TOML parse needed — the boot banner runs before
//! the kernel loads the real config). Falls back to English on any error.

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Fr,
}

impl Lang {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "fr" | "fr-fr" | "fr_fr" | "french" | "francais" | "français" => Lang::Fr,
            _ => Lang::En,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("CAPTAIN_HOME") {
        return Some(PathBuf::from(h).join("config.toml"));
    }
    dirs::home_dir().map(|h| h.join(".captain").join("config.toml"))
}

/// Scan the user config for `language = "xx"` without pulling in a TOML parser.
/// Top-level keys only — subtables are skipped once we hit a `[...]` header.
/// Callers should prefer `current()` which caches the result — this fn exposes
/// the raw lookup for tests and the one-time cache fill.
pub fn detect_from_config() -> Lang {
    let Some(path) = config_path() else {
        return Lang::default();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Lang::default();
    };
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            break;
        }
        if let Some(rest) = line.strip_prefix("language") {
            if let Some(eq) = rest.find('=') {
                let value = rest[eq + 1..].trim().trim_matches('"').trim_matches('\'');
                return Lang::parse(value);
            }
        }
    }
    Lang::default()
}

/// Process-wide cached language. Read once from config on the first call and
/// never re-read afterwards — changing `language` in config.toml requires a
/// restart, which is deliberate (we'd otherwise re-scan the file at every
/// draw tick and slash-command dispatch).
pub fn current() -> Lang {
    static CACHE: OnceLock<Lang> = OnceLock::new();
    *CACHE.get_or_init(detect_from_config)
}

const FRENCH_TRANSLATIONS: &[(&str, &str)] = &[
    ("banner.subtitle", "SYSTÈME D'AGENTS IA"),
    (
        "chat.empty.primary",
        "Envoyez un message pour commencer.",
    ),
    (
        "chat.empty.secondary",
        "Tapez /help pour la liste des commandes.",
    ),
    ("launcher.no_daemon", "aucun daemon"),
    ("launcher.no_provider", "aucune clé API détectée"),
    (
        "chat.hints.default",
        "    [Entrée] Envoyer  [Ctrl+E] Tool  [Ctrl+M] Modèles  [\u{2191}\u{2193}] Défiler  [Échap] Retour",
    ),
    (
        "chat.hints.streaming",
        "    [Entrée] Mettre en file  [\u{2191}\u{2193}] Défiler  [Échap] Stop",
    ),
    (
        "chat.hints.model_picker",
        "    [\u{2191}\u{2193}] Naviguer  [Entrée] Sélectionner  [Échap] Fermer  [texte] Filtrer",
    ),
    ("chat.cleared", "Historique effacé."),
    ("chat.unknown_cmd", "Commande inconnue. Tapez /help."),
    ("chat.no_backend", "Aucun backend connecté."),
    (
        "kill.captain_protected",
        "Captain est l'agent principal et ne peut pas être tué. Utilisez /exit pour quitter le chat ou stoppez le daemon.",
    ),
    (
        "sessions.inproc_only",
        "Les sessions ne sont suivies qu'en mode daemon.",
    ),
    ("sessions.not_connected", "Non connecté."),
    ("sessions.empty", "Aucune session récente."),
    ("agents.empty", "Aucun agent actif."),
    ("status.mode_daemon", "Mode : daemon ({url})"),
    ("status.mode_inproc", "Mode : in-process"),
    ("status.mode_disconnected", "Mode : déconnecté"),
    ("status.agent_count", "Agents : {count}"),
    ("status.current_agent", "Agent : {name}"),
    ("welcome.tagline", "Système d'exploitation d'agents"),
    ("welcome.no_daemon", "Aucun daemon actif"),
    ("welcome.no_provider", "Aucune clé API détectée"),
    ("welcome.menu.connect_daemon", "Se connecter au daemon"),
    (
        "welcome.menu.connect_daemon.hint",
        "discuter avec les agents via l'API",
    ),
    ("welcome.menu.inprocess", "Chat local (in-process)"),
    (
        "welcome.menu.inprocess.hint",
        "démarre le kernel local, sans daemon",
    ),
    ("welcome.menu.wizard", "Assistant de configuration"),
    ("welcome.menu.wizard.hint", "fournisseurs & canaux"),
    ("welcome.menu.exit", "Quitter"),
    ("welcome.menu.exit.hint", "quitter Captain"),
    ("logs.loading", "Chargement des logs…"),
    ("logs.no_match", "  Aucun log ne correspond au filtre."),
    ("logs.level_label", "  Niveau : "),
    ("logs.entries_count", "  ({count} entrées)"),
    ("logs.auto_on", " [auto-refresh ON]"),
    ("logs.auto_off", " [auto-refresh OFF]"),
    ("logs.filter", "  filtre : \"{query}\""),
    ("memory.loading_agents", "Chargement des agents…"),
    ("memory.no_agents", "  Aucun agent disponible."),
    ("memory.loading", "Chargement…"),
    ("memory.key_label", "  Clé : "),
    ("memory.value_label", "  Valeur : "),
    (
        "settings.loading_providers",
        "Chargement des fournisseurs…",
    ),
    ("settings.loading_models", "Chargement des modèles…"),
    ("settings.no_models", "  Aucun modèle disponible."),
    ("settings.no_tools", "  Aucun outil disponible."),
    ("retry.nothing", "Rien à renvoyer (aucun message précédent)."),
    ("undo.done", "Dernier échange annulé."),
    ("undo.nothing", "Rien à annuler."),
    (
        "fortune.quote.1",
        "L'ennemi de mon art, c'est l'improvisation sans préparation.",
    ),
    (
        "fortune.quote.2",
        "Le code qui se lit comme un roman se maintient comme un journal.",
    ),
    (
        "fortune.quote.3",
        "Avant de chercher la bonne réponse, questionne la question.",
    ),
    (
        "fortune.quote.4",
        "Un test raté aujourd'hui est une production sauvée demain.",
    ),
    (
        "fortune.quote.5",
        "La meilleure abstraction est celle qu'on peut supprimer sans douleur.",
    ),
    ("queue.empty", "La file d'envoi est vide."),
    (
        "queue.header",
        "Messages en file (auto-envoyés à la fin du stream) :",
    ),
    ("approvals.title", "Approbation requise"),
    ("approvals.choice.once", "Approuver une fois"),
    ("approvals.choice.session", "Approuver pour la session"),
    ("approvals.choice.always", "Approuver toujours"),
    ("approvals.choice.decline", "Rejeter"),
    (
        "approvals.choice.once.hint",
        "n'approuve que cet appel — re-prompt au suivant",
    ),
    (
        "approvals.choice.session.hint",
        "approuve tous les appels à cet outil pour cet agent jusqu'au redémarrage",
    ),
    (
        "approvals.choice.always.hint",
        "ajoute l'outil à allow_always — persisté en politique",
    ),
    (
        "approvals.choice.decline.hint",
        "rejette cet appel — re-prompt au suivant",
    ),
    (
        "approvals.hints.keys",
        "    [O] Une fois  [S] Session  [A] Toujours  [R] Rejeter  [Échap] Annuler",
    ),
    ("approvals.tool_label", "Outil :"),
    ("approvals.agent_label", "Agent :"),
    ("approvals.summary_label", "Action :"),
    ("approvals.timeout_label", "Délai :"),
    (
        "approvals.cleared",
        "{count} approbations de session oubliées.",
    ),
    (
        "approvals.granted_once",
        "Approuvé pour cet appel uniquement.",
    ),
    (
        "approvals.granted_session",
        "Approuvé pour la session — l'outil ne re-promptera plus.",
    ),
    (
        "approvals.granted_always",
        "Approuvé toujours — persisté dans la politique.",
    ),
    ("approvals.refused", "Rejeté."),
    ("kill.success", "Agent « {name} » tué."),
    ("kill.failed", "Échec : impossible de tuer « {name} »."),
    ("kill.error", "Échec du kill : {err}"),
    (
        "help.body",
        concat!(
            "/help         \u{2014} afficher cette aide\n",
            "/model        \u{2014} ouvrir le sélecteur de modèle (Ctrl+M)\n",
            "/model <nom>  \u{2014} changer de modèle directement\n",
            "/status       \u{2014} info connexion & agent\n",
            "/dashboard    \u{2014} ouvrir le cockpit Status\n",
            "/projects     \u{2014} ouvrir Projects\n",
            "/automation   \u{2014} ouvrir Automation\n",
            "/learning     \u{2014} ouvrir Learning\n",
            "/skills       \u{2014} ouvrir Capabilities\n",
            "/capabilities \u{2014} ouvrir Capabilities\n",
            "/budget       \u{2014} voir la consommation budgétaire (overlay)\n",
            "/new          \u{2014} nouvelle session persistée\n",
            "/resume <id|titre> \u{2014} restaurer une session depuis toute surface\n",
            "/history      \u{2014} ouvrir l'historique de sessions\n",
            "/export       \u{2014} exporter la session en markdown\n",
            "/tokens       \u{2014} afficher les tokens de la session\n",
            "/cost         \u{2014} afficher le coût estimé de la session\n",
            "/queue        \u{2014} lister les messages en file\n",
            "/clear        \u{2014} effacer l'historique du chat\n",
            "/top          \u{2014} remonter au début du scrollback TUI\n",
            "/bottom       \u{2014} revenir au bas du chat\n",
            "/retry        \u{2014} renvoyer le dernier message\n",
            "/undo         \u{2014} annuler le dernier échange\n",
            "/image <path> \u{2014} joindre une image au prochain message\n",
            "/file <path>  \u{2014} joindre un fichier (pdf/txt/md/audio…)\n",
            "/voice [secs] \u{2014} enregistre N s via sox/rec puis envoie (STT auto)\n",
            "/copy [command] \u{2014} copier la dernière réponse ou commande exacte\n",
            "/mouse [on/off] \u{2014} activer clics/scroll souris (off = sélection native)\n",
            "Raccourcis scroll: PgUp/PgDn, Ctrl+B/Ctrl+F, ↑/↓ hors brouillon multi-ligne\n",
            "Routes opérateur exactes: /agents, /sessions, /logs, /settings, /health, /version\n",
            "/exit         \u{2014} quitter la session de chat"
        ),
    ),
];

const ENGLISH_TRANSLATIONS: &[(&str, &str)] = &[
    ("banner.subtitle", "AI AGENT SYSTEM"),
    ("chat.empty.primary", "Send a message to start chatting."),
    (
        "chat.empty.secondary",
        "Type /help for available commands.",
    ),
    ("launcher.no_daemon", "no daemon"),
    ("launcher.no_provider", "no API key detected"),
    (
        "chat.hints.default",
        "    [Enter] Send  [Ctrl+E] Tool  [Ctrl+M] Models  [\u{2191}\u{2193}] Scroll  [Esc] Back",
    ),
    (
        "chat.hints.streaming",
        "    [Enter] Stage  [\u{2191}\u{2193}] Scroll  [Esc] Stop",
    ),
    (
        "chat.hints.model_picker",
        "    [\u{2191}\u{2193}] Navigate  [Enter] Select  [Esc] Close  [type] Filter",
    ),
    ("chat.cleared", "Chat history cleared."),
    ("chat.unknown_cmd", "Unknown command. Type /help."),
    ("chat.no_backend", "No backend connected."),
    (
        "kill.captain_protected",
        "Captain is the primary agent and cannot be killed. Use /exit to leave the chat or stop the daemon instead.",
    ),
    (
        "sessions.inproc_only",
        "Sessions are only tracked in daemon mode.",
    ),
    ("sessions.not_connected", "Not connected."),
    ("sessions.empty", "No recent sessions."),
    ("agents.empty", "No agents running."),
    ("status.mode_daemon", "Mode: daemon ({url})"),
    ("status.mode_inproc", "Mode: in-process"),
    ("status.mode_disconnected", "Mode: disconnected"),
    ("status.agent_count", "Agents: {count}"),
    ("status.current_agent", "Agent: {name}"),
    ("welcome.tagline", "Agent Operating System"),
    ("welcome.no_daemon", "No daemon running"),
    ("welcome.no_provider", "No API keys detected"),
    ("welcome.menu.connect_daemon", "Connect to daemon"),
    (
        "welcome.menu.connect_daemon.hint",
        "talk to running agents via API",
    ),
    ("welcome.menu.inprocess", "Quick in-process chat"),
    (
        "welcome.menu.inprocess.hint",
        "boot kernel locally, no daemon needed",
    ),
    ("welcome.menu.wizard", "Setup wizard"),
    ("welcome.menu.wizard.hint", "configure providers & channels"),
    ("welcome.menu.exit", "Exit"),
    ("welcome.menu.exit.hint", "quit Captain"),
    ("logs.loading", "Loading logs…"),
    (
        "logs.no_match",
        "  No log entries match the current filter.",
    ),
    ("logs.level_label", "  Level: "),
    ("logs.entries_count", "  ({count} entries)"),
    ("logs.auto_on", " [auto-refresh ON]"),
    ("logs.auto_off", " [auto-refresh OFF]"),
    ("logs.filter", "  filter: \"{query}\""),
    ("memory.loading_agents", "Loading agents…"),
    ("memory.no_agents", "  No agents available."),
    ("memory.loading", "Loading…"),
    ("memory.key_label", "  Key: "),
    ("memory.value_label", "  Value: "),
    ("settings.loading_providers", "Loading providers…"),
    ("settings.loading_models", "Loading models…"),
    ("settings.no_models", "  No models available."),
    ("settings.no_tools", "  No tools available."),
    (
        "retry.nothing",
        "Nothing to retry (no previous message).",
    ),
    ("undo.done", "Last exchange undone."),
    ("undo.nothing", "Nothing to undo."),
    (
        "fortune.quote.1",
        "The enemy of craft is improvisation without preparation.",
    ),
    (
        "fortune.quote.2",
        "Code that reads like prose is maintained like prose.",
    ),
    (
        "fortune.quote.3",
        "Before hunting the right answer, interrogate the question.",
    ),
    (
        "fortune.quote.4",
        "A failing test today is a saved production tomorrow.",
    ),
    (
        "fortune.quote.5",
        "The best abstraction is the one you can delete painlessly.",
    ),
    ("queue.empty", "The send queue is empty."),
    (
        "queue.header",
        "Queued messages (auto-sent after the current stream):",
    ),
    ("approvals.title", "Approval required"),
    ("approvals.choice.once", "Approve once"),
    ("approvals.choice.session", "Approve for session"),
    ("approvals.choice.always", "Approve always"),
    ("approvals.choice.decline", "Reject"),
    (
        "approvals.choice.once.hint",
        "approve this single call — re-prompt next time",
    ),
    (
        "approvals.choice.session.hint",
        "approve every call to this tool for this agent until daemon restart",
    ),
    (
        "approvals.choice.always.hint",
        "add the tool to allow_always — persisted into policy",
    ),
    (
        "approvals.choice.decline.hint",
        "reject this call — re-prompt next time",
    ),
    (
        "approvals.hints.keys",
        "    [O] Once  [S] Session  [A] Always  [R] Reject  [Esc] Cancel",
    ),
    ("approvals.tool_label", "Tool:"),
    ("approvals.agent_label", "Agent:"),
    ("approvals.summary_label", "Action:"),
    ("approvals.timeout_label", "Timeout:"),
    (
        "approvals.cleared",
        "{count} session approvals dropped.",
    ),
    ("approvals.granted_once", "Approved for this call only."),
    (
        "approvals.granted_session",
        "Approved for the session — tool will not re-prompt.",
    ),
    (
        "approvals.granted_always",
        "Approved always — persisted to policy.",
    ),
    ("approvals.refused", "Rejected."),
    ("kill.success", "Agent \"{name}\" killed."),
    ("kill.failed", "Failed to kill agent \"{name}\"."),
    ("kill.error", "Kill failed: {err}"),
    (
        "help.body",
        concat!(
            "/help         \u{2014} show this help\n",
            "/model        \u{2014} open model picker (Ctrl+M)\n",
            "/model <name> \u{2014} switch to model directly\n",
            "/status       \u{2014} connection & agent info\n",
            "/dashboard    \u{2014} open the Status cockpit\n",
            "/projects     \u{2014} open Projects\n",
            "/automation   \u{2014} open Automation\n",
            "/learning     \u{2014} open Learning\n",
            "/skills       \u{2014} open Capabilities\n",
            "/capabilities \u{2014} open Capabilities\n",
            "/budget       \u{2014} view spend & budget (overlay)\n",
            "/new          \u{2014} start a new persisted session\n",
            "/resume <id|title> \u{2014} restore a session from any surface\n",
            "/history      \u{2014} open saved sessions\n",
            "/export       \u{2014} export the session as markdown\n",
            "/tokens       \u{2014} show session token usage\n",
            "/cost         \u{2014} show estimated session cost\n",
            "/queue        \u{2014} list staged messages\n",
            "/clear        \u{2014} clear chat history\n",
            "/top          \u{2014} jump to the top of the TUI scrollback\n",
            "/bottom       \u{2014} return to the bottom of the chat\n",
            "/retry        \u{2014} re-send the last user message\n",
            "/undo         \u{2014} drop the last exchange\n",
            "/image <path> \u{2014} attach an image to the next message\n",
            "/file <path>  \u{2014} attach any file (pdf/txt/md/audio…)\n",
            "/voice [secs] \u{2014} record N s via sox/rec then send (auto STT)\n",
            "/copy [command] \u{2014} copy the last response or exact command\n",
            "/mouse [on/off] \u{2014} enable mouse clicks/wheel (off = native selection)\n",
            "Scroll shortcuts: PgUp/PgDn, Ctrl+B/Ctrl+F, ↑/↓ outside multi-line drafts\n",
            "Exact operator routes: /agents, /sessions, /logs, /settings, /health, /version\n",
            "/exit         \u{2014} end chat session"
        ),
    ),
];

/// Translate a known key. Unknown keys return the key itself so callers never
/// receive an empty string (lifetime tied to `key` for the fallback branch).
pub fn t(key: &str, lang: Lang) -> &str {
    if lang == Lang::Fr {
        if let Some(value) = lookup_translation(key, FRENCH_TRANSLATIONS) {
            return value;
        }
    }
    lookup_translation(key, ENGLISH_TRANSLATIONS).unwrap_or(key)
}

fn lookup_translation(key: &str, table: &[(&'static str, &'static str)]) -> Option<&'static str> {
    table
        .iter()
        .find_map(|(candidate, value)| (*candidate == key).then_some(*value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_french_variants() {
        assert_eq!(Lang::parse("fr"), Lang::Fr);
        assert_eq!(Lang::parse("FR"), Lang::Fr);
        assert_eq!(Lang::parse("fr-FR"), Lang::Fr);
        assert_eq!(Lang::parse("français"), Lang::Fr);
    }

    #[test]
    fn parse_falls_back_to_english() {
        assert_eq!(Lang::parse("en"), Lang::En);
        assert_eq!(Lang::parse("de"), Lang::En);
        assert_eq!(Lang::parse(""), Lang::En);
        assert_eq!(Lang::parse("xx-YY"), Lang::En);
    }

    #[test]
    fn t_returns_french_when_requested() {
        assert_eq!(t("banner.subtitle", Lang::Fr), "SYSTÈME D'AGENTS IA");
    }

    #[test]
    fn t_falls_back_to_english_on_unknown_key() {
        assert_eq!(t("nonexistent.key", Lang::Fr), "nonexistent.key");
    }

    #[test]
    fn t_english_is_default_language() {
        assert_eq!(t("banner.subtitle", Lang::En), "AI AGENT SYSTEM");
    }

    #[test]
    fn translation_tables_keep_matching_keys() {
        assert_eq!(FRENCH_TRANSLATIONS.len(), ENGLISH_TRANSLATIONS.len());
        for ((fr_key, _), (en_key, _)) in FRENCH_TRANSLATIONS.iter().zip(ENGLISH_TRANSLATIONS) {
            assert_eq!(fr_key, en_key);
        }
    }
}
