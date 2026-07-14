//! Static browser automation tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn browser_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = browser_core_tool_definitions();
    definitions.extend(browser_input_tool_definitions());
    definitions.extend(browser_read_tool_definitions());
    definitions.extend(browser_navigation_tool_definitions());
    definitions.extend(browser_observability_tool_definitions());
    definitions
}

fn browser_core_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        browser_batch_tool_definition(),
        browser_navigate_tool_definition(),
    ]
}

fn browser_input_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        browser_click_tool_definition(),
        browser_type_tool_definition(),
        browser_keys_tool_definition(),
        browser_select_tool_definition(),
        browser_hover_tool_definition(),
    ]
}

fn browser_read_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        browser_screenshot_tool_definition(),
        browser_read_page_tool_definition(),
        browser_close_tool_definition(),
    ]
}

fn browser_navigation_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        browser_scroll_tool_definition(),
        browser_wait_tool_definition(),
        browser_run_js_tool_definition(),
        browser_back_tool_definition(),
    ]
}

fn browser_observability_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        browser_status_tool_definition(),
        browser_network_log_tool_definition(),
        browser_observe_tool_definition(),
        browser_diagnostics_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn browser_batch_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_batch",
        "[BROWSER GROUPE - TOKEN ECONOMY] Exécute jusqu'à 20 actions navigateur dans un seul appel atomique: navigate, click, type, keys, select, hover, scroll, wait, run_js, read_page, screenshot, observe, status, network_log, diagnostics, back, close. Utiliser par défaut pour éviter les séquences coûteuses navigate→wait→read→diagnostics en plusieurs tours. Retourne des résumés compacts par étape et une observation finale configurable. Pour lire un article complet, mettre final_observation='read_page'; pour interaction UI, garder final_observation='observe' et utiliser les refs @eN.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "description": "Liste d'actions. Chaque item contient action plus les champs requis: navigate.url, click.selector, type.selector/text, keys.keys, select.selector/value, hover.selector, wait.selector/timeout_ms, scroll.direction/amount, run_js.expression.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "navigate, click, type, keys, select, hover, scroll, wait, run_js, read_page, screenshot, observe, status, network_log, diagnostics, back, close" },
                            "url": { "type": "string" },
                            "selector": { "type": "string", "description": "CSS selector, visible text for click/hover, or @eN ref returned by observe." },
                            "text": { "type": "string" },
                            "keys": { "type": "string", "description": "Keyboard input for focused element, e.g. Enter, Tab, Escape, Control+a, Meta+k." },
                            "value": { "type": "string", "description": "Value or visible label for select action." },
                            "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                            "amount": { "type": "integer" },
                            "timeout_ms": { "type": "integer" },
                            "expression": { "type": "string" },
                            "limit": { "type": "integer" },
                            "clear": { "type": "boolean" },
                            "max_elements": { "type": "integer" }
                        },
                        "required": ["action"]
                    }
                },
                "stop_on_error": { "type": "boolean", "description": "Stoppe au premier échec. Défaut true." },
                "include_data": { "type": "boolean", "description": "Inclut les données brutes tronquées par étape. Défaut false pour économiser le contexte." },
                "final_observation": { "type": "string", "enum": ["observe", "read_page", "status", "diagnostics", "none"], "description": "Observation finale après les étapes. Défaut observe." },
                "max_elements": { "type": "integer", "description": "Nombre max d'éléments interactifs dans observe/diagnostics. Défaut 60, max 120." }
            },
            "required": ["steps"]
        }),
    )
}

fn browser_navigate_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_navigate",
        "Ouvre ou navigue vers une URL dans une session navigateur persistante partagée avec les autres tools `browser_*`. Utiliser en premier pour initier toute session de navigation ou changer de page. Ne pas utiliser pour lire la page courante sans changer d'URL — préférer `browser_read_page`. Retourne le titre de la page et son contenu textuel converti en markdown.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL complète à charger (http:// ou https:// uniquement). Doit inclure le protocole." }
            },
            "required": ["url"]
        }),
    )
}

fn browser_click_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_click",
        "Clique sur un élément de la page courante du navigateur par sélecteur CSS ou texte visible. Utiliser après browser_navigate pour interagir avec la page (boutons, liens, menus). Si l'élément n'est pas visible, utiliser browser_scroll d'abord. Retourne l'état de la page après le clic.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector (e.g., '#submit-btn', '.add-to-cart') or visible text to click" }
            },
            "required": ["selector"]
        }),
    )
}

fn browser_type_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_type",
        "Renseigne du texte dans un champ de formulaire sur la page courante du navigateur et déclenche les événements input/change. Utiliser pour remplir des formulaires, des barres de recherche ou des champs de login. Cibler le champ par sélecteur CSS ou ref @eN. Pour valider au clavier après saisie, enchaîner avec browser_keys.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector for the input field (e.g., 'input[name=\"email\"]', '#search-box')" },
                "text": { "type": "string", "description": "The text to type into the field" }
            },
            "required": ["selector", "text"]
        }),
    )
}

