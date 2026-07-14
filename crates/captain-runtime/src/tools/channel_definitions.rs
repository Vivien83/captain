//! Static channel delivery tool definitions.

use crate::tools::channel_policy::ACTIVE_CHANNELS;
use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn channel_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = channel_delivery_tool_definitions();
    definitions.extend(channel_management_tool_definitions());
    definitions.extend(telegram_topic_tool_definitions());
    definitions
}

fn channel_delivery_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        channel_delivery_batch_tool_definition(),
        channel_send_tool_definition(),
    ]
}

fn channel_management_tool_definitions() -> Vec<ToolDefinition> {
    vec![channel_reconfigure_tool_definition()]
}

fn telegram_topic_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        telegram_set_topic_tool_definition(),
        telegram_get_topic_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn channel_delivery_batch_tool_definition() -> ToolDefinition {
    tool_definition(
        "channel_delivery_batch",
        "[LIVRAISON GROUPEE] Envoie plusieurs messages/fichiers médias via channel_send en un seul appel contrôlé. Effets de bord explicites: chaque delivery est un input channel_send complet. À utiliser pour envoyer texte + document + audio/image sans séquence d'appels séparés.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "deliveries": {
                    "type": "array",
                    "maxItems": 10,
                    "items": { "type": "object", "description": "Input channel_send complet: channel, recipient, message, file_path/image_url/file_url, etc." }
                },
                "stop_on_error": { "type": "boolean", "default": true }
            },
            "required": ["deliveries"]
        }),
    )
}

fn channel_send_tool_definition() -> ToolDefinition {
    tool_definition(
        "channel_send",
        "Envoie un message proactif, un média ou du contenu interactif via un canal actif: Telegram, Discord, Signal ou Email. Les autres channels sont gelés jusqu'à ce que le coeur soit Hermes-level. Utiliser pour notifier d'un événement, envoyer un rapport, ou proposer des choix via boutons inline Telegram. Ne pas utiliser pour répondre dans le flux normal de conversation ni pour retransmettre un secret brut; cite seulement le nom de clé vault ou une valeur masquée. Le markdown est automatiquement converti en HTML Telegram. Les file_path audio sont envoyés comme audio natif Telegram, pas comme document générique. Retourne un statut de livraison.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "enum": ACTIVE_CHANNELS, "description": "Canal actif configuré dans Captain: 'telegram', 'discord', 'signal' ou 'email'." },
                "recipient": { "type": "string", "description": "Identifiant du destinataire spécifique à la plateforme (chat_id Telegram, channel/user Discord, numéro Signal, adresse email...). Optionnel si un destinataire par défaut est configuré pour ce canal." },
                "message": { "type": "string", "description": "Corps du message. Markdown supporté (**gras**, *italique*, `code`, ### titres, - listes). Automatiquement converti en HTML Telegram." },
                "subject": { "type": "string", "description": "Sujet du message (email uniquement)" },
                "buttons": {
                    "type": "array",
                    "description": "Inline keyboard buttons (Telegram). Each element is a row. Simple: [\"Option A\", \"Option B\"]. Rich: [{\"text\": \"Visit\", \"url\": \"https://...\"}].",
                    "items": {
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "oneOf": [{ "type": "string" }, { "type": "object" }] } }
                        ]
                    }
                },
                "topic_id": { "type": "string", "description": "ID du topic/thread dans un groupe Telegram avec forums activés (message_thread_id). Route le message vers un topic spécifique." },
                "image_url": { "type": "string", "description": "URL publique d'une image à envoyer en tant que photo" },
                "file_url": { "type": "string", "description": "URL publique d'un fichier à envoyer en pièce jointe" },
                "file_path": { "type": "string", "description": "Chemin local vers un fichier à envoyer en pièce jointe. Sur Telegram, les fichiers audio mp3/wav/ogg générés par text_to_speech sont envoyés comme audio/voice natif." },
                "filename": { "type": "string", "description": "Nom du fichier affiché pour les pièces jointes" },
                "thread_id": { "type": "string", "description": "Alias de topic_id pour compatibilité" }
            },
            "required": ["channel"]
        }),
    )
}

fn channel_reconfigure_tool_definition() -> ToolDefinition {
    tool_definition(
        "channel_reconfigure",
        "[CANAUX] Demande à la bridge de canaux de (re)démarrer UN adapter actif (Telegram, Discord, Signal ou Email) à partir de la config + secrets.env live. Les autres channels sont gelés et doivent être ignorés pour l'instant. Couvre DEUX cas : (1) ROTATION d'un canal déjà actif après que tu viens d'écrire un nouveau token / une nouvelle config, et (2) BOOTSTRAP d'un canal actif jamais activé — section `[channels.<name>]` ajoutée à config.toml + token posé via `secret_write` — l'adapter passe alors de CONFIGURED à ACTIVE sans redémarrer le daemon (le hot-reload re-lit `secrets.env` à chaque appel). À utiliser SPONTANÉMENT — sans qu'on te le demande — quand l'utilisateur dit 'change le bot Telegram', 'mets le nouveau token X dans Y', 'configure Discord et envoie un test'. Le `channel` doit correspondre à une section active `[channels.<name>]` du config.toml live (sinon retour d'erreur listant les noms valides ou indiquant que le channel est gelé). L'adapter ciblé est arrêté + ré-instancié avec la config fraîche, les autres canaux restent connectés. WORKFLOW BRAND-NEW : `secret_write` du token → `config_setup`/`config_write` de la section → `channel_reconfigure({channel})` → `channel_send` direct. EXEMPLE : après avoir posé `DISCORD_BOT_TOKEN` via secret_write et écrit `[channels.discord]` dans config.toml, appelle channel_reconfigure({\"channel\":\"discord\"}) et `channel_send` marche directement.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "enum": ACTIVE_CHANNELS, "description": "Canal actif à recharger: 'telegram', 'discord', 'signal' ou 'email'." }
            },
            "required": ["channel"]
        }),
    )
}

