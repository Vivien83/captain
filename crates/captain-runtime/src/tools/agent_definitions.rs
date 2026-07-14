//! Static local-agent coordination tool definitions.

use captain_types::agent::AGENT_MANIFEST_CANONICAL_EXAMPLE;
use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn agent_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = agent_lifecycle_tool_definitions();
    definitions.extend(agent_observability_tool_definitions());
    definitions.extend(agent_guidance_tool_definitions());
    definitions
}

fn agent_lifecycle_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        agent_send_tool_definition(),
        agent_spawn_tool_definition(),
        agent_list_tool_definition(),
        agent_kill_tool_definition(),
    ]
}

fn agent_observability_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        agent_status_tool_definition(),
        agent_caps_tool_definition(),
        agent_watch_tool_definition(),
    ]
}

fn agent_guidance_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        agent_delegate_tool_definition(),
        agent_correct_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn agent_send_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_send",
        "Envoie un message à un autre agent en cours d'exécution et attend sa réponse de manière synchrone. Utiliser pour déléguer une sous-tâche à un agent spécialisé ou orchestrer une collaboration inter-agents depuis Captain. Ne pas utiliser si l'agent cible est inconnu — utiliser agent_list d'abord pour découvrir les agents disponibles. Pour un client HTTP externe, ne pas inventer un bridge: lire le manifeste `/api/agents/{id}/api/manifest`, générer le bearer avec `/api/agents/{id}/api/token/rotate`, puis appeler l'ingress dédié `POST /hooks/agents/{id}/ingress`. Retourne la réponse textuelle de l'agent cible.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID ou nom exact de l'agent cible (utiliser agent_list pour obtenir les identifiants disponibles)" },
                "message": { "type": "string", "description": "Message à envoyer à l'agent — formuler comme une instruction claire ou une question précise" }
            },
            "required": ["agent_id", "message"]
        }),
    )
}

fn agent_spawn_tool_definition() -> ToolDefinition {
    let description = format!(
        "Instancie un nouvel agent à partir d'un manifest TOML et le démarre immédiatement dans le runtime. Utiliser seulement pour une sous-tâche indépendante, longue ou parallélisable qui justifie son propre contexte (scraper, analyste, worker). Ne pas utiliser pour une action locale simple ni si un agent approprié existe déjà — vérifier avec agent_list d'abord. Le manifest doit déclarer explicitement les outils autorisés avec tool_allowlist ou capabilities.tools; un profil seul ou un accès \"*\" est refusé pour un sous-agent. Captain ajoute les outils de découverte minimaux (capability_search, skill_search, skill_view, tool_search, captain_docs, system_time). Si un outil manque, le sous-agent doit demander l'extension à Captain au lieu de contourner la limite. Si l'agent parent est limité, l'enfant doit rester dans ce même périmètre: aucun spawn ne peut élargir les outils du parent. Format critique: `model` est une table `[model]`, pas une chaîne; ne pas utiliser `[tools] allow = [...]` pour l'allowlist, utiliser `tool_allowlist = [...]`. Protocole création: après spawn, Captain provisionne automatiquement l'ingress agent-as-service, retourne la fiche API complète et le bearer token une seule fois. Pour un vrai in/out prêt à l'emploi, renseigner `agent_api.egress_callback_url` pendant le spawn; sinon annoncer l'état `ingress_ready` et l'action egress/configure restante sans prétendre que tout est ready. Pour un agent jetable (démo, test ponctuel, sous-tâche sans besoin de persister), ajouter `tags = [\"ephemeral\"]` dans le manifest: Captain le termine automatiquement après 30 minutes d'inactivité au lieu de le laisser tourner indéfiniment — ne pas ajouter ce tag pour un agent censé durer (hand, agent-as-service, veille). Exemple valide:\n```toml\n{AGENT_MANIFEST_CANONICAL_EXAMPLE}```\nRetourne l'UUID, le nom et le protocole API de l'agent créé."
    );
    tool_definition(
        "agent_spawn",
        &description,
        serde_json::json!({
            "type": "object",
            "properties": {
                "manifest_toml": {
                    "type": "string",
                    "description": "Manifest TOML complet de l'agent. Minimum recommandé: name, description, module = \"builtin:chat\", tool_allowlist = [\"...\"], puis [model] provider/model/system_prompt. Ne pas écrire model = \"...\" ni [tools] allow = [...]."
                },
                "agent_api": {
                    "type": "object",
                    "description": "Provisioning agent-as-service à la création. Par défaut Captain génère le bearer ingress. Fournir egress_callback_url pour rendre l'in/out callback prêt immédiatement.",
                    "properties": {
                        "provision_ingress_token": { "type": "boolean", "description": "Générer et stocker immédiatement le bearer ingress. Défaut: true." },
                        "egress_callback_url": { "type": "string", "description": "URL HTTPS publique qui recevra les callbacks signés agent_api.completed/failed/test." },
                        "egress_callback_secret": { "type": "string", "description": "Secret HMAC fourni par le service externe. Si absent et generate_callback_secret=true, Captain en génère un et le retourne une seule fois." },
                        "generate_callback_secret": { "type": "boolean", "description": "Générer un secret callback si egress_callback_url est fourni sans secret. Défaut: true." }
                    }
                }
            },
            "required": ["manifest_toml"]
        }),
    )
}