fn browser_keys_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_keys",
        "Envoie une touche ou combinaison clavier à l'élément actuellement focus dans le navigateur: Enter, Tab, Escape, Backspace, flèches, Control+a, Meta+k, etc. Utiliser après browser_type ou browser_click quand une UI réagit au clavier. Pour saisir du texte dans un champ connu, préférer browser_type.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "keys": { "type": "string", "description": "Touche ou combinaison clavier, par exemple 'Enter', 'Tab', 'Escape', 'Control+a', 'Meta+k'." }
            },
            "required": ["keys"]
        }),
    )
}

fn browser_select_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_select",
        "Sélectionne une option dans un élément HTML <select> par valeur, label ou texte visible. Utiliser pour les menus déroulants natifs plutôt que browser_run_js. Accepte les refs @eN retournées par browser_observe.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector or @eN ref for the <select> element." },
                "value": { "type": "string", "description": "Option value, label or visible text to select." }
            },
            "required": ["selector", "value"]
        }),
    )
}

fn browser_hover_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_hover",
        "Déplace la souris au-dessus d'un élément pour ouvrir menus, tooltips ou états hover. Utiliser avant browser_observe/click quand un contrôle n'apparaît qu'au survol. Accepte CSS selector, texte visible ou ref @eN.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector, visible text, or @eN ref returned by browser_observe." }
            },
            "required": ["selector"]
        }),
    )
}

fn browser_screenshot_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_screenshot",
        "Capture une capture d'écran de la page courante dans la session navigateur active. Utiliser pour vérifier visuellement l'état d'une page, détecter un CAPTCHA, ou documenter une interface. Ne pas utiliser pour extraire du texte — `browser_read_page` est plus adapté et moins coûteux. Retourne une image PNG encodée en base64.",
        serde_json::json!({"type": "object", "properties": {}}),
    )
}

fn browser_read_page_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_read_page",
        "Lit le contenu de la page courante du navigateur et le retourne sous forme de markdown structuré. Utiliser après un clic, une navigation ou une action dynamique pour récupérer le nouvel état de la page sans la recharger. Ne pas utiliser pour capturer visuellement la page — préférer `browser_screenshot`. Retourne le titre, l'URL courante et le contenu textuel structuré.",
        serde_json::json!({"type": "object", "properties": {}}),
    )
}

fn browser_close_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_close",
        "Ferme la session navigateur active et libère les ressources associées. Utiliser après avoir terminé toutes les opérations de navigation. Le navigateur se ferme aussi automatiquement à la fin de la boucle agent. Ne pas appeler si aucune session n'est ouverte.",
        serde_json::json!({"type": "object", "properties": {}}),
    )
}

fn browser_scroll_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_scroll",
        "Fait défiler la page du navigateur dans une direction donnée. Utiliser pour accéder au contenu sous le fold ou naviguer dans des pages longues. Combiner avec browser_read_page après le scroll pour lire le nouveau contenu visible. Directions supportées : up, down, left, right.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "direction": { "type": "string", "description": "Scroll direction: 'up', 'down', 'left', 'right' (default: 'down')" },
                "amount": { "type": "integer", "description": "Pixels to scroll (default: 600)" }
            }
        }),
    )
}

fn browser_wait_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_wait",
        "Attend qu'un sélecteur CSS apparaisse sur la page courante du navigateur. Utiliser après un clic ou une navigation qui déclenche un chargement asynchrone (AJAX, SPA). Timeout configurable jusqu'à 30 secondes. Ne pas utiliser si l'élément est déjà présent — vérifier d'abord avec browser_read_page. Retourne le contenu de la page une fois l'élément trouvé.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector to wait for" },
                "timeout_ms": { "type": "integer", "description": "Max wait time in milliseconds (default: 5000, max: 30000)" }
            },
            "required": ["selector"]
        }),
    )
}

fn browser_run_js_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_run_js",
        "Exécute du JavaScript arbitraire dans le contexte de la page courante du navigateur. Utiliser pour des interactions avancées impossibles avec les autres outils browser_* (manipulation du DOM, extraction de données structurées, déclenchement d'événements). Ne pas utiliser pour des actions simples couvertes par browser_click ou browser_type. Retourne le résultat de l'expression JavaScript sérialisé en JSON.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "expression": { "type": "string", "description": "JavaScript expression to run in the page context" }
            },
            "required": ["expression"]
        }),
    )
}

fn browser_back_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_back",
        "Revient à la page précédente dans l'historique du navigateur. Utiliser pour annuler une navigation ou revenir à une page de résultats. Équivalent au bouton Retour du navigateur. Retourne le titre et le contenu de la page précédente.",
        serde_json::json!({"type": "object", "properties": {}}),
    )
}