fn telegram_set_topic_tool_definition() -> ToolDefinition {
    tool_definition(
        "telegram_set_topic",
        "Associe un agent ou une Hand à un topic de forum Telegram. Une fois configuré, tous les messages de cet agent sont automatiquement routés vers ce topic. Persisté entre les redémarrages. Utiliser pour organiser les canaux de communication par agent dans un groupe Telegram avec forums activés. Retourne une confirmation.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_name": { "type": "string", "description": "Agent or hand name (e.g. 'OpsHand', 'assistant')" },
                "topic_id": { "type": "string", "description": "Telegram message_thread_id of the forum topic" }
            },
            "required": ["agent_name", "topic_id"]
        }),
    )
}

fn telegram_get_topic_tool_definition() -> ToolDefinition {
    tool_definition(
        "telegram_get_topic",
        "Vérifie quel topic de forum Telegram est associé à un agent donné. Utiliser avant telegram_set_topic pour vérifier si une association existe déjà. Retourne le topic_id associé ou null si aucun topic n'est configuré.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_name": { "type": "string", "description": "Agent or hand name" }
            },
            "required": ["agent_name"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_tool_definitions_keep_public_order() {
        let tools = channel_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "channel_delivery_batch",
                "channel_send",
                "channel_reconfigure",
                "telegram_set_topic",
                "telegram_get_topic",
            ]
        );
    }

    #[test]
    fn channel_tool_definitions_keep_delivery_contracts_active_only_and_bounded() {
        let tools = channel_tool_definitions();
        let batch = tool(&tools, "channel_delivery_batch");
        let send = tool(&tools, "channel_send");

        assert_eq!(required_fields(batch), vec!["deliveries"]);
        assert_eq!(
            integer_field(property(batch, "deliveries"), "maxItems"),
            Some(10)
        );
        assert_eq!(
            boolean_field(property(batch, "stop_on_error"), "default"),
            Some(true)
        );

        assert_eq!(required_fields(send), vec!["channel"]);
        assert_eq!(
            property(send, "channel")["enum"],
            serde_json::json!(["telegram", "discord", "signal", "email"])
        );
        assert_contains(&send.description, "Les autres channels sont gelés");
        assert_contains(&send.description, "audio natif Telegram");
        assert_not_contains(&send.description, "Slack");
        assert_not_contains(&send.description, "WhatsApp");
    }

    #[test]
    fn channel_tool_definitions_keep_reconfigure_schema_active_only() {
        let tools = channel_tool_definitions();
        let reconfigure = tool(&tools, "channel_reconfigure");

        assert_eq!(required_fields(reconfigure), vec!["channel"]);
        assert_eq!(
            property(reconfigure, "channel")["enum"],
            serde_json::json!(["telegram", "discord", "signal", "email"])
        );
        assert_contains(&reconfigure.description, "SPONTANÉMENT");
        assert_contains(&reconfigure.description, "WORKFLOW BRAND-NEW");
        assert_contains(&reconfigure.description, "Les autres channels sont gelés");
        assert_not_contains(&reconfigure.description, "Slack");
        assert_not_contains(&reconfigure.description, "WhatsApp");
    }

    #[test]
    fn channel_tool_definitions_keep_telegram_topic_contracts() {
        let tools = channel_tool_definitions();
        let set_topic = tool(&tools, "telegram_set_topic");
        let get_topic = tool(&tools, "telegram_get_topic");

        assert_eq!(required_fields(set_topic), vec!["agent_name", "topic_id"]);
        assert_contains(&set_topic.description, "forum Telegram");
        assert_eq!(required_fields(get_topic), vec!["agent_name"]);
        assert_contains(&get_topic.description, "telegram_set_topic");
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered"))
    }

    fn required_fields(tool: &ToolDefinition) -> Vec<String> {
        tool.input_schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn property<'a>(tool: &'a ToolDefinition, name: &str) -> &'a Value {
        tool.input_schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("{} should define property {name}", tool.name))
    }

    fn integer_field(value: &Value, name: &str) -> Option<u64> {
        value.get(name).and_then(Value::as_u64)
    }

    fn boolean_field(value: &Value, name: &str) -> Option<bool> {
        value.get(name).and_then(Value::as_bool)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }

    fn assert_not_contains(haystack: &str, needle: &str) {
        assert!(
            !haystack.contains(needle),
            "expected `{haystack}` not to contain `{needle}`"
        );
    }
}