fn agent_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_list",
        "Liste tous les agents actuellement enregistrés dans le runtime avec leurs métadonnées. Utiliser systématiquement avant agent_send, agent_kill ou une intégration API externe pour obtenir les UUIDs valides, ou pour vérifier l'état d'un agent spawné. Pour exposer un agent comme service, prendre l'id retourné ici puis inspecter `/api/agents/{id}/api/manifest`; l'ingress externe est `POST /hooks/agents/{id}/ingress`. Ne prend aucun paramètre. Retourne un tableau d'objets avec id, name, state (running/idle/stopped) et model pour chaque agent.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn agent_kill_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_kill",
        "Termine définitivement un agent par son UUID, libérant ses ressources mémoire et ses connexions. Utiliser pour nettoyer des agents temporaires après leur mission ou arrêter un agent défaillant. Ne pas utiliser sur des agents système critiques ou l'agent courant lui-même. Retourne une confirmation de terminaison avec l'ID et le nom de l'agent arrêté.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID de l'agent à terminer (obtenir via agent_list — ne pas utiliser un nom, uniquement l'UUID)" }
            },
            "required": ["agent_id"]
        }),
    )
}

fn agent_status_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_status",
        "Vue détaillée d'un agent : état, modèle, tokens consommés, nombre de messages en session, dernière activité. Utiliser pour surveiller les sub-agents avant de décider s'il faut les corriger ou les arrêter.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID de l'agent à inspecter" }
            },
            "required": ["agent_id"]
        }),
    )
}

fn agent_caps_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_caps",
        "Rapport complet des capacités et du budget d'un agent (toi-même ou un autre) : outils déclarés et effectifs, network/shell/memory scopes, quotas de ressources, coût heure/jour/mois et tokens/heure consommés vs limite. Équivalent agent-facing de la commande CLI `captain agent caps` — utiliser CET outil plutôt que shell_exec pour inspecter des capacités/budget, le binaire `captain` n'est pas accessible depuis le sandbox shell_exec.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID de l'agent à inspecter (utiliser agent_list pour le tien ou celui d'un autre agent)" }
            },
            "required": ["agent_id"]
        }),
    )
}

fn agent_watch_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_watch",
        "Derniers événements d'un agent (tool calls, phases, erreurs). Utiliser pour comprendre ce qu'un sub-agent a fait récemment et détecter des problèmes.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID de l'agent à surveiller" },
                "limit": { "type": "integer", "description": "Nombre d'événements à retourner (défaut: 10)", "default": 10 }
            },
            "required": ["agent_id"]
        }),
    )
}

fn agent_delegate_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_delegate",
        "Délègue une tâche à un sub-agent avec un budget de tokens maximum. Actuellement synchrone: poste une tâche, exécute un tour du worker, complète la tâche avec le résultat et retourne ce résultat. Pour du vrai fire-and-forget, utiliser task_post/task_claim. À utiliser seulement si la tâche est indépendante, vérifiable et moins coûteuse en contexte que de la faire dans l'agent courant. Toujours fixer max_tokens à un budget serré et demander un livrable précis. Éviter pour une action locale simple, car la délégation peut consommer plus de tokens qu'elle n'en économise. Dimensionner selon la nature de la tâche: une extraction/vérification factuelle courte tient dans 800-1500 tokens, mais un rôle de critique/QA ouvert (lister risques, angles morts, améliorations) a besoin de 3000-5000 tokens pour produire un livrable complet — un budget trop serré coupe le worker en plein raisonnement (\"Budget de delegation atteint\") avant qu'il ait fini.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID de l'agent à qui déléguer" },
                "task": { "type": "string", "description": "Description de la tâche à accomplir" },
                "max_tokens": { "type": "integer", "description": "Budget maximum en tokens (0 = illimité)", "default": 5000 }
            },
            "required": ["agent_id", "task"]
        }),
    )
}

