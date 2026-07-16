//! Static config, secret, and model-auth tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn config_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = base_config_tool_definitions();
    definitions.extend(model_switch_tool_definitions());
    definitions.extend(codex_auth_tool_definitions());
    definitions.extend(secret_tool_definitions());
    definitions.extend(setup_tool_definitions());
    definitions
}

fn base_config_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        config_read_tool_definition(),
        config_write_tool_definition(),
        config_schema_tool_definition(),
        self_configure_tool_definition(),
    ]
}

fn model_switch_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        model_switch_plan_tool_definition(),
        model_switch_apply_tool_definition(),
    ]
}

fn codex_auth_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        codex_auth_status_tool_definition(),
        codex_tool_probe_tool_definition(),
        codex_login_start_tool_definition(),
        codex_login_poll_tool_definition(),
    ]
}

fn secret_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        secret_read_tool_definition(),
        secret_write_tool_definition(),
    ]
}

fn setup_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        web_credentials_update_tool_definition(),
        config_setup_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn config_read_tool_definition() -> ToolDefinition {
    tool_definition(
        "config_read",
        "Lit une valeur de configuration système en suivant un chemin pointé dans config.toml. Utiliser pour consulter les paramètres actifs (canaux, modèles, scheduler, budgets) avant de les modifier ou pour construire de la logique conditionnelle basée sur la config. Ne pas utiliser pour des secrets stockés dans les variables d'environnement. Retourne la valeur courante sous forme de chaîne.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin pointé vers la clé (ex: 'channels.telegram.default_chat_id', 'default_model.provider', 'scheduler.enabled')" }
            },
            "required": ["path"]
        }),
    )
}

fn config_write_tool_definition() -> ToolDefinition {
    tool_definition(
        "config_write",
        "Écrit et persiste une valeur de configuration système dans config.toml avec effet immédiat. La clé est validée contre le schéma runtime AU MOMENT de l'écriture : écris directement, sans pré-vérifier via config_schema/config_read/captain_docs — si la clé est inconnue, l'erreur liste les clés valides de la section. Workflow attendu : une écriture ciblée, une vérification finale, stop. Ne pas utiliser pour des secrets (clés API) — ils sont gérés par variables d'environnement. Retourne une confirmation avec le chemin modifié et la nouvelle valeur.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin pointe (ex: 'channels.telegram.default_chat_id')" },
                "value": { "type": "string", "description": "La valeur a definir" },
                "force": { "type": "boolean", "description": "Écrire même si la clé est absente du schéma (champs optionnels documentés uniquement)" }
            },
            "required": ["path", "value"]
        }),
    )
}

fn config_schema_tool_definition() -> ToolDefinition {
    tool_definition(
        "config_schema",
        "Retourne le template complet de config.toml avec toutes les cles configurables et leur valeur par defaut. Utile avant d'utiliser config_read/config_write pour savoir ce qui existe. Retourne du TOML brut, directement lisible ou parseable.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn self_configure_tool_definition() -> ToolDefinition {
    tool_definition(
        "self_configure",
        "Modifie ta propre configuration d'agent (chaîne de fallbacks, description, system prompt). Le modèle configuré reste autoritaire pour chaque tour. Pour changer de modèle ou de provider, ne pas utiliser ce chemin directement: appeler model_switch_plan puis model_switch_apply afin de choisir explicitement new_session ou compact_session et éviter de casser l'historique de tool calls. Pour une spécialisation ponctuelle, créer un sous-agent explicitement. Les autres changements sont persistés et prennent effet immédiatement sans redémarrage.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": { "type": "string", "description": "Nouveau modele LLM (ex: 'xiaomi/mimo-v2-pro')" },
                "provider": { "type": "string", "description": "Nouveau provider (ex: 'openrouter', 'gemini')" },
                "session_strategy": { "type": "string", "enum": ["new_session", "compact_session"], "description": "Obligatoire si model/provider est fourni. Utiliser seulement apres model_switch_plan et accord utilisateur." },
                "description": { "type": "string", "description": "Nouvelle description de l'agent" },
                "system_prompt": { "type": "string", "description": "Nouveau system prompt" },
                "fallback_models": {
                    "type": "array",
                    "description": "Chaine de fallback (provider/modele differents pour la continuite)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "provider": { "type": "string" },
                            "model": { "type": "string" }
                        },
                        "required": ["provider", "model"]
                    }
                }
            }
        }),
    )
}