fn browser_status_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_status",
        "Inspecte la session navigateur native de l'agent sans en créer une nouvelle: Chrome disponible, profil isolé, viewport, nombre de sessions actives, page courante si ouverte. Utiliser pour diagnostiquer une navigation, vérifier quel profil est utilisé ou savoir si browser_navigate a déjà ouvert une session.",
        serde_json::json!({"type": "object", "properties": {}}),
    )
}

fn browser_network_log_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_network_log",
        "Retourne les derniers événements réseau CDP de la session navigateur active: requêtes, réponses HTTP, types MIME et échecs de chargement. Utiliser après browser_navigate/click/wait quand une page dynamique échoue, qu'une API backend renvoie une erreur ou qu'il faut auditer les ressources chargées. Ne crée pas de session navigateur.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Nombre maximal d'événements à retourner, défaut 50, max 200." },
                "clear": { "type": "boolean", "description": "Si true, vide le journal réseau après lecture. Défaut false." }
            }
        }),
    )
}

fn browser_observe_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_observe",
        "Observe la page active sans screenshot lourd: titre, URL, viewport, scroll et liste compacte d'éléments interactifs avec refs stables @e1, @e2, etc. Utiliser pour comprendre l'interface puis cliquer/saisir/sélectionner/survoler via browser_click/browser_type/browser_select/browser_hover avec selector='@eN', ou dans browser_batch. Ne crée pas de session si aucune page n'est ouverte.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "max_elements": { "type": "integer", "description": "Nombre max d'éléments interactifs à retourner. Défaut 60, max 120." }
            }
        }),
    )
}

fn browser_diagnostics_tool_definition() -> ToolDefinition {
    tool_definition(
        "browser_diagnostics",
        "Retourne en un seul appel l'état navigateur, l'observation interactive, le journal réseau CDP récent et les erreurs/console JS récentes. Utiliser quand une page web, un fetch interne ou une interaction semble échouer, plutôt que d'appeler browser_status puis browser_network_log séparément.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Nombre maximal d'événements réseau/console à retourner, défaut 50, max 200." },
                "clear": { "type": "boolean", "description": "Si true, vide les journaux après lecture. Défaut false." },
                "max_elements": { "type": "integer", "description": "Nombre max d'éléments interactifs dans l'observation. Défaut 60, max 120." }
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_tool_definitions_keep_public_order() {
        let tools = browser_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "browser_batch",
                "browser_navigate",
                "browser_click",
                "browser_type",
                "browser_keys",
                "browser_select",
                "browser_hover",
                "browser_screenshot",
                "browser_read_page",
                "browser_close",
                "browser_scroll",
                "browser_wait",
                "browser_run_js",
                "browser_back",
                "browser_status",
                "browser_network_log",
                "browser_observe",
                "browser_diagnostics",
            ]
        );
    }

    #[test]
    fn browser_tool_definitions_keep_batch_contract() {
        let tools = browser_tool_definitions();
        let batch = tool(&tools, "browser_batch");
        let steps = property(batch, "steps");

        assert_eq!(required_fields(batch), vec!["steps"]);
        assert_eq!(steps["minItems"].as_i64(), Some(1));
        assert_eq!(steps["maxItems"].as_i64(), Some(20));
        assert_eq!(required_fields_from(&steps["items"]), vec!["action"]);
        assert_eq!(
            enum_values(&property(batch, "final_observation")["enum"]),
            vec!["observe", "read_page", "status", "diagnostics", "none"]
        );
    }

    #[test]
    fn browser_tool_definitions_keep_input_and_observe_contracts() {
        let tools = browser_tool_definitions();

        assert_eq!(
            required_fields(tool(&tools, "browser_type")),
            vec!["selector", "text"]
        );
        assert_eq!(
            required_fields(tool(&tools, "browser_select")),
            vec!["selector", "value"]
        );
        assert_eq!(
            required_fields(tool(&tools, "browser_wait")),
            vec!["selector"]
        );
        assert_contains(
            property(tool(&tools, "browser_observe"), "max_elements"),
            "max 120",
        );
        assert_contains(
            property(tool(&tools, "browser_network_log"), "limit"),
            "max 200",
        );
        assert_contains(
            property(tool(&tools, "browser_diagnostics"), "limit"),
            "max 200",
        );
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("missing browser tool {name}"))
    }

    fn required_fields(definition: &ToolDefinition) -> Vec<&str> {
        required_fields_from(&definition.input_schema)
    }

    fn required_fields_from(value: &Value) -> Vec<&str> {
        value
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

    fn assert_contains(property: &Value, expected: &str) {
        let description = property["description"]
            .as_str()
            .unwrap_or_else(|| panic!("missing description on property {property:?}"));
        assert!(
            description.contains(expected),
            "description did not contain {expected:?}: {description}"
        );
    }
}