fn agent_correct_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_correct",
        "Injecte une correction système dans un sub-agent. Le message apparaîtra comme une instruction système dans sa prochaine interaction. Utiliser pour corriger un agent qui se trompe sans le tuer.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "UUID de l'agent à corriger" },
                "message": { "type": "string", "description": "Message de correction (ex: 'Utilise l'outil X au lieu de Y', 'Le format attendu est JSON')" }
            },
            "required": ["agent_id", "message"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tool_definitions_keep_public_order() {
        let tools = agent_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "agent_send",
                "agent_spawn",
                "agent_list",
                "agent_kill",
                "agent_status",
                "agent_caps",
                "agent_watch",
                "agent_delegate",
                "agent_correct",
            ]
        );
    }

    #[test]
    fn agent_lifecycle_tools_keep_scope_contracts() {
        let tools = agent_tool_definitions();
        let send = tool(&tools, "agent_send");
        let spawn = tool(&tools, "agent_spawn");
        let list = tool(&tools, "agent_list");
        let kill = tool(&tools, "agent_kill");

        assert_eq!(required_fields(send), vec!["agent_id", "message"]);
        assert_contains(&send.description, "agent_list d'abord");
        assert_contains(&send.description, "/api/agents/{id}/api/manifest");
        assert_contains(&send.description, "/api/agents/{id}/api/token/rotate");
        assert_contains(&send.description, "POST /hooks/agents/{id}/ingress");

        assert_eq!(required_fields(spawn), vec!["manifest_toml"]);
        assert_contains(&spawn.description, "tool_allowlist");
        assert_contains(&spawn.description, "capabilities.tools");
        assert_contains(&spawn.description, "skill_view");
        assert_contains(&spawn.description, "aucun spawn ne peut élargir");
        assert_contains(&spawn.description, "`model` est une table `[model]`");
        assert_contains(&spawn.description, "tool_allowlist = [");
        assert_contains(&spawn.description, "provider = \"codex\"");
        assert_contains(&spawn.description, "bearer token une seule fois");
        assert_contains(&spawn.description, "egress_callback_url");
        assert_contains(&spawn.description, "ingress_ready");
        assert_contains(&spawn.description, "tags = [\"ephemeral\"]");
        assert_contains(&spawn.description, "30 minutes d'inactivité");

        assert!(required_fields(list).is_empty());
        assert_contains(&list.description, "/api/agents/{id}/api/manifest");
        assert_contains(&list.description, "POST /hooks/agents/{id}/ingress");
        assert_eq!(
            list.input_schema
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| properties.len()),
            Some(0)
        );

        assert_eq!(required_fields(kill), vec!["agent_id"]);
        assert_contains(
            property(kill, "agent_id")["description"]
                .as_str()
                .unwrap_or_default(),
            "UUID",
        );
        assert_contains(
            property(kill, "agent_id")["description"]
                .as_str()
                .unwrap_or_default(),
            "ne pas utiliser un nom",
        );
    }

    #[test]
    fn agent_observability_tools_keep_required_agent_and_limits() {
        let tools = agent_tool_definitions();
        let status = tool(&tools, "agent_status");
        let watch = tool(&tools, "agent_watch");

        assert_eq!(required_fields(status), vec!["agent_id"]);
        assert_contains(&status.description, "tokens consommés");

        assert_eq!(required_fields(watch), vec!["agent_id"]);
        assert_eq!(integer_field(property(watch, "limit"), "default"), Some(10));
        assert_contains(&watch.description, "tool calls");
    }

    #[test]
    fn agent_guidance_tools_keep_budget_and_correction_contracts() {
        let tools = agent_tool_definitions();
        let delegate = tool(&tools, "agent_delegate");
        let correct = tool(&tools, "agent_correct");

        assert_eq!(required_fields(delegate), vec!["agent_id", "task"]);
        assert_eq!(
            integer_field(property(delegate, "max_tokens"), "default"),
            Some(5000)
        );
        assert_contains(&delegate.description, "budget serré");
        assert_contains(&delegate.description, "moins coûteuse");
        assert_contains(&delegate.description, "3000-5000 tokens");
        assert_contains(&delegate.description, "Budget de delegation atteint");

        assert_eq!(required_fields(correct), vec!["agent_id", "message"]);
        assert_contains(&correct.description, "instruction système");
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

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
