pub(crate) fn full_tui_help(lang: crate::i18n::Lang) -> &'static str {
    crate::i18n::t("help.body", lang)
}

pub(crate) fn standalone_help(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Fr => concat!(
            "/help         — afficher cette aide\n",
            "/model        — ouvrir le sélecteur de modèle (Ctrl+M)\n",
            "/model <nom>  — changer de modèle directement\n",
            "/status       — info connexion & agent\n",
            "/agents       — lister les agents actifs\n",
            "/sessions     — lister toutes les sessions persistées\n",
            "/resume <id|titre> — restaurer une session depuis toute surface\n",
            "/tasks        — alias de /sessions\n",
            "/health       — santé du daemon\n",
            "/version      — version daemon et chemins\n",
            "/config       — config.toml exacte (owner only)\n",
            "/restart      — redémarrer le daemon (owner only)\n",
            "/shutdown confirm — arrêter le daemon (owner only)\n",
            "/clear        — effacer l'historique du chat\n",
            "/new          — démarrer une nouvelle session persistée\n",
            "/top          — remonter au début du scrollback TUI\n",
            "/bottom       — revenir au bas du chat\n",
            "/retry        — renvoyer le dernier message utilisateur\n",
            "/undo         — annuler le dernier échange\n",
            "/queue        — lister les messages en file\n",
            "/tokens       — afficher les tokens de la session\n",
            "/cost         — afficher le coût estimé de la session\n",
            "/export       — exporter la session en markdown\n",
            "/history      — ouvrir l'historique de sessions\n",
            "/copy [commande] — copier la dernière réponse ou commande tool-call\n",
            "/mouse [on/off] — clics + scroll souris (off = sélection native)\n",
            "Scroll       — PgUp/PgDn, Ctrl+B/Ctrl+F, molette/touchpad\n",
            "Hubs TUI: /projects, /automation, /learning, /skills, /dashboard via `captain tui`.\n",
            "/kill         — tuer l'agent courant (Captain protégé)\n",
            "/exit         — quitter la session de chat"
        ),
        crate::i18n::Lang::En => concat!(
            "/help         — show this help\n",
            "/model        — open model picker (Ctrl+M)\n",
            "/model <name> — switch model directly\n",
            "/status       — connection & agent info\n",
            "/agents       — list running agents\n",
            "/sessions     — list every persisted session\n",
            "/resume <id|title> — restore a session from any surface\n",
            "/tasks        — alias of /sessions\n",
            "/health       — daemon health\n",
            "/version      — daemon version and paths\n",
            "/config       — exact config.toml (owner only)\n",
            "/restart      — restart daemon (owner only)\n",
            "/shutdown confirm — stop daemon (owner only)\n",
            "/clear        — clear chat history\n",
            "/new          — start a new persisted session\n",
            "/top          — jump to the top of TUI scrollback\n",
            "/bottom       — return to the bottom of the chat\n",
            "/retry        — re-send the last user message\n",
            "/undo         — drop the last exchange\n",
            "/queue        — list staged messages\n",
            "/tokens       — show session token usage\n",
            "/cost         — show estimated session cost\n",
            "/export       — export the session as markdown\n",
            "/history      — open saved sessions\n",
            "/copy [command] — copy last response or tool-call command\n",
            "/mouse [on/off] — mouse clicks + wheel scrolling (off = native selection)\n",
            "Scroll       — PgUp/PgDn, Ctrl+B/Ctrl+F, wheel/touchpad\n",
            "TUI hubs: /projects, /automation, /learning, /skills, /dashboard via `captain tui`.\n",
            "/kill         — kill the current agent (Captain protected)\n",
            "/exit         — end chat session"
        ),
    }
}

#[cfg(test)]
#[path = "slash_help/tests.rs"]
mod tests;