fn model_switch_plan_tool_definition() -> ToolDefinition {
    tool_definition(
        "model_switch_plan",
        "[SAFE MODEL SWITCH - READ ONLY] Prépare un changement de modèle/provider pour l'agent courant sans rien modifier. Vérifie provider, modèle, auth, capacités tools/vision/streaming, session active et risque de migration Claude/Codex/OpenAI. À appeler AVANT tout changement de default model ou self_configure model/provider. Si session_strategy_required=true, utilise ask_user avec options \"Nouvelle session\" et \"Résumé compact\". Quand l'utilisateur répond (ex: \"Nouvelle\"), appelle directement model_switch_apply avec session_strategy=new_session ou compact_session; ne lance pas shell_exec/captain CLI/config_write pour ce flux.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": { "type": "string", "description": "Modèle cible, avec ou sans prefix provider (ex: 'codex/gpt-5.4', 'claude-sonnet-4-20250514')" },
                "provider": { "type": "string", "description": "Provider cible explicite si ambigu (ex: 'codex', 'anthropic', 'openai')" }
            },
            "required": ["model"]
        }),
    )
}

fn model_switch_apply_tool_definition() -> ToolDefinition {
    tool_definition(
        "model_switch_apply",
        "[SAFE MODEL SWITCH - CRITICAL MUTATION] Applique un changement de modèle/provider après model_switch_plan et choix utilisateur. Pour Captain, l'agent principal, ce changement persiste aussi le [default_model] global dans config.toml; pour un agent spécialisé, il ne modifie que cet agent. Nécessite session_strategy: new_session démarre une session vide; compact_session convertit le contexte actif en résumé provider-neutral puis démarre une nouvelle session. Ne transporte jamais l'historique tool-call brut entre Claude/Codex/OpenAI. Si le message utilisateur précédent est un choix clair (\"Nouvelle\", \"Résumé compact\"), appelle ce tool immédiatement. Refuse si le provider n'est pas prêt ou si le modèle cible ne supporte pas les tools.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": { "type": "string", "description": "Modèle cible validé par model_switch_plan" },
                "provider": { "type": "string", "description": "Provider cible explicite si nécessaire" },
                "session_strategy": { "type": "string", "enum": ["new_session", "compact_session"], "description": "Choix explicite utilisateur: new_session ou compact_session" }
            },
            "required": ["model", "session_strategy"]
        }),
    )
}

fn codex_auth_status_tool_definition() -> ToolDefinition {
    tool_definition(
        "codex_auth_status",
        "[CODEX OAUTH - READ ONLY] Vérifie si une session ChatGPT/Codex locale est présente et structurellement utilisable par le provider codex. À appeler quand l'utilisateur demande Codex, gpt-5.5, ou quand model_switch_plan signale une auth Codex manquante. Ne lit jamais le token en clair dans la réponse.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn codex_tool_probe_tool_definition() -> ToolDefinition {
    tool_definition(
        "codex_tool_probe",
        "[CODEX OAUTH - READ ONLY, NETWORK] Teste un modèle Codex réel avec un outil factice pour vérifier que le backend émet bien un function/tool call structuré. À utiliser pour diagnostiquer gpt-5.3-codex/gpt-5.4/gpt-5.5 avant d'en faire le modèle agent principal. Ne modifie aucune config et n'exécute aucun outil externe.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Modèle Codex à tester, avec ou sans prefix provider (ex: 'gpt-5.5', 'codex/gpt-5.3-codex'). Défaut: premier modèle Codex disponible, sinon gpt-5.5."
                }
            }
        }),
    )
}

