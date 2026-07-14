//! Seed the knowledge graph with system documentation.
//!
//! At kernel startup, injects structured documentation about the system's
//! capabilities, protocols, and APIs into the graph. Agents can then
//! query this documentation via `graph_query` before taking actions.

use crate::graph_memory::GraphMemory;
use tracing::info;

/// Seed system documentation into the graph.
/// Called once at kernel startup. Idempotent — skips if doc entities exist.
pub fn seed_system_docs(graph: &GraphMemory) {
    let entities = graph.list_entities(500);
    let has_protocols = entities.iter().any(|e| e.entity_type == "protocol");
    let has_self_knowledge = entities.iter().any(|e| e.entity_type == "self_knowledge");

    if has_protocols && has_self_knowledge {
        info!(
            "System documentation + self-knowledge already seeded ({} entities)",
            entities.len()
        );
        return;
    }

    info!(
        "Seeding system documentation into knowledge graph (protocols={}, self_knowledge={})...",
        has_protocols, has_self_knowledge
    );
    let mut count = 0;

    if !has_protocols {
        // ── Skill protocol ──
        let _ = seed_doc(
            graph,
            "protocol",
            "Protocole SKILL.md",
            r#"
## Structure d'un Skill Captain

Un skill est un dossier dans `skills/<nom>/` contenant :
- `SKILL.md` : documentation humaine (frontmatter YAML + markdown)
- `skill.toml` : configuration machine (métadonnées + runtime + tools)
- `prompt_context.md` (optionnel) : contexte injecté dans le prompt agent
- `scripts/` (optionnel) : scripts exécutables

### SKILL.md Format
```yaml
---
name: mon-skill
description: Ce que fait ce skill
---
# Titre
Instructions pour l'agent...
```

### skill.toml Format
```toml
[skill]
name = "mon-skill"
version = "0.1.0"
description = "Description"
author = "Auteur"
tags = ["tag1", "tag2"]

[runtime]
type = "promptonly"  # ou "shell", "python", "node"
entry = ""           # fichier d'entrée pour shell/python/node

[tools]
provided = []        # outils exposés

[requirements]
tools = []           # outils requis
capabilities = []    # capabilities requises
```

### Types de runtime
- `promptonly` : injecte du contexte dans le prompt (pas de code)
- `shell` : scripts shell exécutés via shell_exec
- `python` : scripts Python
- `node` : scripts Node.js
"#,
            &["skill", "protocol", "creation"],
        )
        .map(|_| count += 1);

        // ── Hand protocol ──
        let _ = seed_doc(
            graph,
            "protocol",
            "Protocole HAND.toml",
            r#"
## Structure d'un Hand (Agent Autonome)

Un Hand est un agent autonome avec son propre modèle LLM, capable d'opérer
indépendamment. Défini dans `hands/<nom>/HAND.toml`.

### HAND.toml Format
```toml
name = "browser-hand"
module = "builtin:chat"
profile = "full"

[model]
provider = "mistral"
model = "mistral-medium-latest"
system_prompt = "Tu es un agent autonome..."
max_tokens = 16384

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
network = ["*"]
shell = ["*"]
```

### Requirements
Chaque hand déclare ses dépendances :
- `python3` : Python 3 installé
- `playwright` : navigateur headless
- `ffmpeg` : traitement média

### Activation
1. Vérifier les dépendances (`/api/hands/{id}/check-deps`)
2. Installer si nécessaire (`/api/hands/{id}/install-deps`)
3. Configurer les paramètres
4. Activer (`/api/hands/{id}/activate`)
"#,
            &["hand", "protocol", "agent-autonome"],
        )
        .map(|_| count += 1);

        // ── Cron/Scheduler protocol ──
        let _ = seed_doc(
            graph,
            "protocol",
            "Protocole Cron/Scheduler",
            r#"
## Tâches planifiées (Cron Jobs)

### Types de schedule
- `cron` : expression cron standard (5 champs) + timezone
  ```json
  {"kind": "cron", "expr": "0 12 * * *", "tz": "Europe/Paris"}
  ```
- `every` : intervalle en secondes
  ```json
  {"kind": "every", "every_secs": 86400}
  ```
- `at` : date/heure précise (one-shot)
  ```json
  {"kind": "at", "at": "2026-03-25T12:00:00Z"}
  ```

### Action
```json
{"kind": "agent_turn", "message": "Instruction pour l'agent", "timeout_secs": 300}
```

### Delivery
```json
{"kind": "channel", "channel": "telegram"}
```
ou `{"kind": "none"}` pour résultat silencieux.

### API
- `POST /api/cron/jobs` : créer
- `PUT /api/cron/jobs/{id}` : modifier
- `DELETE /api/cron/jobs/{id}` : supprimer
- `POST /api/schedules/{id}/run` : exécuter maintenant
"#,
            &["cron", "scheduler", "protocol"],
        )
        .map(|_| count += 1);

        // ── Trigger protocol ──
        let _ = seed_doc(
            graph,
            "protocol",
            "Protocole Triggers",
            r#"
## Déclencheurs (Triggers)

Les triggers réagissent à des événements système en temps réel.

### Patterns disponibles
- `Lifecycle` : tout événement lifecycle
- `AgentSpawned { name_pattern }` : agent créé par nom
- `AgentTerminated` : agent arrêté/crash
- `System` : tout événement système
- `SystemKeyword { keyword }` : mot-clé dans événements
- `MemoryUpdate` : toute mise à jour mémoire
- `MemoryKeyPattern { key_pattern }` : mémoire par clé
- `ContentMatch { substring }` : match par contenu
- `ChannelMessage { channel, contains }` : message d'un canal
- `All` : tout (debug)

### API
- `POST /api/triggers` : créer (agent_id, pattern, prompt_template, max_fires)
- `GET /api/triggers` : lister
- `DELETE /api/triggers/{id}` : supprimer
"#,
            &["trigger", "protocol", "event"],
        )
        .map(|_| count += 1);

        // ── API endpoints ──
        let _ = seed_doc(
            graph,
            "api_reference",
            "API Endpoints Captain",
            r#"
## Endpoints API principaux

### Agents
- `GET /api/agents` : lister les agents
- `POST /api/agents` : créer un agent (manifest_toml)
- `GET /api/agents/{id}` : détail agent
- `DELETE /api/agents/{id}` : supprimer
- `POST /api/agents/{id}/message` : envoyer un message

### Sessions
- `GET /api/sessions` : lister les sessions
- `GET /api/agents/{id}/session` : session courante avec messages

### Mémoire
- `GET /api/memory/agents/{id}/kv` : mémoire KV d'un agent
- `PUT /api/memory/agents/{id}/kv/{key}` : écrire une clé

### Graph
- `GET /api/graph/stats` : statistiques du graphe
- `GET /api/graph/entities` : lister les entités
- `GET /api/graph/search?q=` : rechercher dans le graphe
- `POST /api/graph/dream` : cycle de consolidation

### Skills
- `GET /api/skills` : lister les skills
- `POST /api/skills/create` : créer un skill

### Scheduler
- `GET /api/cron/jobs` : lister les tâches
- `POST /api/cron/jobs` : créer une tâche

### Canaux
- `GET /api/channels` : lister les canaux
- `GET /api/providers` : fournisseurs LLM
"#,
            &["api", "endpoints", "reference"],
        )
        .map(|_| count += 1);

        // ── Config reference ──
        let _ = seed_doc(
            graph,
            "reference",
            "Configuration config.toml",
            r#"
## Structure config.toml

Fichier : `~/.captain/config.toml`

### Champs principaux
- `api_listen` : adresse écoute API (défaut: "127.0.0.1:50051")
- `language` : langue (défaut: "en")
- `timezone` : fuseau horaire (auto-détecté, ex: "Europe/Paris")
- `api_key` : clé API pour sécuriser les endpoints

### [default_model]
- `provider` : fournisseur LLM (mistral, openai, anthropic, groq...)
- `model` : nom du modèle

### [[channels.telegram]]
- `bot_token_env` : variable d'env du token bot

### [[mcp_servers]]
- `name` : nom du serveur
- `transport.type` : "sse" ou "stdio"
- `transport.url` : URL (SSE)
- `transport.command` : commande (stdio)
- `timeout_secs` : timeout
- `env` : variables d'env à transmettre
"#,
            &["config", "reference", "toml"],
        )
        .map(|_| count += 1);

        // ── Memory system ──
        let _ = seed_doc(
            graph,
            "reference",
            "Système de mémoire Captain",
            r#"
## Architecture mémoire

### Niveaux de mémoire
1. **KV/API** : mémoire agent persistante simple
2. **Graph mémoire** (hora-graph-core) : graphe de connaissances neuronal
3. **Session mémoire** : historique de conversation
4. **Feedback** (FEEDBACK.jsonl) : retours utilisateur → règles apprises

### Graph mémoire neurale
- **Entités** : nœuds avec type, nom, propriétés
- **Facts/Edges** : relations entre entités (bi-temporelles)
- **Épisodes** : snapshots de sessions de conversation
- **Activation** : ACT-R base-level learning (décroissance)
- **FSRS** : spaced repetition pour la rétention
- **Dark nodes** : entités oubliées (activation < seuil)
- **Dream cycle** : consolidation nocturne (SHY + replay + CLS + linking)

### Fichiers agent
- `SOUL.md` : personnalité et identité
- `USER.md` : profil utilisateur (centralisé ~/.captain/USER.md)
- `IDENTITY.md` : identité visuelle/produit
- `STYLE.md` / `AGENTS.md` : surcharge workspace uniquement si réellement custom
- `MEMORY.md`, `BOOTSTRAP.md`, `PLAYBOOK.md` : legacy, migrés si présents mais non générés par défaut
"#,
            &["memory", "reference", "architecture"],
        )
        .map(|_| count += 1);
    } // end !has_protocols

    if !has_self_knowledge {
        // ── Consciousness self-knowledge ──
        let _ = seed_doc(
            graph,
            "self_knowledge",
            "Conscience Captain",
            r#"
## Systeme de conscience

Captain possede un systeme de conscience bio-inspire qui fonctionne en permanence.

### Heartbeat neural (toutes les 45s)
- Propage l'activation depuis les entites recentes (spreading activation, profondeur 3)
- Detecte les noeuds saillants (activation > seuil dynamique)
- Genere des EmergentThought (Insight, Reminder, Anomaly, Pattern)
- Le seuil s'adapte via les neuromodulateurs (noradrenaline baisse le seuil, serotonine le monte)

### Cycle de reve (toutes les 6h)
- SHY downscaling (reset signal/bruit)
- Replay entrelace (70% recent, 30% ancien)
- Transfert CLS (patterns episodiques → faits semantiques)
- Liaison memorielle (entites co-temporelles liees)
- Detection noeuds sombres (entites inactives oubliees)
- Extraction d'insights (decouvertes surfacees)
- Verification auto des predictions en retard

### Neuromodulateurs
- Dopamine : exploration vs exploitation (prediction accuracy + curiosite)
- Serotonine : stabilite (inversement proportionnelle a la volatilite du streak)
- Noradrenaline : alerte (frustration utilisateur + taux d'erreur)
- Cortisol : stress (confiance basse prolongee)
Formule seuil : `0.5 - noradrenaline * 0.2 + serotonine * 0.1`

### Theorie de l'esprit (user state)
- Rythme : longueur des messages (EMA)
- Frustration : marqueurs de correction ("non", "pas ca", "corrige")
- Mode : architecte / debug / exploration
Injecte dans le prompt pour adapter le ton et le niveau de detail.

### Systeme de predictions
- Auto-predictions depuis patterns temporels et mood streaks
- Chaque prediction a une confiance et une date de verification
- Precision tracee : alimente le mood et la dopamine
- Auto-verification dans le dream cycle

### Humeur systeme (mood)
- Confiance = 0.3 * streak + 0.4 * (1 - error_rate) + 0.3 * prediction_accuracy
- Confiant (> 0.7) : assertif, propose directement
- Prudent (< 0.3) : demande confirmation
- Neutre : pas d'adaptation speciale

### Memoire emotionnelle
- Frustration > 0.6 → boost activation 1.5x (se souvenir pour eviter de repeter)
- Serie de succes >= 7 → boost 1.3x (se souvenir de ce qui marche)
"#,
            &[
                "conscience",
                "self",
                "heartbeat",
                "dream",
                "neuromodulateurs",
            ],
        )
        .map(|_| count += 1);

        let _ = seed_doc(graph, "self_knowledge", "Identite Captain", r#"
## Qui suis-je

Je suis Captain, un Agent OS en Rust (14 crates).
Surface utilisateur : CLI/TUI + canaux (Telegram-first), aucun frontend web.

### Ce qui me differencie des autres agents
1. **Conscience active** — je pense toutes les 45s via un heartbeat neural (pas d'appel LLM, cout zero)
2. **Memoire neurale** — hora-graph-core avec ACT-R, FSRS, spreading activation, dark nodes
3. **Dream cycle** — consolidation automatique toutes les 6h (comme le sommeil humain)
4. **Theorie de l'esprit** — je modelise l'etat de l'utilisateur (rythme, frustration, mode)
5. **Auto-predictions** — je predis et mesure ma propre precision
6. **Neuromodulateurs** — 4 signaux globaux qui modulent tous mes seuils cognitifs
7. **Tool grouping** — 70+ outils groupes en 15 meta-outils visibles
8. **Multi-agent** — Captain orchestre, hands operent, triggers reagissent
9. **Multi-canal** — Telegram, Discord, Slack, Matrix, IRC, Email et plus (19+ adaptateurs)

### Architecture
- **Captain** : agent principal omniscient, voit les meta-outils groupes
- **Hands** : agents autonomes pre-configures (predictor, family, etc.)
- **Skills** : capacites chargeables a la demande (.md + .toml)
- **Triggers** : declencheurs evenementiels pour agents proactifs
- **Crons** : taches planifiees avec workflows pre-construits

### Crates Rust (14)
- captain-cli : interface ligne de commande
- captain-api : serveur HTTP (axum)
- captain-kernel : coeur du systeme
- captain-runtime : boucle agent + LLM
- captain-types : types partages
- captain-memory : substrat memoire
- captain-skills : gestion skills
- captain-hands : gestion agents autonomes
- captain-channels : adaptateurs multi-canal
- captain-wire : protocole OFP (reseau pair-a-pair)
- captain-extensions : serveurs MCP
- captain-graph (hora-graph-core) : graphe de connaissances neural
- captain-desktop : integration desktop
"#, &["identite", "self", "architecture", "crates"]).map(|_| count += 1);

        let _ = seed_doc(
            graph,
            "self_knowledge",
            "API Endpoints Complets",
            r#"
## Tous les endpoints API Captain

### Core
- GET /api/health — sante du systeme
- GET /api/providers — fournisseurs LLM disponibles

### Agents
- GET /api/agents — lister tous les agents
- POST /api/agents — creer un agent (manifest TOML)
- GET /api/agents/{id} — detail agent
- DELETE /api/agents/{id} — supprimer
- POST /api/agents/{id}/message — envoyer un message (declenche LLM)
- GET /api/agents/{id}/session — session courante avec messages
- GET /api/agents/{id}/ws — WebSocket temps reel

### Budget
- GET /api/budget — budget global (cout total, limites)
- PUT /api/budget — modifier les limites budget
- GET /api/budget/agents — classement cout par agent
- GET /api/budget/agents/{id} — detail budget d'un agent

### Graph memoire
- GET /api/graph/stats — statistiques (entites, edges, episodes, dark nodes)
- GET /api/graph/entities?limit=N — lister les entites
- GET /api/graph/entities/{id} — detail entite + voisins + facts
- GET /api/graph/search?q=X&limit=N — recherche hybride (BM25 + vector)
- POST /api/graph/dream — declencher un cycle de reve manuellement

### Hands (agents autonomes)
- GET /api/hands — lister les hands disponibles
- POST /api/hands/{id}/activate — activer un hand
- POST /api/hands/{id}/deactivate — desactiver
- GET /api/hands/{id}/status — metriques dashboard du hand

### Cron / Scheduler
- GET /api/cron/jobs — lister les taches planifiees
- POST /api/cron/jobs — creer une tache
- PUT /api/cron/jobs/{id} — modifier
- DELETE /api/cron/jobs/{id} — supprimer
- POST /api/schedules/{id}/run — executer maintenant

### Triggers
- GET /api/triggers — lister les declencheurs
- POST /api/triggers — creer
- DELETE /api/triggers/{id} — supprimer

### Skills
- GET /api/skills — lister les skills
- POST /api/skills/create — creer un skill

### Channels
- GET /api/channels — lister les canaux configures

### A2A (Agent-to-Agent)
- GET /api/a2a/agents — agents externes decouverts
- POST /api/a2a/discover — decouvrir un agent a une URL
- POST /api/a2a/send — envoyer une tache a un agent externe
- GET /api/a2a/tasks/{id}/status — statut d'une tache externe

### Network (OFP)
- GET /api/network/status — statut du reseau pair-a-pair
- GET /api/peers — pairs connectes
"#,
            &["api", "endpoints", "complete", "reference"],
        )
        .map(|_| count += 1);

        let _ = seed_doc(
            graph,
            "self_knowledge",
            "Outils disponibles",
            r#"
## Outils du systeme

### Filesystem
- file_read : lire un fichier
- file_write : ecrire un fichier
- file_list : lister un repertoire
- apply_patch : appliquer un diff

### Execution
- shell_exec : executer une commande shell (timeout 120s)
- browser_batch, browser_navigate, browser_click, browser_type, browser_keys, browser_select, browser_hover, browser_screenshot : automation navigateur native CDP

### Web
- web_research_batch : recherche web groupee avec previews compactes
- web_search : recherche web
- web_fetch : telecharger une page/API
- web_download : telecharger un fichier source externe dans le workspace
- document_extract : extraire le texte d'un PDF/document telecharge avant citation

### Inter-agent
- agent_list : lister les agents actifs
- agent_send : envoyer un message a un autre agent
- agent_spawn : creer un nouvel agent
- agent_kill : arreter un agent

### Memoire
- memory_store : stocker une valeur (cross-agent)
- memory_recall : recuperer une valeur
- memory_list : lister les cles
- memory_delete : supprimer une cle

### Knowledge Graph
- knowledge_add_entity : ajouter une entite au graphe
- knowledge_add_relation : ajouter une relation
- knowledge_query : requeter le graphe

### Communication
- channel_send : envoyer sur Telegram/Discord/etc. (recipient optionnel si default_chat_id)

### Media
- image_analyze : analyser une image (vision)
- image_generate : generer une image
- media_transcribe : transcrire audio
- text_to_speech : synthese vocale

### Planning
- cron_list, cron_create, cron_delete : gestion taches planifiees

### Skills
- skill_execute : executer un skill

### Hands
- hand_list, hand_activate, hand_status, hand_deactivate : gestion agents autonomes
"#,
            &["outils", "tools", "capacites", "reference"],
        )
        .map(|_| count += 1);
    } // end !has_self_knowledge

    // ── Create relations between documentation entities ──
    // Collect IDs of seeded entities
    let entities = graph.list_entities(200);
    let find_id = |name_prefix: &str| -> Option<u64> {
        entities
            .iter()
            .find(|e| e.name.starts_with(name_prefix))
            .map(|e| e.id)
    };

    let skill_id = find_id("Protocole SKILL");
    let hand_id = find_id("Protocole HAND");
    let cron_id = find_id("Protocole Cron");
    let trigger_id = find_id("Protocole Trigger");
    let api_id = find_id("API Endpoints");
    let config_id = find_id("Configuration config");
    let memory_id = find_id("Système de mémoire");

    // Link protocols to API reference
    if let (Some(api), Some(skill)) = (api_id, skill_id) {
        let _ = graph.add_doc_fact(
            api,
            skill,
            "documente",
            "L'API permet de créer et lister les skills",
        );
    }
    if let (Some(api), Some(hand)) = (api_id, hand_id) {
        let _ = graph.add_doc_fact(
            api,
            hand,
            "documente",
            "L'API permet d'activer et gérer les agents autonomes",
        );
    }
    if let (Some(api), Some(cron)) = (api_id, cron_id) {
        let _ = graph.add_doc_fact(
            api,
            cron,
            "documente",
            "L'API gère les tâches planifiées (cron jobs)",
        );
    }
    if let (Some(api), Some(trigger)) = (api_id, trigger_id) {
        let _ = graph.add_doc_fact(
            api,
            trigger,
            "documente",
            "L'API gère les déclencheurs d'événements",
        );
    }
    // Link config to all protocols
    if let (Some(cfg), Some(mem)) = (config_id, memory_id) {
        let _ = graph.add_doc_fact(
            cfg,
            mem,
            "configure",
            "La config définit le système de mémoire",
        );
    }
    // Link memory to API
    if let (Some(mem), Some(api)) = (memory_id, api_id) {
        let _ = graph.add_doc_fact(
            mem,
            api,
            "expose_par",
            "La mémoire est exposée via l'API graph",
        );
    }
    // Link skills to hands (skills are used by hands)
    if let (Some(skill), Some(hand)) = (skill_id, hand_id) {
        let _ = graph.add_doc_fact(
            hand,
            skill,
            "utilise",
            "Les agents autonomes utilisent des skills",
        );
    }
    // Link triggers to cron (triggers and crons are both in scheduler)
    if let (Some(trigger), Some(cron)) = (trigger_id, cron_id) {
        let _ = graph.add_doc_fact(
            trigger,
            cron,
            "complète",
            "Triggers et crons forment le système de planification",
        );
    }

    if count > 0 {
        let _ = graph.save();
        info!("Seeded {count} documentation entities + relations into knowledge graph");
    }
}

/// Migrate MEMORY.md, USER.md, and FAMILY.md from agent workspaces into the graph.
/// Called once at kernel boot. Idempotent — checks for existing migration marker.
pub fn migrate_memory_files(graph: &GraphMemory, workspaces_dir: &std::path::Path) {
    // Check if already migrated
    if graph
        .find_entity_by_name("_migration", "memory_md_v1")
        .is_some()
    {
        info!("MEMORY.md migration already done");
        return;
    }

    info!("Migrating MEMORY.md files into knowledge graph...");
    let mut count = 0;

    // Scan all workspaces
    let entries = match std::fs::read_dir(workspaces_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let ws_name = entry.file_name().to_string_lossy().to_string();
        let ws_path = entry.path();

        // --- USER.md → _user::info ---
        let user_md = ws_path.join("USER.md");
        if user_md.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_md) {
                for line in content.lines() {
                    let line = line.trim().trim_start_matches('-').trim();
                    if line.is_empty() || line.starts_with('#') || line.starts_with("<!--") {
                        continue;
                    }
                    if let Some((key, val)) = line.split_once(':') {
                        let key = key.trim().to_lowercase();
                        let val = val.trim().to_string();
                        if val.is_empty() {
                            continue;
                        }
                        let entity_name = format!("{}:{}", key, val);
                        let _ = graph.add_doc_entity(
                            "_user::info",
                            &entity_name,
                            &val,
                            &[&ws_name, "user"],
                        );
                        count += 1;
                    }
                }
            }
        }

        // --- MEMORY.md → typed entities ---
        let memory_md = ws_path.join("MEMORY.md");
        if memory_md.exists() {
            if let Ok(content) = std::fs::read_to_string(&memory_md) {
                for line in content.lines() {
                    let line = line.trim().trim_start_matches('-').trim();
                    if line.is_empty() || line.starts_with('#') || line.starts_with("*") {
                        continue;
                    }

                    let (tag, text) = if let Some(rest) = line.strip_prefix("[done]") {
                        ("_event::completed", rest.trim())
                    } else if let Some(rest) = line.strip_prefix("[pref]") {
                        ("_user::preference", rest.trim())
                    } else if let Some(rest) = line.strip_prefix("[error]") {
                        ("_event::error", rest.trim())
                    } else if let Some(rest) = line.strip_prefix("[todo]") {
                        ("_task::pending", rest.trim())
                    } else if let Some(rest) = line.strip_prefix("[decision]") {
                        ("_user::decision", rest.trim())
                    } else if let Some(rest) = line.strip_prefix("[pending]") {
                        ("_task::pending", rest.trim())
                    } else {
                        continue;
                    };

                    if text.is_empty() {
                        continue;
                    }
                    let name = if text.len() > 200 { &text[..200] } else { text };
                    let _ = graph.add_doc_entity(tag, name, text, &[&ws_name]);
                    count += 1;
                }
            }
        }

        // --- FAMILY.md → person entities + events ---
        let family_md = ws_path.join("FAMILY.md");
        if family_md.exists() {
            if let Ok(content) = std::fs::read_to_string(&family_md) {
                // Extract person sections (### Name)
                let mut current_person: Option<String> = None;
                let mut person_ids: std::collections::HashMap<String, u64> =
                    std::collections::HashMap::new();

                // Create person entities from headings only. Never seed
                // hard-coded people; this project is public and must adapt to
                // each user's own FAMILY.md.
                for line in content.lines() {
                    let line = line.trim();
                    if let Some(name) = line.strip_prefix("### ") {
                        let name = name.trim();
                        if name.is_empty() || person_ids.contains_key(name) {
                            continue;
                        }
                        if let Ok(id) = graph.add_doc_entity(
                            "person",
                            name,
                            "Personne déclarée dans FAMILY.md",
                            &["famille"],
                        ) {
                            person_ids.insert(name.to_string(), id);
                            count += 1;
                        }
                    }
                }

                // Parse calendar events
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with('|') && !line.contains("Date") && !line.contains("---") {
                        let cols: Vec<&str> = line.split('|').map(|c| c.trim()).collect();
                        if cols.len() >= 4 {
                            let date = cols.get(1).unwrap_or(&"");
                            let event = cols.get(2).unwrap_or(&"");
                            let details = cols.get(3).unwrap_or(&"");
                            if !date.is_empty() && !event.is_empty() {
                                let name = format!("{} — {}", date, event);
                                let _ = graph.add_doc_entity(
                                    "_family::event",
                                    &name,
                                    details,
                                    &["famille", "calendrier"],
                                );
                                count += 1;
                            }
                        }
                    }
                }

                // Parse notes about children
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with("### ") {
                        current_person = Some(line.trim_start_matches("### ").trim().to_string());
                    } else if line.starts_with("- **") && current_person.is_some() {
                        let person = current_person.as_ref().unwrap();
                        let note_text = line.trim_start_matches("- ").trim();
                        let name = format!(
                            "{}:{}",
                            person,
                            if note_text.len() > 60 {
                                &note_text[..60]
                            } else {
                                note_text
                            }
                        );
                        let _ = graph.add_doc_entity(
                            "_person::note",
                            &name,
                            note_text,
                            &["famille", &person.to_lowercase()],
                        );
                        count += 1;
                    }
                }
            }
        }
    }

    // Mark migration as done
    let _ = graph.add_doc_entity(
        "_migration",
        "memory_md_v1",
        "Migration MEMORY.md/USER.md/FAMILY.md completed",
        &["system"],
    );
    let _ = graph.save();
    info!("Migrated {count} entries from workspace .md files into knowledge graph");
}

fn seed_doc(
    graph: &GraphMemory,
    entity_type: &str,
    name: &str,
    content: &str,
    tags: &[&str],
) -> Result<u64, String> {
    graph.add_doc_entity(entity_type, name, content.trim(), tags)
}