fn codex_login_start_tool_definition() -> ToolDefinition {
    tool_definition(
        "codex_login_start",
        "[CODEX OAUTH - USER ACTION REQUIRED] Démarre un login Codex ChatGPT par device-code directement dans le chat courant. Retourne verification_url, user_code, login_id et interval; l'utilisateur doit ouvrir l'URL et saisir le code. À utiliser au lieu d'abandonner quand Codex n'est pas authentifié.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn codex_login_poll_tool_definition() -> ToolDefinition {
    tool_definition(
        "codex_login_poll",
        "[CODEX OAUTH - POLL/APPLY] Vérifie un login Codex démarré par codex_login_start. Si l'utilisateur a validé le code, échange les tokens et les persiste dans ~/.codex/auth.json. Peut ensuite appliquer le switch modèle via model_switch_apply si apply_model_switch=true, mais seulement après accord explicite utilisateur sur model et session_strategy.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "login_id": { "type": "string", "description": "ID retourné par codex_login_start" },
                "apply_model_switch": { "type": "boolean", "description": " true uniquement si l'utilisateur a demandé explicitement de basculer après login" },
                "model": { "type": "string", "description": "Modèle à appliquer si apply_model_switch=true (ex: codex/gpt-5.5)" },
                "provider": { "type": "string", "description": "Provider cible, normalement codex" },
                "session_strategy": { "type": "string", "enum": ["new_session", "compact_session"], "description": "Obligatoire si apply_model_switch=true" }
            },
            "required": ["login_id"]
        }),
    )
}

fn secret_read_tool_definition() -> ToolDefinition {
    tool_definition(
        "secret_read",
        "[CREDENTIALS] Lit un secret ou credential depuis le store centralisé (~/.captain/secrets.env). À utiliser SPONTANÉMENT dès que tu as besoin d'une clé API, token ou mot de passe pour une intégration tierce — n'invente jamais de credential et NE DEMANDE PAS à l'utilisateur si la clé pourrait déjà être dans le vault. Cas typiques : (1) avant un appel HTTP nécessitant un Bearer/API key (Anthropic, OpenAI, Mistral...), (2) pour vérifier si un token Telegram/Discord ou le mot de passe Email est déjà configuré, (3) pour récupérer un mot de passe d'application avant un login automatisé. Chaque accès est audité dans les logs de sécurité. Ne pas utiliser pour des valeurs de configuration standard — utiliser config_read à la place. Retourne la valeur du secret ou une erreur si la clé n'existe pas.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Nom de la cle secrete (ex: 'TELEGRAM_BOT_TOKEN', 'MISTRAL_API_KEY')" }
            },
            "required": ["key"]
        }),
    )
}

fn secret_write_tool_definition() -> ToolDefinition {
    tool_definition(
        "secret_write",
        "Stocke un secret ou credential dans le store centralisé (~/.captain/secrets.env) avec persistance immédiate. Utiliser pour enregistrer des clés API, tokens OAuth ou mots de passe fournis par l'utilisateur. Chaque écriture est auditée dans les logs de sécurité. Ne pas stocker de données non-sensibles ici — utiliser config_write pour la configuration standard. Retourne une confirmation d'écriture.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Nom de la cle secrete (ex: 'OPENAI_API_KEY')" },
                "value": { "type": "string", "description": "La valeur secrete" }
            },
            "required": ["key", "value"]
        }),
    )
}

fn web_credentials_update_tool_definition() -> ToolDefinition {
    tool_definition(
        "web_credentials_update",
        "[AUTH WEB - MUTATION SÉCURISÉE] Modifie les identifiants de connexion du terminal web à partir d'une demande en langage naturel. Met à jour [auth] dans config.toml, active auth.enabled, hash le mot de passe en SHA256, écrit un backup, puis déclenche le hot-reload config. Utiliser quand l'utilisateur dit \"change mon login web\", \"mets le mot de passe du terminal à ...\", \"génère un nouveau mot de passe web\" ou \"renomme l'utilisateur admin\". Préférer generate_password=true si l'utilisateur demande un nouveau mot de passe sans en fournir. Ne jamais mémoriser le mot de passe généré.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "username": {
                    "type": "string",
                    "description": "Nouveau username web optionnel. ASCII letters/digits/._- uniquement."
                },
                "password": {
                    "type": "string",
                    "description": "Nouveau mot de passe web optionnel. Secret: ne jamais le répéter dans la réponse si fourni par l'utilisateur."
                },
                "generate_password": {
                    "type": "boolean",
                    "description": "true pour générer un mot de passe fort si l'utilisateur n'en a pas fourni."
                },
                "session_ttl_hours": {
                    "type": "integer",
                    "description": "Durée des sessions web en heures, entre 1 et 8760."
                }
            }
        }),
    )
}

fn config_setup_tool_definition() -> ToolDefinition {
    tool_definition(
        "config_setup",
        "[AUTO-INSTALL ATOMIQUE] Installe et configure une intégration complète (Telegram, ElevenLabs, …) en UNE SEULE action : valide les credentials, sauvegarde config.toml (.bak.<ts>), chiffre les secrets dans le vault, patche les sections TOML en préservant les commentaires, et exécute optionnellement un test live pour valider le bon fonctionnement. Préférer cet outil à secret_write+config_write enchaînés quand l'utilisateur veut activer une intégration entière (ex: « active Telegram avec ce bot_token »). Tout échec rollback la config depuis le backup. Intégrations disponibles : 'telegram', 'tts_elevenlabs'. Retourne un récapitulatif JSON (vault_keys écrites, sections patchées, backup_path, test_message).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "integration": {
                    "type": "string",
                    "description": "Nom canonique de l'intégration à installer",
                    "enum": ["telegram", "tts_elevenlabs", "tts_openai", "stt_whisper"]
                },
                "credentials": {
                    "type": "object",
                    "description": "Objet JSON dont les champs dépendent de l'intégration. telegram: {bot_token, default_chat_id, allowed_users?[]}. tts_elevenlabs: {api_key, voice_id?, model_id?} (model? accepté par compatibilité). tts_openai: {api_key, voice?(alloy|echo|fable|onyx|nova|shimmer), model?(tts-1|tts-1-hd), format?(mp3|wav|opus|flac)}. stt_whisper: {api_key, provider?(groq|openai)}."
                },
                "run_test": {
                    "type": "boolean",
                    "description": "Si true, exécute un appel live (getMe pour Telegram, /v1/voices pour ElevenLabs) pour valider les credentials end-to-end. Défaut: false."
                }
            },
            "required": ["integration", "credentials"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_tool_definitions_keep_public_order() {
        let tools = config_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "config_read",
                "config_write",
                "config_schema",
                "self_configure",
                "model_switch_plan",
                "model_switch_apply",
                "codex_auth_status",
                "codex_tool_probe",
                "codex_login_start",
                "codex_login_poll",
                "secret_read",
                "secret_write",
                "web_credentials_update",
                "config_setup",
            ]
        );
    }

    #[test]
    fn config_tool_definitions_keep_model_switch_contracts() {
        let tools = config_tool_definitions();

        assert_eq!(
            required_fields(tool(&tools, "model_switch_plan")),
            vec!["model"]
        );
        assert_eq!(
            required_fields(tool(&tools, "model_switch_apply")),
            vec!["model", "session_strategy"]
        );
        assert_eq!(
            enum_values(&property(tool(&tools, "model_switch_apply"), "session_strategy")["enum"]),
            vec!["new_session", "compact_session"]
        );
    }

    #[test]
    fn config_tool_definitions_keep_secret_and_setup_contracts() {
        let tools = config_tool_definitions();
        let secret_read = tool(&tools, "secret_read");
        let config_setup = tool(&tools, "config_setup");

        assert!(!secret_read.description.contains("Slack"));
        assert!(!secret_read.description.contains("WhatsApp"));
        assert!(secret_read.description.contains("Telegram/Discord"));
        assert!(secret_read.description.contains("Email"));
        assert_eq!(required_fields(secret_read), vec!["key"]);
        assert_eq!(
            enum_values(&property(config_setup, "integration")["enum"]),
            vec!["telegram", "tts_elevenlabs", "tts_openai", "stt_whisper"]
        );
        assert_eq!(
            required_fields(config_setup),
            vec!["integration", "credentials"]
        );
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("missing config tool {name}"))
    }

    fn required_fields(definition: &ToolDefinition) -> Vec<&str> {
        definition
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn property<'a>(definition: &'a ToolDefinition, name: &str) -> &'a Value {
        definition
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("missing property {name} on {}", definition.name))
    }

    fn enum_values(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }
}
