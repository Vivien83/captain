//! Graph-enhanced memory via hora-graph-core.
//!
//! Stores conversation turns as entities in the knowledge graph,
//! extracts semantic entities and relations, creates episodes for
//! dream-cycle consolidation, and provides activation-aware recall.

use chrono::Datelike;
use hora_graph_core::{
    EntityId, EpisodeSource, HoraConfig, HoraCore, PropertyValue, SearchOpts, SpreadingParams,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Maximum recent entities tracked for neural heartbeat.
const RECENT_ENTITY_BUFFER_SIZE: usize = 50;

/// Maximum queued thoughts before oldest are discarded.
const THOUGHT_QUEUE_CAPACITY: usize = 20;

/// Minimum seconds between surfacing thoughts (anti-spam).
const THOUGHT_COOLDOWN_SECS: u64 = 120;

/// Wraps `HoraCore` for conversation memory with dedup and BM25 recall.
pub struct GraphMemory {
    graph: Mutex<HoraCore>,
    last_message_hash: Mutex<Option<(u64, Instant)>>,
    file_path: Option<PathBuf>,
    /// Ring buffer of recently touched entity IDs (for heartbeat pulse).
    recent_entities: Mutex<std::collections::VecDeque<EntityId>>,
    /// Queue of thoughts pending delivery to agents.
    thought_queue: Mutex<std::collections::VecDeque<EmergentThought>>,
    /// Timestamp of last surfaced thought (cooldown enforcement).
    last_surfaced: Mutex<Option<Instant>>,
    /// D.3: Inferred user state (theory of mind).
    user_state: Mutex<UserState>,
    /// D.5: System-wide mood/confidence signal.
    system_mood: Mutex<SystemMood>,
    /// D.6: Temporal action counts — key: "action:weekday:hour", value: (count, last_ts).
    temporal_counts: Mutex<HashMap<String, (u32, i64)>>,
    /// D.9: Global neuromodulators.
    neuromodulators: Mutex<NeuroModulators>,
    /// Batch buffer for Telegram digest (instead of spamming each thought).
    thought_digest: Mutex<Vec<EmergentThought>>,
    /// Last assistant turn ID — for feedback scoring when next user message arrives.
    last_assistant_turn: Mutex<Option<(EntityId, String)>>,
    /// M.4 cache: shared knowledge prompt (rebuilt every 60s, not every message).
    shared_knowledge_cache: Mutex<(String, Instant)>,
    /// D.8: Ring buffer of recent narration summaries (last 5).
    recent_narrations: Mutex<std::collections::VecDeque<String>>,
}

impl GraphMemory {
    /// Create a new graph memory, optionally persisted to disk.
    pub fn new(persist_path: Option<PathBuf>) -> Result<Self, String> {
        let config = HoraConfig {
            embedding_dims: 0,
            ..HoraConfig::default()
        };

        let graph = if let Some(ref path) = persist_path {
            HoraCore::open(path, config).map_err(|e| format!("Failed to open graph: {e}"))?
        } else {
            HoraCore::new(config).map_err(|e| format!("Failed to create graph: {e}"))?
        };

        Ok(Self {
            graph: Mutex::new(graph),
            last_message_hash: Mutex::new(None),
            file_path: persist_path,
            recent_entities: Mutex::new(std::collections::VecDeque::with_capacity(
                RECENT_ENTITY_BUFFER_SIZE,
            )),
            thought_queue: Mutex::new(std::collections::VecDeque::with_capacity(
                THOUGHT_QUEUE_CAPACITY,
            )),
            last_surfaced: Mutex::new(None),
            user_state: Mutex::new(UserState::default()),
            system_mood: Mutex::new(SystemMood::default()),
            temporal_counts: Mutex::new(HashMap::new()),
            neuromodulators: Mutex::new(NeuroModulators::default()),
            thought_digest: Mutex::new(Vec::new()),
            last_assistant_turn: Mutex::new(None),
            shared_knowledge_cache: Mutex::new((String::new(), Instant::now())),
            recent_narrations: Mutex::new(std::collections::VecDeque::with_capacity(5)),
        })
    }

    /// Store a conversation turn in the graph with semantic extraction.
    ///
    /// 1. Creates a `_conv::turn` entity (as before)
    /// 2. Extracts named entities from content (heuristic, no LLM)
    /// 3. Creates relations between the turn and extracted entities
    /// 4. Records an episode for dream-cycle consolidation
    ///
    /// Dedup: skip if same message hash within 60s. Skip messages < 3 chars.
    pub fn store_turn(&self, agent_name: &str, role: &str, content: &str) -> Result<(), String> {
        if content.len() < 3 {
            return Ok(());
        }

        let hash = simple_hash(content);
        {
            let mut last = self.last_message_hash.lock().unwrap();
            if let Some((prev_hash, prev_time)) = *last {
                if prev_hash == hash && prev_time.elapsed().as_secs() < 60 {
                    debug!("Skipping duplicate message (dedup 60s)");
                    return Ok(());
                }
            }
            *last = Some((hash, Instant::now()));
        }

        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());

        // --- 1. Create the turn entity ---
        let mut props = hora_graph_core::Properties::new();
        props.insert("role".into(), PropertyValue::String(role.into()));
        props.insert("agent".into(), PropertyValue::String(agent_name.into()));
        props.insert("content".into(), PropertyValue::String(content.into()));

        let name = truncate_utf8(content, 200);

        let turn_id = graph
            .add_entity("_conv::turn", name, Some(props), None)
            .map_err(|e| format!("Failed to store turn: {e}"))?;

        // --- 2. Extract semantic entities (heuristic) ---
        // ONLY extract from user messages — assistant responses may contain hallucinations
        let extracted = if role == "user" {
            extract_entities(content)
        } else {
            vec![]
        };
        let mut entity_ids: Vec<EntityId> = vec![turn_id];
        let mut fact_ids: Vec<hora_graph_core::EdgeId> = Vec::new();

        for mention in &extracted {
            let eid = graph
                .add_entity(&mention.entity_type, &mention.name, None, None)
                .map_err(|e| format!("Failed to add extracted entity: {e}"))?;
            entity_ids.push(eid);

            // --- 3. Relation: turn --mentions--> entity ---
            let fact_id = graph
                .add_fact(turn_id, eid, "mentions", &mention.name, None)
                .map_err(|e| format!("Failed to add mention fact: {e}"))?;
            fact_ids.push(fact_id);
        }

        // Relate turn to its agent
        let agent_eid = graph
            .add_entity("agent", agent_name, None, None)
            .map_err(|e| format!("Failed to add agent entity: {e}"))?;
        entity_ids.push(agent_eid);
        let agent_fact = graph
            .add_fact(
                turn_id,
                agent_eid,
                if role == "user" {
                    "sent_to"
                } else {
                    "produced_by"
                },
                agent_name,
                None,
            )
            .map_err(|e| format!("Failed to add agent fact: {e}"))?;
        fact_ids.push(agent_fact);

        // --- 4. Create episode for dream-cycle ---
        let session_id = format!("{}:{}", agent_name, chrono::Local::now().format("%Y-%m-%d"));
        let _ = graph
            .add_episode(
                EpisodeSource::Conversation,
                &session_id,
                &entity_ids,
                &fact_ids,
            )
            .map_err(|e| format!("Failed to add episode: {e}"))?;

        // --- 5. M.3: Extract preferences/configs from user messages ---
        if role == "user" {
            let lower = content.to_lowercase();

            // Preference patterns (direct)
            for prefix in &[
                "je préfère ",
                "je prefere ",
                "j'aime ",
                "je veux ",
                "j'aimerais ",
                "utilise toujours ",
                "envoie-moi ",
                "envoie moi ",
                "fais toujours ",
                "ne fais jamais ",
                "ne fait jamais ",
                "evite de ",
                "évite de ",
            ] {
                if let Some(rest) = lower.strip_prefix(prefix) {
                    let pref = rest
                        .split(&['.', ',', '!', '\n'][..])
                        .next()
                        .unwrap_or(rest)
                        .trim();
                    if pref.len() > 3 {
                        let _ = graph.add_entity("_user::preference", pref, None, None);
                    }
                }
            }

            // Preference patterns (inline — anywhere in the message)
            for marker in &[
                "je préfère ",
                "je prefere ",
                "toujours en ",
                "jamais de ",
                "par défaut ",
                "par defaut ",
            ] {
                if let Some(pos) = lower.find(marker) {
                    let rest = &lower[pos + marker.len()..];
                    let pref = rest
                        .split(&['.', ',', '!', '\n'][..])
                        .next()
                        .unwrap_or(rest)
                        .trim();
                    if pref.len() > 3 && pref.len() < 100 {
                        let _ = graph.add_entity("_user::preference", pref, None, None);
                    }
                }
            }

            // Info patterns: "mon X est Y", "ma X est Y"
            for prefix in &["mon ", "ma ", "mes "] {
                if lower.contains(prefix) {
                    if let Some(rest) = lower.split_once(prefix).map(|(_, r)| r) {
                        if let Some((key, val)) = rest.split_once(" est ") {
                            let key = key.trim();
                            let val = val
                                .split(&['.', ',', '!', '\n'][..])
                                .next()
                                .unwrap_or(val)
                                .trim();
                            if key.len() > 1 && val.len() > 1 && val.len() < 100 {
                                let name = format!("{}:{}", key, val);
                                let _ = graph.add_entity("_user::info", &name, None, None);
                            }
                        }
                    }
                }
            }
        }

        // --- 6. Feedback scoring: score previous assistant turn based on user reaction ---
        if role == "user" {
            let prev = self.last_assistant_turn.lock().unwrap().take();
            if let Some((prev_turn_id, _prev_content)) = prev {
                let confidence = score_user_feedback(content);
                let mut update_props = hora_graph_core::Properties::new();
                update_props.insert(
                    "feedback_score".into(),
                    PropertyValue::Float(confidence as f64),
                );
                let _ = graph.update_entity(
                    prev_turn_id,
                    hora_graph_core::EntityUpdate {
                        properties: Some(update_props),
                        ..Default::default()
                    },
                );
            }
        }
        if role == "assistant" {
            *self.last_assistant_turn.lock().unwrap() = Some((turn_id, content.to_string()));
        }

        // --- 7. Track recent entities for neural heartbeat ---
        {
            let mut recent = self.recent_entities.lock().unwrap();
            for eid in &entity_ids {
                if recent.len() >= RECENT_ENTITY_BUFFER_SIZE {
                    recent.pop_front();
                }
                recent.push_back(*eid);
            }
        }

        if !extracted.is_empty() {
            debug!(
                "Stored turn + {} extracted entities, {} facts, 1 episode",
                extracted.len(),
                fact_ids.len()
            );
        }

        Ok(())
    }

    /// Recall relevant memories using BM25 search + spreading activation.
    ///
    /// 1. BM25 search for initial hits
    /// 2. Spreading activation from hit entities to discover associated context
    /// 3. Combine scores: `saillance = bm25_score + spreading_activation`
    /// 4. Return enriched results sorted by saillance
    pub fn recall(&self, query: &str, top_k: usize) -> Vec<RecalledTurn> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());

        // --- Step 1: BM25 search (cast wider net: 3x top_k) ---
        let search_k = top_k * 3;
        let opts = SearchOpts {
            top_k: search_k,
            ..SearchOpts::default()
        };

        let hits = match graph.search(Some(query), None, opts) {
            Ok(h) => h,
            Err(e) => {
                warn!("Graph recall failed: {e}");
                return vec![];
            }
        };

        if hits.is_empty() {
            return vec![];
        }

        // --- Step 2: Spreading activation from BM25 hits ---
        let sources: Vec<(EntityId, f64)> =
            hits.iter().map(|h| (h.entity_id, h.score as f64)).collect();

        let spread_params = SpreadingParams {
            max_depth: 2,
            ..SpreadingParams::default()
        };
        let spread_activations = graph
            .spread_activation(&sources, &spread_params)
            .unwrap_or_default();

        // --- Step 3: Collect turn entities with combined saillance ---
        // Build a map of entity_id -> bm25_score for direct hits
        let bm25_scores: HashMap<u64, f32> =
            hits.iter().map(|h| (h.entity_id.0, h.score)).collect();

        // Gather all candidate entity IDs (direct hits + spread neighbors)
        let mut candidate_ids: Vec<EntityId> = hits.iter().map(|h| h.entity_id).collect();
        for &eid in spread_activations.keys() {
            if !bm25_scores.contains_key(&eid.0) {
                candidate_ids.push(eid);
            }
        }

        let mut results: Vec<RecalledTurn> = candidate_ids
            .iter()
            .filter_map(|&eid| {
                let entity = graph.get_entity(eid).ok()??;

                // Skip internal system entities (not useful for recall)
                if entity.entity_type.starts_with("_sys::")
                    || entity.entity_type.starts_with("_self::")
                    || entity.entity_type.starts_with("_migration")
                {
                    return None;
                }

                // For conversation turns: ONLY recall user messages, never assistant.
                // Assistant responses are not facts — they may contain hallucinations.
                if entity.entity_type == "_conv::turn" {
                    let role = entity
                        .properties
                        .get("role")
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or("");
                    if role != "user" {
                        return None; // Skip assistant/system turns
                    }
                }

                let content = entity
                    .properties
                    .get("content")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| entity.name.clone());
                let role = entity
                    .properties
                    .get("role")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let agent = entity
                    .properties
                    .get("agent")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();

                let bm25 = bm25_scores.get(&eid.0).copied().unwrap_or(0.0) as f64;
                let spread = spread_activations.get(&eid).copied().unwrap_or(0.0);
                let saillance = bm25 + spread.max(0.0);

                Some(RecalledTurn {
                    content,
                    role,
                    agent,
                    score: saillance as f32,
                })
            })
            .collect();

        // --- Step 4: Sort by saillance descending, take top_k ---
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);

        if !results.is_empty() {
            debug!(
                "Recall: {} BM25 hits → {} after spreading → {} returned (top_k={})",
                hits.len(),
                candidate_ids.len(),
                results.len(),
                top_k,
            );
        }

        results
    }

    /// M.4: Build shared knowledge context from graph for prompt injection.
    /// Returns a compact summary of user info, preferences, people, and habits.
    /// Cached for 60 seconds to avoid scanning the full graph on every message.
    pub fn shared_knowledge_prompt(&self) -> String {
        {
            let cache = self.shared_knowledge_cache.lock().unwrap();
            if !cache.0.is_empty() && cache.1.elapsed().as_secs() < 60 {
                return cache.0.clone();
            }
        }
        let result = self.build_shared_knowledge();
        let mut cache = self.shared_knowledge_cache.lock().unwrap();
        *cache = (result.clone(), Instant::now());
        result
    }

    fn build_shared_knowledge(&self) -> String {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let entities = match graph.scan_entities() {
            Ok(e) => e,
            Err(_) => return String::new(),
        };

        let mut lines = Vec::new();

        // User info
        let infos: Vec<&str> = entities
            .iter()
            .filter(|e| e.entity_type == "_user::info")
            .map(|e| e.name.as_str())
            .take(10)
            .collect();
        if !infos.is_empty() {
            lines.push("Utilisateur :".to_string());
            for info in &infos {
                lines.push(format!("  - {info}"));
            }
        }

        // Preferences
        let prefs: Vec<&str> = entities
            .iter()
            .filter(|e| e.entity_type == "_user::preference")
            .map(|e| e.name.as_str())
            .take(5)
            .collect();
        if !prefs.is_empty() {
            lines.push("Preferences :".to_string());
            for p in &prefs {
                lines.push(format!("  - {p}"));
            }
        }

        // People (with descriptions from content + notes)
        let people: Vec<String> = entities
            .iter()
            .filter(|e| e.entity_type == "person")
            .map(|e| {
                let desc = e
                    .properties
                    .get("content")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("");
                let tags = e
                    .properties
                    .get("tags")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("");
                if desc.is_empty() {
                    format!("{} ({})", e.name, tags)
                } else {
                    format!("{} — {} ({})", e.name, desc, tags)
                }
            })
            .take(8)
            .collect();
        if !people.is_empty() {
            lines.push("Personnes :".to_string());
            for p in &people {
                lines.push(format!("  - {p}"));
            }
        }

        // Person notes (details about family members)
        let notes: Vec<String> = entities
            .iter()
            .filter(|e| e.entity_type == "_person::note")
            .map(|e| {
                let content = e
                    .properties
                    .get("content")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or(&e.name);
                if content.len() > 120 {
                    format!("{}...", &content[..120])
                } else {
                    content.to_string()
                }
            })
            .take(10)
            .collect();
        if !notes.is_empty() {
            lines.push("Notes famille :".to_string());
            for n in &notes {
                lines.push(format!("  - {n}"));
            }
        }

        // Family events
        let events: Vec<&str> = entities
            .iter()
            .filter(|e| e.entity_type == "_family::event")
            .map(|e| e.name.as_str())
            .take(10)
            .collect();
        if !events.is_empty() {
            lines.push("Evenements famille :".to_string());
            for ev in &events {
                lines.push(format!("  - {ev}"));
            }
        }

        // Habits
        let habits: Vec<&str> = entities
            .iter()
            .filter(|e| e.entity_type == "_habit")
            .map(|e| e.name.as_str())
            .take(5)
            .collect();
        if !habits.is_empty() {
            lines.push("Habitudes :".to_string());
            for h in &habits {
                lines.push(format!("  - {h}"));
            }
        }

        if lines.is_empty() {
            return String::new();
        }

        let mut result = "[CONNAISSANCES PARTAGEES]\n".to_string();
        result.push_str(&lines.join("\n"));
        result
    }

    /// Recall past reflections relevant to the current context.
    /// Returns a formatted string of tool success/failure history for prompt injection.
    pub fn recall_reflections(&self, agent_name: &str, limit: usize) -> String {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let entities = match graph.scan_entities() {
            Ok(e) => e,
            Err(_) => return String::new(),
        };

        let mut reflections: Vec<(String, String, i64)> = entities
            .iter()
            .filter(|e| e.entity_type == "_self::reflection")
            .filter(|e| {
                e.properties
                    .get("agent")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str() == agent_name),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .map(|e| {
                let outcome = e
                    .properties
                    .get("outcome")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let tools = e
                    .properties
                    .get("tools_used")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                (outcome, tools, e.created_at)
            })
            .collect();

        reflections.sort_by_key(|r| std::cmp::Reverse(r.2));
        reflections.truncate(limit);

        if reflections.is_empty() {
            return String::new();
        }

        let mut lines = vec!["[REFLEXIONS PASSEES — apprendre de l'experience]".to_string()];
        for (outcome, tools, _) in &reflections {
            let (icon, label) = if outcome == "success" {
                ("✓", "succes")
            } else {
                ("✗", "echec")
            };
            lines.push(format!("- {} {} (outils: {})", icon, label, tools));
        }

        lines.join("\n")
    }

    /// Persist the graph to disk (if file-backed).
    pub fn save(&self) -> Result<(), String> {
        if self.file_path.is_some() {
            let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
            graph
                .flush()
                .map_err(|e| format!("Failed to flush graph: {e}"))?;
        }
        Ok(())
    }

    /// Get stats about the graph.
    pub fn stats(&self) -> GraphStats {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        match graph.stats() {
            Ok(s) => GraphStats {
                entities: s.entities as usize,
                edges: s.edges as usize,
            },
            Err(_) => GraphStats {
                entities: 0,
                edges: 0,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecalledTurn {
    pub content: String,
    pub role: String,
    pub agent: String,
    pub score: f32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphStats {
    pub entities: usize,
    pub edges: usize,
}

// ── Extended graph API for frontend + agent tools ──────────────────────────

/// Serializable entity for API responses.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphEntity {
    pub id: u64,
    pub name: String,
    pub entity_type: String,
    pub properties: std::collections::HashMap<String, serde_json::Value>,
    pub created_at: i64,
}

/// Serializable edge/fact for API responses.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphFact {
    pub id: u64,
    pub source: u64,
    pub target: u64,
    pub relation_type: String,
    pub description: String,
    pub confidence: f32,
    pub valid_at: i64,
    /// 0 means still valid (bi-temporal: valid_at..invalid_at)
    pub invalid_at: i64,
    pub created_at: i64,
}

/// Search result from the graph.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphSearchHit {
    pub entity_id: u64,
    pub name: String,
    pub entity_type: String,
    pub score: f32,
    pub snippet: String,
}

fn prop_to_json(val: &PropertyValue) -> serde_json::Value {
    match val {
        PropertyValue::String(s) => serde_json::Value::String(s.clone()),
        PropertyValue::Int(n) => serde_json::json!(*n),
        PropertyValue::Float(f) => serde_json::json!(*f),
        PropertyValue::Bool(b) => serde_json::json!(*b),
        _ => serde_json::Value::Null,
    }
}

fn entity_to_api(entity: &hora_graph_core::Entity) -> GraphEntity {
    GraphEntity {
        id: entity.id.0,
        name: entity.name.clone(),
        entity_type: entity.entity_type.clone(),
        properties: entity
            .properties
            .iter()
            .map(|(k, v)| (k.clone(), prop_to_json(v)))
            .collect(),
        created_at: entity.created_at,
    }
}

fn edge_to_api(edge: &hora_graph_core::Edge) -> GraphFact {
    GraphFact {
        id: edge.id.0,
        source: edge.source.0,
        target: edge.target.0,
        relation_type: edge.relation_type.clone(),
        description: edge.description.clone(),
        confidence: edge.confidence,
        valid_at: edge.valid_at,
        invalid_at: edge.invalid_at,
        created_at: edge.created_at,
    }
}

impl GraphMemory {
    /// List all entities in the graph (uses scan_entities for performance).
    pub fn list_entities(&self, limit: usize) -> Vec<GraphEntity> {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        match graph.scan_entities() {
            Ok(entities) => entities.iter().take(limit).map(entity_to_api).collect(),
            Err(e) => {
                warn!("Failed to scan entities: {e}");
                vec![]
            }
        }
    }

    /// List all facts/edges in the graph.
    pub fn list_facts(&self, limit: usize) -> Vec<GraphFact> {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        match graph.scan_edges() {
            Ok(edges) => edges.iter().take(limit).map(edge_to_api).collect(),
            Err(e) => {
                warn!("Failed to scan edges: {e}");
                vec![]
            }
        }
    }

    /// Get a single entity with its facts and neighbors.
    pub fn get_entity_detail(&self, id: u64) -> Option<(GraphEntity, Vec<GraphFact>, Vec<u64>)> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let entity = graph.get_entity(hora_graph_core::EntityId(id)).ok()??;
        let ge = entity_to_api(&entity);
        let facts: Vec<GraphFact> = graph
            .get_entity_facts(hora_graph_core::EntityId(id))
            .unwrap_or_default()
            .iter()
            .map(edge_to_api)
            .collect();
        let neighbors: Vec<u64> = graph
            .neighbors(hora_graph_core::EntityId(id))
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.0)
            .collect();
        Some((ge, facts, neighbors))
    }

    /// Delete an entity and all its connected edges from the graph.
    pub fn delete_entity(&self, id: u64) -> Result<(), String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        graph
            .delete_entity(EntityId(id))
            .map_err(|e| format!("Failed to delete entity: {e}"))
    }

    /// Invalidate a fact (soft-delete with bi-temporal invalid_at).
    pub fn invalidate_fact(&self, id: u64) -> Result<(), String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        graph
            .invalidate_fact(hora_graph_core::EdgeId(id))
            .map_err(|e| format!("Failed to invalidate fact: {e}"))
    }

    /// Search the graph by text query (BM25 + hybrid).
    pub fn search_graph(&self, query: &str, top_k: usize) -> Vec<GraphSearchHit> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let opts = SearchOpts {
            top_k,
            ..SearchOpts::default()
        };
        let hits = match graph.search(Some(query), None, opts) {
            Ok(h) => h,
            Err(_) => return vec![],
        };
        hits.into_iter()
            .filter_map(|hit| {
                let entity = graph.get_entity(hit.entity_id).ok()??;
                let content = entity
                    .properties
                    .get("content")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| entity.name.clone());
                let snippet = if content.len() > 200 {
                    let mut end = 200;
                    while end > 0 && !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &content[..end])
                } else {
                    content
                };
                Some(GraphSearchHit {
                    entity_id: hit.entity_id.0,
                    name: entity.name.clone(),
                    entity_type: entity.entity_type.clone(),
                    score: hit.score,
                    snippet,
                })
            })
            .collect()
    }

    /// Add a documentation entity to the graph.
    pub fn add_doc_entity(
        &self,
        entity_type: &str,
        name: &str,
        content: &str,
        tags: &[&str],
    ) -> Result<u64, String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let mut props = hora_graph_core::Properties::new();
        props.insert("content".into(), PropertyValue::String(content.into()));
        props.insert("tags".into(), PropertyValue::String(tags.join(",")));
        props.insert("source".into(), PropertyValue::String("system_doc".into()));
        let id = graph
            .add_entity(entity_type, name, Some(props), None)
            .map_err(|e| format!("Failed to add doc entity: {e}"))?;
        Ok(id.0)
    }

    /// Add a fact/relation between two entities.
    pub fn add_doc_fact(
        &self,
        source_id: u64,
        target_id: u64,
        relation: &str,
        description: &str,
    ) -> Result<u64, String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let id = graph
            .add_fact(
                hora_graph_core::EntityId(source_id),
                hora_graph_core::EntityId(target_id),
                relation,
                description,
                None, // default confidence
            )
            .map_err(|e| format!("Failed to add doc fact: {e}"))?;
        Ok(id.0)
    }

    /// Run a dream cycle (consolidation, linking, replay).
    pub fn dream_cycle(&self) -> Result<serde_json::Value, String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let config = hora_graph_core::DreamCycleConfig::default();
        let stats = graph
            .dream_cycle(&config)
            .map_err(|e| format!("Dream cycle failed: {e}"))?;
        Ok(serde_json::json!({
            "entities_downscaled": stats.entities_downscaled,
            "episodes_replayed": stats.replay.episodes_replayed,
            "entities_reactivated": stats.replay.entities_reactivated,
            "facts_created": stats.cls.facts_created,
            "links_created": stats.linking.links_created,
            "dark_nodes_marked": stats.dark_nodes_marked,
            "gc_deleted": stats.gc_deleted,
        }))
    }

    /// Get extended stats.
    pub fn extended_stats(&self) -> serde_json::Value {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let base = match graph.stats() {
            Ok(s) => s,
            Err(_) => {
                return serde_json::json!({"entities":0,"edges":0,"episodes":0,"dark_nodes":0})
            }
        };
        let dark = graph.dark_nodes().len();
        serde_json::json!({
            "entities": base.entities,
            "edges": base.edges,
            "episodes": base.episodes,
            "dark_nodes": dark,
        })
    }

    /// Get activation level for an entity.
    pub fn get_activation(&self, id: u64) -> Option<f64> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        graph.get_activation(hora_graph_core::EntityId(id))
    }

    /// Get memory phase for an entity.
    pub fn get_memory_phase(&self, id: u64) -> String {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        match graph.get_memory_phase(hora_graph_core::EntityId(id)) {
            Some(phase) => format!("{:?}", phase),
            None => "unknown".to_string(),
        }
    }
}

// ── Neural heartbeat (consciousness pulse) ──────────────────────────────────

/// Type of emergent thought detected by the neural pulse.
#[derive(Debug, Clone, serde::Serialize)]
pub enum ThoughtType {
    /// Two previously unconnected clusters activated together.
    Insight,
    /// FSRS says this entity needs review / is fading.
    Reminder,
    /// A recent fact contradicts an older one.
    Anomaly,
    /// Recurring pattern detected (CLS candidate).
    Pattern,
}

/// An emergent thought produced by the neural heartbeat.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmergentThought {
    /// Entities that triggered this thought.
    pub trigger_entities: Vec<u64>,
    /// Names of the trigger entities.
    pub trigger_names: Vec<String>,
    /// Strength of the signal.
    pub activation_score: f64,
    /// What kind of thought emerged.
    pub thought_type: ThoughtType,
    /// Human-readable summary.
    pub summary: String,
    /// Creation timestamp (millis since epoch) — used to discard stale thoughts.
    pub created_at: i64,
}

impl GraphMemory {
    /// Run a neural pulse: spreading activation from recent entities,
    /// detect salient nodes, produce emergent thoughts.
    ///
    /// This is the core of the consciousness loop — called every 30-60s
    /// by the background heartbeat. Pure graph computation, no LLM call.
    /// Multi-stage neural pulse inspired by brain architecture:
    /// S1 (Working Memory): spreading from recent entities, degree-normalized
    /// S2 (Episodic): scan graph for entities accessed in last 7 days, boost by recency
    /// S4 (Réminiscence): find dormant entities with old high-degree connections
    /// Results are merged with cross-stage boosting (GWT-inspired).
    pub fn neural_pulse(&self, saillance_threshold: f64) -> Vec<EmergentThought> {
        let recent: Vec<EntityId> = {
            let buf = self.recent_entities.lock().unwrap();
            buf.iter().copied().collect()
        };

        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let now_ms = chrono::Utc::now().timestamp_millis();

        // ── S1: Working Memory (spreading from recent, <2h) ──────────
        let mut s1_candidates: Vec<(EntityId, f64, &str)> = Vec::new();
        if !recent.is_empty() {
            let mut seen = std::collections::HashSet::new();
            let sources: Vec<(EntityId, f64)> = recent
                .iter()
                .filter(|id| seen.insert(**id))
                .map(|&id| (id, 1.0))
                .collect();

            let params = SpreadingParams {
                max_depth: 2,
                ..SpreadingParams::default()
            };
            if let Ok(activations) = graph.spread_activation(&sources, &params) {
                for (eid, score) in activations {
                    if score < saillance_threshold || seen.contains(&eid) {
                        continue;
                    }
                    let degree = graph.get_entity_facts(eid).map(|f| f.len()).unwrap_or(0);
                    // Soft penalty: high-degree entities still participate but score is normalized
                    let degree_factor = 1.0 / (1.0 + (degree as f64).ln().max(0.0));
                    let adj = score * degree_factor;
                    if adj > 0.1 {
                        s1_candidates.push((eid, adj, "S1"));
                    }
                }
            }
            s1_candidates
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            s1_candidates.truncate(10);
        }

        // ── S2: Episodic (<7 days, concept entities with recent facts) ──
        let mut s2_candidates: Vec<(EntityId, f64, &str)> = Vec::new();
        let seven_days_ms = 7 * 24 * 3_600_000_i64;
        if let Ok(entities) = graph.scan_entities() {
            for e in &entities {
                if !Self::is_thought_worthy_type(&e.entity_type) || e.name.len() <= 3 {
                    continue;
                }
                let age_ms = now_ms - e.created_at;
                if age_ms > seven_days_ms {
                    continue;
                }

                let degree = graph.get_entity_facts(e.id).map(|f| f.len()).unwrap_or(0);
                if degree == 0 || degree > 60 {
                    continue;
                }

                // Score: more connections + more recent = higher
                let recency = 1.0 - (age_ms as f64 / seven_days_ms as f64);
                let score = (degree as f64).sqrt() * recency;
                if score > 0.5 {
                    s2_candidates.push((e.id, score, "S2"));
                }
            }
            s2_candidates
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            s2_candidates.truncate(5);
        }

        // ── S4: Réminiscence (dormant >14 days, once active) ──────────
        let mut s4_candidates: Vec<(EntityId, f64, &str)> = Vec::new();
        let fourteen_days_ms = 14 * 24 * 3_600_000_i64;
        if let Ok(entities) = graph.scan_entities() {
            for e in &entities {
                if !Self::is_thought_worthy_type(&e.entity_type) || e.name.len() <= 3 {
                    continue;
                }
                let age_ms = now_ms - e.created_at;
                if age_ms < fourteen_days_ms {
                    continue;
                }

                let degree = graph.get_entity_facts(e.id).map(|f| f.len()).unwrap_or(0);
                if !(3..=80).contains(&degree) {
                    continue;
                }

                // Dormant but once-connected = interesting
                let dormancy_days = age_ms as f64 / 86_400_000.0;
                let score = (degree as f64).ln() * (1.0 - (-0.05 * dormancy_days).exp());
                if score > 0.3 {
                    s4_candidates.push((e.id, score, "S4"));
                }
            }
            s4_candidates
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            s4_candidates.truncate(2); // max 2 réminiscences
        }

        // ── S3: Semantic co-occurrence (entities that appear together across episodes) ──
        let mut s3_candidates: Vec<(EntityId, f64, &str)> = Vec::new();
        let fourteen_days_ago = now_ms - 14 * 24 * 3_600_000;
        if let Ok(episodes) = graph.get_episodes(None, None, None, Some(fourteen_days_ago)) {
            // Build co-occurrence counts: how often do two concept entities share an episode?
            let mut cooccurrence: HashMap<(u64, u64), u32> = HashMap::new();

            for ep in &episodes {
                let worthy: Vec<u64> = ep
                    .entity_ids
                    .iter()
                    .filter(|eid| {
                        graph.get_entity(**eid).ok().flatten().is_some_and(|e| {
                            Self::is_thought_worthy_type(&e.entity_type) && e.name.len() > 3
                        })
                    })
                    .map(|eid| eid.0)
                    .collect();

                // Count pairwise co-occurrences (sorted to avoid duplicates)
                for i in 0..worthy.len() {
                    for j in (i + 1)..worthy.len() {
                        let pair = if worthy[i] < worthy[j] {
                            (worthy[i], worthy[j])
                        } else {
                            (worthy[j], worthy[i])
                        };
                        *cooccurrence.entry(pair).or_default() += 1;
                    }
                }
            }

            // Find entities with high co-occurrence across DIFFERENT episodes
            // Score = co-occurrence count × average confidence of their shared facts
            let mut entity_scores: HashMap<u64, f64> = HashMap::new();
            for ((a, b), count) in &cooccurrence {
                if *count < 2 {
                    continue;
                } // need at least 2 shared episodes
                let score = (*count as f64).sqrt();
                *entity_scores.entry(*a).or_default() += score;
                *entity_scores.entry(*b).or_default() += score;
            }

            let mut scored: Vec<_> = entity_scores.into_iter().collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (eid, score) in scored.into_iter().take(5) {
                if score > 1.0 {
                    s3_candidates.push((EntityId(eid), score, "S3"));
                }
            }
        }

        // ── S5: GWT Integration (merge + cross-stage boost) ──────────
        let mut merged: Vec<(EntityId, f64, &str)> = Vec::new();
        merged.extend(s1_candidates.iter().cloned());
        merged.extend(s2_candidates.iter().cloned());
        merged.extend(s3_candidates.iter().cloned());
        merged.extend(s4_candidates.iter().cloned());

        // Cross-stage boost: entity in 2+ stages → ×1.5
        let mut entity_stage_count: HashMap<u64, usize> = HashMap::new();
        for (eid, _, _) in &merged {
            *entity_stage_count.entry(eid.0).or_default() += 1;
        }
        for (eid, score, _) in &mut merged {
            let count = entity_stage_count.get(&eid.0).copied().unwrap_or(1);
            if count > 1 {
                *score *= 1.0 + 0.5 * (count as f64 - 1.0);
            }
        }

        // Deduplicate by entity ID (keep highest score)
        let mut best: HashMap<u64, (EntityId, f64, &str)> = HashMap::new();
        for (eid, score, stage) in &merged {
            let entry = best.entry(eid.0).or_insert((*eid, 0.0, stage));
            if *score > entry.1 {
                *entry = (*eid, *score, stage);
            }
        }
        let mut final_candidates: Vec<_> = best.into_values().collect();
        final_candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        final_candidates.truncate(8);

        if !final_candidates.is_empty() {
            info!(
                "Neural pulse: S1={} S2={} S3={} S4={} → {} merged, top: {:.3}",
                s1_candidates.len(),
                s2_candidates.len(),
                s3_candidates.len(),
                s4_candidates.len(),
                final_candidates.len(),
                final_candidates.first().map_or(0.0, |(_, s, _)| *s),
            );
        }

        // ── Resolve content and build thoughts ───────────────────────
        let mut thoughts = Vec::new();
        let mut seen_content: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (eid, score, stage) in &final_candidates {
            let entity = match graph.get_entity(*eid) {
                Ok(Some(e)) => e,
                _ => continue,
            };
            if !Self::is_thought_worthy_type(&entity.entity_type) || entity.name.len() <= 3 {
                continue;
            }

            let context = Self::resolve_content(&mut graph, &entity);
            if context.is_empty()
                || context.contains("@1")
                || context.starts_with("**NO_REPLY**")
                || context.starts_with("[CRON")
                || context.starts_with("_event::")
                || context.starts_with("_sys::")
            {
                continue;
            }

            // Deduplicate by content (different entities can resolve to the same turn)
            let content_key = truncate_utf8(&context, 80).to_string();
            if seen_content.contains(&content_key) {
                continue;
            }
            seen_content.insert(content_key);

            let thought_type = match *stage {
                "S3" => ThoughtType::Pattern,
                "S4" => ThoughtType::Reminder,
                _ => ThoughtType::Insight,
            };

            info!("  [{}] thought: {}", stage, truncate_utf8(&context, 60));

            thoughts.push(EmergentThought {
                trigger_entities: vec![eid.0],
                trigger_names: vec![entity.name.clone()],
                activation_score: *score,
                thought_type,
                summary: truncate_utf8(&context, 100).to_string(),
                created_at: chrono::Utc::now().timestamp_millis(),
            });
        }

        thoughts
    }

    /// Check if entity type is semantically meaningful for thoughts.
    fn is_thought_worthy_type(entity_type: &str) -> bool {
        matches!(
            entity_type,
            "concept"
                | "person"
                | "project"
                | "tool"
                | "technology"
                | "location"
                | "event"
                | "organization"
                | "date"
        )
    }

    /// Resolve human-readable content for an entity by traversing its facts.
    /// Prefers user turns > validated assistant turns > fact descriptions.
    fn resolve_content(graph: &mut HoraCore, entity: &hora_graph_core::Entity) -> String {
        let facts = match graph.get_entity_facts(entity.id) {
            Ok(f) => f,
            Err(_) => return String::new(),
        };

        let mut relevant: Vec<_> = facts.iter().filter(|f| f.confidence > 0.3).collect();
        relevant.sort_by(|a, b| {
            b.created_at.cmp(&a.created_at).then(
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        let mut best_assistant: Option<String> = None;
        for fact in relevant {
            let other_id = if fact.source == entity.id {
                fact.target
            } else {
                fact.source
            };
            if let Ok(Some(other)) = graph.get_entity(other_id) {
                if other.entity_type.starts_with("_conv::") {
                    let role = other
                        .properties
                        .get("role")
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or("unknown");
                    let content = other.name.clone();
                    if content.is_empty() || content == entity.name {
                        continue;
                    }
                    if role == "user" {
                        return content;
                    }
                    let feedback = other
                        .properties
                        .get("feedback_score")
                        .and_then(|v| match v {
                            PropertyValue::Float(f) => Some(*f as f32),
                            _ => None,
                        })
                        .unwrap_or(0.0);
                    if best_assistant.is_none() && feedback >= -0.2 {
                        best_assistant = Some(content);
                    }
                    continue;
                }
                if other.name != entity.name && !other.name.is_empty() {
                    let desc = if !fact.description.is_empty()
                        && fact.description != entity.name
                        && fact.description != other.name
                    {
                        fact.description.clone()
                    } else {
                        other.name.clone()
                    };
                    return desc;
                }
            }
            if !fact.description.is_empty() && fact.description != entity.name {
                return fact.description.clone();
            }
        }
        best_assistant.unwrap_or_default()
    }

    /// Drain the recent entity buffer (after pulse processing).
    pub fn drain_recent_entities(&self) {
        let mut buf = self.recent_entities.lock().unwrap();
        buf.clear();
    }

    /// Filter emergent thoughts and route them: Act (surface now), Queue, or Discard.
    ///
    /// Criteria:
    /// - Score < 0.3 → Discard (noise)
    /// - Cooldown active (< 120s since last surface) → Queue
    /// - Anomaly type → always Act (urgent)
    /// - Score >= 0.8 → Act (strong signal)
    /// - Otherwise → Queue (accumulate until agent asks)
    pub fn filter_thoughts(&self, thoughts: Vec<EmergentThought>) -> Vec<EmergentThought> {
        if thoughts.is_empty() {
            return vec![];
        }

        let now = Instant::now();
        let cooldown_active = {
            let last = self.last_surfaced.lock().unwrap();
            last.is_some_and(|t| now.duration_since(t).as_secs() < THOUGHT_COOLDOWN_SECS)
        };

        let mut to_surface = Vec::new();
        let mut queue = self.thought_queue.lock().unwrap();

        for thought in thoughts {
            // Discard: too weak
            if thought.activation_score < 0.3 {
                continue;
            }

            // Anomalies always surface (urgent)
            let is_urgent = matches!(thought.thought_type, ThoughtType::Anomaly);

            // Strong signal or urgent → surface if cooldown allows
            let should_act = is_urgent || (!cooldown_active && thought.activation_score >= 0.8);

            if should_act {
                to_surface.push(thought);
            } else {
                // Queue for later consumption
                if queue.len() >= THOUGHT_QUEUE_CAPACITY {
                    queue.pop_front();
                }
                queue.push_back(thought);
            }
        }

        if !to_surface.is_empty() {
            *self.last_surfaced.lock().unwrap() = Some(now);
        }

        to_surface
    }

    /// Consume all queued thoughts (called when agent starts a new interaction).
    /// Returns queued thoughts sorted by score descending, limited to `max`.
    pub fn consume_queued_thoughts(&self, max: usize) -> Vec<EmergentThought> {
        let mut queue = self.thought_queue.lock().unwrap();
        let mut thoughts: Vec<EmergentThought> = queue.drain(..).collect();
        let now = chrono::Utc::now().timestamp_millis();
        thoughts.retain(|t| now - t.created_at < 600_000); // discard thoughts older than 10 min
        thoughts.sort_by(|a, b| {
            b.activation_score
                .partial_cmp(&a.activation_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        thoughts.truncate(max);
        thoughts
    }

    /// Peek at queued thought count (for status/dashboard).
    pub fn queued_thought_count(&self) -> usize {
        self.thought_queue.lock().unwrap().len()
    }
}

// ── Telegram Digest (batch, not spam) ───────────────────────────────────────

impl GraphMemory {
    /// Buffer a thought for the hourly Telegram digest (not sent immediately).
    pub fn buffer_thought_for_digest(&self, thought: &EmergentThought) {
        let mut digest = self.thought_digest.lock().unwrap();
        if digest.iter().any(|t| t.summary == thought.summary) {
            return;
        }
        digest.push(thought.clone());
    }

    /// Preview the digest without draining the buffer.
    /// Returns None if nothing interesting to report.
    pub fn peek_telegram_digest(&self) -> Option<String> {
        let digest = self.thought_digest.lock().unwrap();
        if digest.is_empty() {
            return None;
        }
        self.format_digest(&digest)
    }

    /// Drain buffered thoughts and format a human-readable Telegram digest.
    /// Returns None if no thoughts accumulated or nothing interesting to report.
    pub fn flush_telegram_digest(&self) -> Option<String> {
        let mut digest = self.thought_digest.lock().unwrap();
        if digest.is_empty() {
            return None;
        }

        let thoughts: Vec<EmergentThought> = digest.drain(..).collect();
        self.format_digest(&thoughts)
    }

    /// Format thoughts into a human-readable digest message.
    fn format_digest(&self, thoughts: &[EmergentThought]) -> Option<String> {
        // Group by type
        let mut insights: Vec<&EmergentThought> = Vec::new();
        let mut patterns: Vec<&EmergentThought> = Vec::new();
        let mut anomalies: Vec<&EmergentThought> = Vec::new();

        for t in thoughts {
            match t.thought_type {
                ThoughtType::Insight | ThoughtType::Reminder => insights.push(t),
                ThoughtType::Pattern => patterns.push(t),
                ThoughtType::Anomaly => anomalies.push(t),
            }
        }

        // Decide if digest is worth sending:
        // - Any anomaly → always send
        // - At least 1 insight/pattern with score > 0.6 → send
        // - Only low-score noise → skip
        let has_anomaly = !anomalies.is_empty();
        let has_strong_insight = insights.iter().any(|t| t.activation_score > 0.6)
            || patterns.iter().any(|t| t.activation_score > 0.6);

        if !has_anomaly && !has_strong_insight {
            return None;
        }

        // Collect data before locking graph
        let mood = self.get_mood();
        let (accuracy, correct, total) = self.prediction_accuracy();
        let user_state = self.get_user_state();

        // Entity count from graph (separate lock, prediction_accuracy already released it)
        let entity_count = {
            let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
            graph.scan_entities().map(|e| e.len()).unwrap_or(0)
        };

        // Header: weekday + time in French
        let now = chrono::Local::now();
        let weekday_fr = match now.weekday() {
            chrono::Weekday::Mon => "lundi",
            chrono::Weekday::Tue => "mardi",
            chrono::Weekday::Wed => "mercredi",
            chrono::Weekday::Thu => "jeudi",
            chrono::Weekday::Fri => "vendredi",
            chrono::Weekday::Sat => "samedi",
            chrono::Weekday::Sun => "dimanche",
        };
        let header = format!(
            "🧠 Captain — {} {}h{:02}",
            weekday_fr,
            now.format("%H"),
            now.format("%M")
        );

        // Duration: oldest thought's created_at vs now
        let duration_str = if let Some(oldest) = thoughts.iter().map(|t| t.created_at).min() {
            let elapsed_secs = (chrono::Utc::now().timestamp_millis() - oldest) / 1000;
            let hours = elapsed_secs / 3600;
            let mins = (elapsed_secs % 3600) / 60;
            if hours > 0 {
                format!("{}h", hours)
            } else if mins > 0 {
                format!("{}min", mins)
            } else {
                "quelques instants".to_string()
            }
        } else {
            "quelques heures".to_string()
        };

        // Mood phrase
        let mood_phrase = if mood.confidence > 0.7 {
            format!("confiant (streak +{})", mood.streak)
        } else if mood.confidence > 0.3 {
            "en observation".to_string()
        } else {
            "prudent (erreurs recentes)".to_string()
        };

        // Build insight lines (max 3, deduplicated)
        let mut seen = std::collections::HashSet::new();
        let mut insight_lines: Vec<String> = Vec::new();

        // Combine insights + patterns, sorted by activation score descending
        let mut candidates: Vec<&EmergentThought> = insights
            .iter()
            .chain(patterns.iter())
            .filter(|t| t.activation_score > 0.5)
            .copied()
            .collect();
        candidates.sort_by(|a, b| {
            b.activation_score
                .partial_cmp(&a.activation_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for t in candidates {
            if insight_lines.len() >= 3 {
                break;
            }
            if !seen.insert(t.summary.clone()) {
                continue;
            }

            let line = humanize_thought(&t.summary, &t.trigger_names);
            insight_lines.push(truncate_utf8(&line, 80).to_string());
        }

        // Prediction phrase
        let prediction_phrase = if total < 5 {
            "en apprentissage".to_string()
        } else if accuracy > 0.6 {
            format!("{}/{} correctes ({:.0}%)", correct, total, accuracy * 100.0)
        } else {
            format!("en progression ({}/{})", correct, total)
        };

        // Assemble message
        let mut msg = format!(
            "{}\n\nJe tourne en arriere-plan depuis {}.\nEtat : {}\n",
            header, duration_str, mood_phrase
        );

        // Anomalies (urgent — keep prominent)
        if !anomalies.is_empty() {
            msg.push_str("\n⚠️ A verifier :\n");
            for t in &anomalies {
                msg.push_str(&format!("  ⚠️ {}\n", truncate_utf8(&t.summary, 100)));
            }
        }

        // Insights
        if !insight_lines.is_empty() {
            msg.push_str("\nCe que j'ai remarque :\n");
            for line in &insight_lines {
                msg.push_str(&format!("  → {}\n", line));
            }
        }

        msg.push_str(&format!(
            "\nMemoire : {} entites · {} echanges aujourd'hui\nPredictions : {}",
            entity_count, user_state.interaction_count, prediction_phrase
        ));

        Some(msg)
    }
}

// ── Self-model & Reflection (consciousness C.5) ─────────────────────────────

impl GraphMemory {
    /// Seed the self-model: register system capabilities as graph entities.
    /// Called once at kernel boot.
    pub fn seed_self_model(&self, tools: &[&str], hands: &[&str], agents: &[&str]) {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());

        let system_id = match graph.add_entity("_self::system", "captain", None, None) {
            Ok(id) => id,
            Err(_) => return,
        };

        for name in tools {
            if let Ok(id) = graph.add_entity("_self::tool", name, None, None) {
                let _ = graph.add_fact(system_id, id, "has_capability", name, None);
            }
        }
        for name in hands {
            if let Ok(id) = graph.add_entity("_self::hand", name, None, None) {
                let _ = graph.add_fact(system_id, id, "has_hand", name, None);
            }
        }
        for name in agents {
            if let Ok(id) = graph.add_entity("_self::agent", name, None, None) {
                let _ = graph.add_fact(system_id, id, "runs_agent", name, None);
            }
        }

        debug!(
            "Self-model seeded: {} tools, {} hands, {} agents",
            tools.len(),
            hands.len(),
            agents.len()
        );
    }

    /// Seed tool usage rules into the graph so agents know the exact format.
    /// Called once at kernel boot, idempotent.
    pub fn seed_tool_rules(&self) {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());

        let rules: &[(&str, &str)] = &[
            ("memory_store", "Stocker une info: memory({\"action\":\"store\",\"key\":\"user_timezone\",\"value\":\"Europe/Paris\"})"),
            ("memory_recall", "Chercher une info: memory({\"action\":\"recall\",\"key\":\"user_timezone\"})"),
            ("cron_create", "Rappel one-shot: cron({\"action\":\"create\",\"name\":\"rappel_X\",\"schedule\":{\"kind\":\"at\",\"at\":\"2026-04-01T09:00:00+02:00\"},\"action_config\":{\"kind\":\"agent_turn\",\"message\":\"Rappel: faire X\"},\"delivery\":{\"kind\":\"channel\",\"channel\":\"telegram\"},\"one_shot\":true})"),
            ("cron_create_recurring", "Cron récurrent: cron({\"action\":\"create\",\"name\":\"daily_X\",\"schedule\":{\"kind\":\"cron\",\"expr\":\"0 9 * * *\",\"tz\":\"Europe/Paris\"},\"action_config\":{\"kind\":\"agent_turn\",\"message\":\"Exécuter X\"},\"delivery\":{\"kind\":\"channel\",\"channel\":\"telegram\"}})"),
            ("shell_exec", "Commande système: exec({\"action\":\"command\",\"command\":\"uptime\"})"),
            ("agent_list", "Lister agents: agent({\"action\":\"list\"})"),
            ("channel_send", "Envoyer Telegram: channel({\"action\":\"send\",\"channel\":\"telegram\",\"to\":\"default\",\"message\":\"Le contenu\"})"),
            ("web_search", "Recherche web: web({\"action\":\"search\",\"query\":\"météo Lyon demain\"})"),
            ("web_fetch", "Lire une URL: web({\"action\":\"fetch\",\"url\":\"https://example.com\"})"),
            ("file_read", "Lire fichier: file({\"action\":\"read\",\"path\":\"/chemin/fichier.txt\"})"),
            ("file_write", "Écrire fichier: file({\"action\":\"write\",\"path\":\"/chemin/fichier.txt\",\"content\":\"contenu\"})"),
            ("cron_list", "Lister crons: cron({\"action\":\"list\"})"),
            ("hand_list", "Lister hands: hand({\"action\":\"list\"})"),
            ("hand_activate", "Activer hand: hand({\"action\":\"activate\",\"hand_id\":\"family-hand\"})"),
            ("ask_user", "Demander confirmation: ask_user({\"question\":\"Tu confirmes ?\",\"options\":[\"Oui\",\"Non\"]})"),
        ];

        for (name, rule) in rules {
            let mut props = hora_graph_core::Properties::new();
            props.insert("usage_rule".into(), PropertyValue::String(rule.to_string()));
            let _ = graph.add_entity("_self::tool_rule", name, Some(props), None);
        }

        debug!("Tool usage rules seeded: {} rules", rules.len());
    }

    /// Get the usage rule for a specific tool (from the graph).
    /// Returns the format example if found, empty string otherwise.
    pub fn get_tool_rule(&self, tool_name: &str) -> String {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(entities) = graph.scan_entities() {
            for e in &entities {
                if e.entity_type == "_self::tool_rule" && e.name == tool_name {
                    return e
                        .properties
                        .get("usage_rule")
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                }
            }
        }
        String::new()
    }

    /// Record a post-interaction reflection.
    /// Creates a reflection entity + links to tools used + episode for dream cycle.
    pub fn reflect(
        &self,
        agent_name: &str,
        tools_used: &[&str],
        success: bool,
        turn_count: u32,
    ) -> Result<(), String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());

        let outcome = if success { "success" } else { "failure" };
        let mut props = hora_graph_core::Properties::new();
        props.insert("agent".into(), PropertyValue::String(agent_name.into()));
        props.insert("outcome".into(), PropertyValue::String(outcome.into()));
        props.insert(
            "tools_used".into(),
            PropertyValue::String(tools_used.join(",")),
        );
        props.insert("turn_count".into(), PropertyValue::Int(turn_count as i64));

        let summary = format!(
            "reflection:{}:{} ({} turns, {} tools)",
            agent_name,
            outcome,
            turn_count,
            tools_used.len()
        );

        let reflection_id = graph
            .add_entity("_self::reflection", &summary, Some(props), None)
            .map_err(|e| format!("Failed to create reflection: {e}"))?;

        let mut entity_ids = vec![reflection_id];
        let mut fact_ids = Vec::new();

        for tool_name in tools_used {
            if let Ok(tool_id) = graph.add_entity("_self::tool", tool_name, None, None) {
                entity_ids.push(tool_id);
                let relation = if success {
                    "succeeded_with"
                } else {
                    "failed_with"
                };
                if let Ok(fid) = graph.add_fact(reflection_id, tool_id, relation, tool_name, None) {
                    fact_ids.push(fid);
                }
            }
        }

        if let Ok(agent_id) = graph.add_entity("agent", agent_name, None, None) {
            entity_ids.push(agent_id);
            if let Ok(fid) =
                graph.add_fact(reflection_id, agent_id, "reflected_by", agent_name, None)
            {
                fact_ids.push(fid);
            }
        }

        let session_id = format!(
            "reflect:{}:{}",
            agent_name,
            chrono::Local::now().format("%Y-%m-%d")
        );
        let _ = graph.add_episode(EpisodeSource::Api, &session_id, &entity_ids, &fact_ids);

        debug!(
            "Reflection: {} — {} tools, outcome={}",
            agent_name,
            tools_used.len(),
            outcome
        );
        Ok(())
    }
}

// ── D.3: User State Modeling (theory of mind) ───────────────────────────────

/// Inferred user state from interaction patterns.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UserState {
    /// Message brevity: short messages = rushed, long = exploring.
    pub pace: f64, // 0.0 = rushed, 1.0 = deliberate
    /// Error correction frequency: high = frustrated.
    pub frustration: f64, // 0.0 = calm, 1.0 = frustrated
    /// Interaction count in current session.
    pub interaction_count: u32,
    /// Inferred mode: "architect" (creative), "debug" (focused), "explore" (open).
    pub mode: String,
}

impl Default for UserState {
    fn default() -> Self {
        Self {
            pace: 0.5,
            frustration: 0.0,
            interaction_count: 0,
            mode: "explore".into(),
        }
    }
}

impl GraphMemory {
    /// Update user state based on the latest message.
    /// Analyzes message length, punctuation, corrections, and time patterns.
    pub fn update_user_state(&self, content: &str) -> UserState {
        let mut state = self.user_state.lock().unwrap();

        state.interaction_count += 1;

        // Pace: short messages (< 50 chars) = rushed
        let len = content.len();
        let pace_signal = if len < 30 {
            0.1
        } else if len < 100 {
            0.4
        } else if len < 300 {
            0.7
        } else {
            1.0
        };
        state.pace = state.pace * 0.7 + pace_signal * 0.3; // EMA smoothing

        // Frustration: detect correction markers
        let has_correction = content.contains("non")
            || content.contains("pas ça")
            || content.contains("j'ai dit")
            || content.contains("corrige")
            || content.contains("wrong")
            || content.contains("no not");
        let frust_signal = if has_correction { 0.8 } else { 0.0 };
        state.frustration = (state.frustration * 0.8 + frust_signal * 0.2).min(1.0);
        // Natural decay
        if !has_correction {
            state.frustration = (state.frustration - 0.05).max(0.0);
        }

        // Mode inference from content patterns
        let lower = content.to_lowercase();
        state.mode = if lower.contains("plan")
            || lower.contains("architect")
            || lower.contains("design")
            || lower.contains("comment faire")
        {
            "architect".into()
        } else if lower.contains("bug")
            || lower.contains("error")
            || lower.contains("fix")
            || lower.contains("debug")
            || lower.contains("crash")
        {
            "debug".into()
        } else {
            "explore".into()
        };

        state.clone()
    }

    /// Get current user state without updating.
    pub fn get_user_state(&self) -> UserState {
        self.user_state.lock().unwrap().clone()
    }

    /// Format user state for prompt injection.
    pub fn user_state_prompt(&self) -> String {
        let s = self.user_state.lock().unwrap();
        if s.interaction_count < 2 {
            return String::new();
        }
        let tone = if s.frustration > 0.5 {
            "L'utilisateur semble frustre — sois prudent, confirme avant d'agir"
        } else if s.pace < 0.3 {
            "L'utilisateur est presse — reponses courtes et actionnables"
        } else if s.mode == "architect" {
            "L'utilisateur est en mode planification — propose des approches structurees"
        } else if s.mode == "debug" {
            "L'utilisateur debug — concentre-toi sur la cause racine"
        } else {
            return String::new();
        };
        format!("[ETAT UTILISATEUR] {}", tone)
    }
}

// ── D.4: Prediction System ──────────────────────────────────────────────────

/// A prediction made by the system, tracked for accuracy.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Prediction {
    pub id: u64,
    pub what: String,
    pub confidence: f32,
    pub created_at: i64,
    pub verify_after: i64,
    pub outcome: Option<bool>,
}

impl GraphMemory {
    /// Record a prediction in the graph.
    pub fn predict(
        &self,
        what: &str,
        confidence: f32,
        verify_after_secs: u64,
    ) -> Result<u64, String> {
        let now = chrono::Utc::now().timestamp_millis();
        let verify_at = now + (verify_after_secs as i64 * 1000);

        let mut props = hora_graph_core::Properties::new();
        props.insert("confidence".into(), PropertyValue::Float(confidence as f64));
        props.insert("verify_after".into(), PropertyValue::Int(verify_at));
        props.insert("outcome".into(), PropertyValue::String("pending".into()));

        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let id = graph
            .add_entity("_self::prediction", what, Some(props), None)
            .map_err(|e| format!("Failed to record prediction: {e}"))?;
        Ok(id.0)
    }

    /// Resolve a prediction outcome (true = correct, false = wrong).
    pub fn resolve_prediction(&self, entity_id: u64, correct: bool) -> Result<(), String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let outcome = if correct { "correct" } else { "wrong" };
        graph
            .update_entity(
                EntityId(entity_id),
                hora_graph_core::EntityUpdate {
                    properties: Some({
                        let mut p = hora_graph_core::Properties::new();
                        p.insert("outcome".into(), PropertyValue::String(outcome.into()));
                        p
                    }),
                    ..Default::default()
                },
            )
            .map_err(|e| format!("Failed to resolve prediction: {e}"))
    }

    /// Calculate prediction accuracy (correct / total resolved).
    pub fn prediction_accuracy(&self) -> (f64, usize, usize) {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let entities = match graph.scan_entities() {
            Ok(e) => e,
            Err(_) => return (0.0, 0, 0),
        };
        let (mut correct, mut total) = (0usize, 0usize);
        for e in &entities {
            if e.entity_type != "_self::prediction" {
                continue;
            }
            let outcome = e.properties.get("outcome").and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            });
            match outcome {
                Some("correct") => {
                    correct += 1;
                    total += 1;
                }
                Some("wrong") => {
                    total += 1;
                }
                _ => {} // pending
            }
        }
        let accuracy = if total > 0 {
            correct as f64 / total as f64
        } else {
            0.0
        };
        (accuracy, correct, total)
    }
}

// ── D.5: System Mood (neuromodulators) ──────────────────────────────────────

/// System-wide mood/confidence signal.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemMood {
    /// Overall confidence (0.0 = defensive, 1.0 = assertive).
    pub confidence: f64,
    /// Consecutive successes (resets on failure).
    pub streak: i32,
    /// Error rate in last N interactions (rolling window).
    pub error_rate: f64,
    /// Prediction accuracy (from D.4).
    pub prediction_accuracy: f64,
}

impl Default for SystemMood {
    fn default() -> Self {
        Self {
            confidence: 0.5,
            streak: 0,
            error_rate: 0.0,
            prediction_accuracy: 0.0,
        }
    }
}

impl GraphMemory {
    /// Update system mood based on interaction outcome.
    pub fn update_mood(&self, success: bool) {
        let mut mood = self.system_mood.lock().unwrap();

        if success {
            mood.streak = mood.streak.saturating_add(1);
            mood.error_rate = (mood.error_rate * 0.9).max(0.0); // Decay
        } else {
            mood.streak = (-1i32).max(mood.streak.saturating_sub(3)); // Failures hit harder
            mood.error_rate = (mood.error_rate * 0.9 + 0.1).min(1.0);
        }

        // Confidence = weighted blend of streak, error rate, prediction accuracy
        let (acc, _, total) = self.prediction_accuracy();
        mood.prediction_accuracy = if total >= 3 { acc } else { 0.5 };

        let streak_signal = (mood.streak as f64 / 10.0).clamp(0.0, 1.0);
        let error_signal = 1.0 - mood.error_rate;
        mood.confidence =
            (streak_signal * 0.3 + error_signal * 0.4 + mood.prediction_accuracy * 0.3)
                .clamp(0.1, 0.95);
    }

    /// Get current system mood.
    pub fn get_mood(&self) -> SystemMood {
        self.system_mood.lock().unwrap().clone()
    }

    /// Format mood for prompt injection.
    pub fn mood_prompt(&self) -> String {
        let mood = self.system_mood.lock().unwrap();
        if mood.confidence > 0.7 {
            "[HUMEUR SYSTEME: confiant] Propose des solutions directement. Sois assertif.".into()
        } else if mood.confidence < 0.3 {
            "[HUMEUR SYSTEME: prudent] Demande confirmation avant les actions importantes. Verifie tes hypotheses.".into()
        } else {
            String::new() // Neutral — no special instruction
        }
    }
}

// ── D.6: Temporal Patterns (learned crons) ──────────────────────────────────

/// A detected temporal pattern in user behavior.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TemporalPattern {
    pub action: String,
    pub hour: u32,
    pub weekday: Option<u32>, // 0=Mon..6=Sun, None=daily
    pub occurrences: u32,
    pub last_seen: i64,
}

impl GraphMemory {
    /// Record a timestamped user action for temporal analysis.
    pub fn record_temporal_action(&self, action: &str) {
        let now = chrono::Local::now();
        let key = format!("{}:{}:{}", action, now.format("%u"), now.format("%H"));

        let mut counts = self.temporal_counts.lock().unwrap();
        let entry = counts.entry(key).or_insert((0u32, 0i64));
        entry.0 += 1;
        entry.1 = chrono::Utc::now().timestamp_millis();
    }

    /// Detect recurring patterns from accumulated temporal data.
    /// A pattern is significant if it occurred >= `min_occurrences` times.
    pub fn detect_patterns(&self, min_occurrences: u32) -> Vec<TemporalPattern> {
        let counts = self.temporal_counts.lock().unwrap();
        let mut patterns = Vec::new();

        for (key, &(count, last_seen)) in counts.iter() {
            if count < min_occurrences {
                continue;
            }
            let parts: Vec<&str> = key.split(':').collect();
            if parts.len() != 3 {
                continue;
            }
            let action = parts[0].to_string();
            let weekday = parts[1].parse::<u32>().ok();
            let hour = parts[2].parse::<u32>().unwrap_or(0);

            patterns.push(TemporalPattern {
                action,
                hour,
                weekday,
                occurrences: count,
                last_seen,
            });
        }

        patterns.sort_by_key(|p| std::cmp::Reverse(p.occurrences));
        patterns
    }

    /// Check if any pattern is due now (within +/- 30 min window).
    pub fn patterns_due_now(&self, min_occurrences: u32) -> Vec<TemporalPattern> {
        let now = chrono::Local::now();
        let current_hour = now.format("%H").to_string().parse::<u32>().unwrap_or(0);
        let current_weekday = now.format("%u").to_string().parse::<u32>().unwrap_or(1);

        self.detect_patterns(min_occurrences)
            .into_iter()
            .filter(|p| p.hour == current_hour && p.weekday.is_none_or(|wd| wd == current_weekday))
            .collect()
    }

    /// Format due patterns for prompt or notification.
    pub fn temporal_prompt(&self) -> String {
        let due = self.patterns_due_now(3);
        if due.is_empty() {
            return String::new();
        }
        let mut lines = vec!["[ACTIONS ANTICIPEES — basees sur les habitudes]".to_string()];
        for p in &due {
            let day = p.weekday.map_or("quotidien".to_string(), |d| {
                ["Lun", "Mar", "Mer", "Jeu", "Ven", "Sam", "Dim"]
                    .get(d.saturating_sub(1) as usize)
                    .unwrap_or(&"?")
                    .to_string()
            });
            lines.push(format!(
                "- {} (habituellement a {}h, {}, vu {}x)",
                p.action, p.hour, day, p.occurrences
            ));
        }
        lines.join("\n")
    }
}

// ── D.8: Internal Narration (explainability) ────────────────────────────────

impl GraphMemory {
    /// Generate a brief internal narration after an interaction.
    /// Summarizes what happened, what was learned, and what to watch.
    /// Returns a string stored as a `_self::narration` entity.
    pub fn narrate(
        &self,
        agent_name: &str,
        _user_message: &str,
        tools_used: &[&str],
        success: bool,
    ) -> Result<(), String> {
        let mood = self.get_mood();
        let user = self.get_user_state();

        let mut parts = Vec::new();
        parts.push(format!("Agent {} processed a request.", agent_name));

        if !tools_used.is_empty() {
            parts.push(format!("Used: {}.", tools_used.join(", ")));
        }

        if !success {
            parts.push("Outcome: failure — should investigate.".into());
        }

        if user.frustration > 0.5 {
            parts.push("User showed signs of frustration.".into());
        }

        if mood.confidence < 0.3 {
            parts.push("System confidence is low — being cautious.".into());
        }

        // Record temporal action for pattern learning
        let action_type = if tools_used.is_empty() {
            "chat"
        } else if tools_used.iter().any(|t| t.contains("exec")) {
            "execute"
        } else if tools_used.iter().any(|t| t.contains("read")) {
            "read"
        } else {
            "tool_use"
        };
        self.record_temporal_action(action_type);

        let narration = parts.join(" ");

        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let mut props = hora_graph_core::Properties::new();
        props.insert("agent".into(), PropertyValue::String(agent_name.into()));
        props.insert("mood".into(), PropertyValue::Float(mood.confidence));
        props.insert("user_pace".into(), PropertyValue::Float(user.pace));
        props.insert("user_mode".into(), PropertyValue::String(user.mode.clone()));

        let name = truncate_utf8(&narration, 200);
        let _ = graph.add_entity("_self::narration", name, Some(props), None);
        drop(graph);

        // Push to in-memory ring buffer for prompt injection (D.8).
        let mut buf = self
            .recent_narrations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if buf.len() == 5 {
            buf.pop_front();
        }
        buf.push_back(narration.clone());

        debug!("Narration: {}", narration);
        Ok(())
    }

    /// D.8: Format the last 3–5 narration summaries as a system prompt snippet.
    pub fn narration_prompt(&self) -> String {
        let buf = self
            .recent_narrations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if buf.is_empty() {
            return String::new();
        }
        let mut out = String::from("[SELF REFLECTION]\n");
        for entry in buf.iter() {
            out.push_str(&format!("- {}\n", entry));
        }
        out
    }
}

// ── D.7: Active Curiosity ────────────────────────────────────────────────────

/// A curiosity item — something the system wants to investigate.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CuriosityItem {
    pub topic: String,
    pub reason: String,
    pub priority: f64,
}

impl GraphMemory {
    /// Identify topics worth investigating based on graph state.
    /// Scans for: stale entities, tools never reflected on, unresolved predictions.
    pub fn curiosity_scan(&self) -> Vec<CuriosityItem> {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let entities = match graph.scan_entities() {
            Ok(e) => e,
            Err(_) => return vec![],
        };

        let mut items = Vec::new();

        // Find tools in self-model that have no reflections (never used)
        let tools: Vec<&str> = entities
            .iter()
            .filter(|e| e.entity_type == "_self::tool")
            .map(|e| e.name.as_str())
            .collect();
        let reflected_tools: std::collections::HashSet<String> = entities
            .iter()
            .filter(|e| e.entity_type == "_self::reflection")
            .filter_map(|e| {
                e.properties.get("tools_used").and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .flat_map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();

        for tool in &tools {
            if !reflected_tools.contains(*tool) && !tool.starts_with("_") {
                items.push(CuriosityItem {
                    topic: format!("tool:{}", tool),
                    reason: format!(
                        "Tool '{}' has never been used — explore its capabilities",
                        tool
                    ),
                    priority: 0.3,
                });
            }
        }

        // Find unresolved predictions past their verify_after deadline
        let now = chrono::Utc::now().timestamp_millis();
        for e in &entities {
            if e.entity_type != "_self::prediction" {
                continue;
            }
            let outcome = e.properties.get("outcome").and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            });
            if outcome != Some("pending") {
                continue;
            }
            let verify_after = e
                .properties
                .get("verify_after")
                .and_then(|v| match v {
                    PropertyValue::Int(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(i64::MAX);
            if now > verify_after {
                items.push(CuriosityItem {
                    topic: format!("prediction:{}", e.id.0),
                    reason: format!("Prediction '{}' is overdue for verification", e.name),
                    priority: 0.7,
                });
            }
        }

        items.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items.truncate(5);
        items
    }
}

// ── D.9: Neuromodulators ────────────────────────────────────────────────────

/// Global neuromodulators that influence all cognitive thresholds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NeuroModulators {
    /// Curiosity/exploration drive (0-1). High = explore new, low = exploit known.
    pub dopamine: f64,
    /// Stability/patience (0-1). High = steady, low = volatile.
    pub serotonin: f64,
    /// Alertness/urgency (0-1). High = hyper-vigilant, low = relaxed.
    pub norepinephrine: f64,
    /// Stress level (0-1). High = defensive mode, low = confident.
    pub cortisol: f64,
}

impl Default for NeuroModulators {
    fn default() -> Self {
        Self {
            dopamine: 0.5,
            serotonin: 0.7,
            norepinephrine: 0.3,
            cortisol: 0.1,
        }
    }
}

impl GraphMemory {
    /// Recompute neuromodulators based on current system state.
    /// Called after each interaction or on heartbeat.
    pub fn recompute_neuromodulators(&self) -> NeuroModulators {
        let mood = self.get_mood();
        let user = self.get_user_state();
        let curiosity_items = self.curiosity_scan().len();

        let mut nm = self.neuromodulators.lock().unwrap();

        // Dopamine: driven by prediction success + unexplored territory
        nm.dopamine = (mood.prediction_accuracy * 0.5
            + (curiosity_items as f64 / 5.0).min(1.0) * 0.5)
            .clamp(0.1, 0.9);

        // Serotonin: inversely proportional to mood volatility
        nm.serotonin = if mood.streak.abs() < 3 { 0.7 } else { 0.4 };

        // Norepinephrine: spikes on user frustration or system errors
        nm.norepinephrine = (user.frustration * 0.6 + mood.error_rate * 0.4).clamp(0.0, 1.0);

        // Cortisol: accumulates with sustained errors, decays with successes
        nm.cortisol = if mood.confidence < 0.3 {
            (nm.cortisol + 0.1).min(0.9)
        } else {
            (nm.cortisol - 0.05).max(0.0)
        };

        nm.clone()
    }

    /// Get current neuromodulators.
    pub fn get_neuromodulators(&self) -> NeuroModulators {
        self.neuromodulators.lock().unwrap().clone()
    }

    /// Adjust the heartbeat saillance threshold based on neuromodulators.
    /// Dopamine (curiosity) + norepinephrine (alertness) = lower threshold (explore more).
    /// Serotonin (stability) = higher threshold (filter noise).
    pub fn adjusted_saillance_threshold(&self) -> f64 {
        let nm = self.neuromodulators.lock().unwrap();
        let base = 0.3_f64;
        // High dopamine (curiosity) + norepinephrine (alertness) → lower threshold (explore more)
        // High cortisol (stress) → higher threshold (only strong signals surface)
        // Serotonin (stability) → slight upward pressure
        let t = base - (nm.dopamine - 0.5) * 0.2 - nm.norepinephrine * 0.15
            + nm.serotonin * 0.1
            + (nm.cortisol - 0.3) * 0.3;
        t.clamp(0.1, 0.7)
    }
}

// ── E.1: Auto-Predictions ───────────────────────────────────────────────────

impl GraphMemory {
    /// Generate automatic predictions based on detected patterns and reflections.
    /// The system predicts what will happen and tracks its own accuracy.
    pub fn auto_predict(&self) -> Vec<String> {
        let _patterns = self.detect_patterns(3);
        let mood = self.get_mood();
        let mut predictions = Vec::new();

        // Predict from temporal patterns: "user will likely do X soon"
        let due = self.patterns_due_now(3);
        for p in &due {
            let prediction = format!(
                "User will likely perform '{}' action (seen {}x at this time)",
                p.action, p.occurrences
            );
            let confidence = (p.occurrences as f32 / 10.0).min(0.9);
            if let Ok(id) = self.predict(&prediction, confidence, 3600) {
                predictions.push(prediction);
                debug!("Auto-prediction #{id}: confidence {confidence:.2}");
            }
        }

        // Predict from mood: streak-based (verifiable via tool call outcomes)
        if mood.streak >= 5 {
            let pred = format!(
                "Next tool call will succeed (positive streak of {})",
                mood.streak
            );
            let _ = self.predict(&pred, 0.7, 1800);
            predictions.push(pred);
        } else if mood.streak <= -3 {
            let pred = format!(
                "Next tool call may fail (negative streak of {})",
                mood.streak
            );
            let _ = self.predict(&pred, 0.6, 1800);
            predictions.push(pred);
        }

        predictions
    }

    /// Auto-verify predictions that are past their deadline.
    /// Uses heuristics since we can't know ground truth for all predictions.
    pub fn auto_verify_predictions(&self) {
        let now = chrono::Utc::now().timestamp_millis();
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let entities = match graph.scan_entities() {
            Ok(e) => e,
            Err(_) => return,
        };

        // Collect prediction texts to match against recent actions
        let pred_data: Vec<(u64, String)> = entities
            .iter()
            .filter(|e| e.entity_type == "_self::prediction")
            .filter(|e| {
                e.properties.get("outcome").and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.as_str()),
                    _ => None,
                }) == Some("pending")
            })
            .filter(|e| {
                e.properties
                    .get("verify_after")
                    .and_then(|v| match v {
                        PropertyValue::Int(n) => Some(*n),
                        _ => None,
                    })
                    .is_some_and(|t| now > t)
            })
            .map(|e| {
                let text = e
                    .properties
                    .get("prediction_text")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| e.name.clone());
                (e.id.0, text)
            })
            .collect();

        drop(graph);

        // Check recent temporal actions for matches
        let counts = self.temporal_counts.lock().unwrap();
        let recent_actions: Vec<String> = counts
            .keys()
            .filter_map(|k| k.split(':').next().map(String::from))
            .collect();
        drop(counts);

        let mood = self.get_mood();
        for (id, prediction_text) in pred_data {
            // Mood predictions: verify via current streak direction
            let was_correct = if prediction_text.contains("tool call will succeed") {
                mood.streak > 0
            } else if prediction_text.contains("tool call may fail") {
                mood.streak < 0
            } else {
                // Temporal predictions: correct if the predicted action appears in recent records
                recent_actions.iter().any(|action| {
                    prediction_text.contains(&format!("'{action}'"))
                        || prediction_text
                            .to_lowercase()
                            .contains(&action.to_lowercase())
                })
            };
            let _ = self.resolve_prediction(id, was_correct);
        }
    }
}

// ── E.3: Dream Insights ─────────────────────────────────────────────────────

/// An insight discovered during the dream cycle.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DreamInsight {
    pub insight: String,
    pub confidence: f64,
}

impl GraphMemory {
    /// Run dream cycle and extract insights from what was consolidated.
    /// Returns both the stats and any new discoveries.
    pub fn dream_with_insights(&self) -> Result<(serde_json::Value, Vec<DreamInsight>), String> {
        let stats = self.dream_cycle()?;
        let mut insights = Vec::new();

        let facts_created = stats["facts_created"].as_u64().unwrap_or(0);
        let links_created = stats["links_created"].as_u64().unwrap_or(0);
        let dark_nodes = stats["dark_nodes_marked"].as_u64().unwrap_or(0);

        if facts_created > 0 {
            insights.push(DreamInsight {
                insight: format!(
                    "Consolidated {} recurring patterns into permanent knowledge",
                    facts_created
                ),
                confidence: 0.8,
            });
        }

        if links_created > 0 {
            insights.push(DreamInsight {
                insight: format!(
                    "Discovered {} temporal connections between concepts",
                    links_created
                ),
                confidence: 0.7,
            });
        }

        if dark_nodes > 0 {
            insights.push(DreamInsight {
                insight: format!(
                    "Forgot {} inactive memories (freeing cognitive space)",
                    dark_nodes
                ),
                confidence: 0.9,
            });
        }

        // Record insights in the graph
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        for insight in &insights {
            let mut props = hora_graph_core::Properties::new();
            props.insert(
                "confidence".into(),
                PropertyValue::Float(insight.confidence),
            );
            props.insert("source".into(), PropertyValue::String("dream_cycle".into()));
            let _ = graph.add_entity("_self::insight", &insight.insight, Some(props), None);
        }

        Ok((stats, insights))
    }
}

// ── E.4: Emotional Memory Weighting ─────────────────────────────────────────

impl GraphMemory {
    /// Boost activation of entities associated with emotionally significant moments.
    /// Frustration and breakthrough moments are remembered more strongly.
    pub fn emotional_boost(&self) {
        let user = self.get_user_state();
        let mood = self.get_mood();

        // Only boost during emotionally significant states
        let boost_factor = if user.frustration > 0.6 {
            1.5 // Frustration = remember this to avoid repeating
        } else if mood.streak >= 7 {
            1.3 // Success streak = remember what works
        } else {
            return; // Neutral state — no boost needed
        };

        let recent: Vec<EntityId> = {
            let buf = self.recent_entities.lock().unwrap();
            buf.iter().copied().collect()
        };

        if recent.is_empty() {
            return;
        }

        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        for &eid in &recent {
            // Record additional access to boost ACT-R activation
            let _ = graph.get_entity(eid); // Side effect: buffers access event
        }

        debug!(
            "Emotional boost: factor={:.1}, {} entities boosted (frustration={:.2}, streak={})",
            boost_factor,
            recent.len(),
            user.frustration,
            mood.streak
        );
    }
}

// ── System event recording ─────────────────────────────────────────────────

impl GraphMemory {
    /// Record a system event as a typed entity with properties and optional relations.
    /// Used for tool calls, cron executions, errors, triggers, budget events, etc.
    pub fn record_event(
        &self,
        event_type: &str,
        name: &str,
        properties: Vec<(&str, &str)>,
        relate_to: Option<(u64, &str, &str)>,
    ) -> Result<u64, String> {
        let mut graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        let mut props = hora_graph_core::Properties::new();
        for (k, v) in &properties {
            props.insert((*k).into(), PropertyValue::String((*v).into()));
        }
        let safe_name = {
            let mut end = 200.min(name.len());
            while end > 0 && !name.is_char_boundary(end) {
                end -= 1;
            }
            &name[..end]
        };
        let id = graph
            .add_entity(event_type, safe_name, Some(props), None)
            .map_err(|e| format!("Failed to record event: {e}"))?;
        if let Some((target_id, relation, description)) = relate_to {
            let _ = graph.add_fact(
                id,
                hora_graph_core::EntityId(target_id),
                relation,
                description,
                None,
            );
        }
        Ok(id.0)
    }

    /// Find an entity by type and name (for linking).
    pub fn find_entity_by_name(&self, entity_type: &str, name: &str) -> Option<u64> {
        let graph = self.graph.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(entities) = graph.scan_entities() {
            entities
                .iter()
                .find(|e| e.entity_type == entity_type && e.name == name)
                .map(|e| e.id.0)
        } else {
            None
        }
    }
}

/// Heuristic feedback scoring — inspired by predictive coding (Friston).
/// Analyzes a user message to infer how it evaluates the previous assistant response.
/// Returns a confidence score in [-1.0, 1.0]:
///   -1.0 = strong correction, +1.0 = strong confirmation, 0.0 = neutral
fn score_user_feedback(user_msg: &str) -> f32 {
    let lower = user_msg.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    // Correction signals (negative prediction error)
    let correction_markers = [
        "non",
        "pas ça",
        "pas ca",
        "c'est faux",
        "c'est pas",
        "incorrect",
        "erreur",
        "mauvais",
        "wrong",
        "no",
        "nope",
        "corrige",
        "fix",
        "recommence",
        "refais",
        "annule",
        "revert",
        "pas comme ça",
    ];
    let correction_score: f32 = correction_markers
        .iter()
        .filter(|m| lower.contains(*m))
        .count() as f32
        * -0.4;

    // Confirmation signals (prediction fulfilled)
    let confirmation_markers = [
        "oui",
        "ok",
        "parfait",
        "merci",
        "super",
        "bien",
        "genial",
        "yes",
        "great",
        "nice",
        "exactement",
        "c'est ça",
        "bravo",
        "impec",
        "top",
        "correct",
        "good",
    ];
    let confirmation_score: f32 = confirmation_markers
        .iter()
        .filter(|m| lower.contains(*m))
        .count() as f32
        * 0.3;

    // Reformulation signals (high surprise + semantic overlap = new frame)
    let reformulation_markers = [
        "plutot",
        "plutôt",
        "en fait",
        "je voulais dire",
        "ce que je veux",
        "pas exactement",
        "presque mais",
        "plus précisément",
    ];
    let reformulation_score: f32 = reformulation_markers
        .iter()
        .filter(|m| lower.contains(*m))
        .count() as f32
        * -0.2;

    // Continuation signals (user builds on the response = implicit validation)
    let continuation_score: f32 = if words.len() > 10 && correction_score == 0.0 {
        0.15 // Long follow-up without correction = implicit acceptance
    } else {
        0.0
    };

    // Short acknowledgment = mild positive
    let short_ack = if words.len() <= 3 && correction_score == 0.0 && confirmation_score > 0.0 {
        0.1
    } else {
        0.0
    };

    (correction_score + confirmation_score + reformulation_score + continuation_score + short_ack)
        .clamp(-1.0, 1.0)
}

fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Translate a raw thought summary into a human-readable French sentence.
fn humanize_thought(summary: &str, trigger_names: &[String]) -> String {
    // "Discovered N temporal connections between concepts"
    if let Some(rest) = summary.strip_prefix("Discovered ") {
        if let Some(n_end) = rest.find(" temporal connection") {
            let n = &rest[..n_end];
            return format!("{} nouvelles connexions entre tes sujets recents", n);
        }
        return format!("Decouvert : {}", rest);
    }

    // "Outil inexploré" — bare tool names (snake_case)
    let looks_like_tool =
        summary.chars().all(|c| c.is_alphanumeric() || c == '_') && summary.contains('_');
    if looks_like_tool {
        return format!("Outil inexplore : {}", summary);
    }

    // Named trigger entity with no prose context → prefix with label
    if !trigger_names.is_empty() && summary.trim() == trigger_names[0].trim() {
        return format!("Sujet actif : {}", summary);
    }

    // Default: return as-is (already readable)
    summary.to_string()
}

// ── User fact extraction (for SSOT mirror) ─────────────────────────────────

/// A user fact extracted from a message (preference, personal info).
#[derive(Debug, Clone, PartialEq)]
pub struct UserFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

/// Extract preferences and personal info from a user message.
/// Returns structured facts suitable for mirroring to any memory backend.
pub fn extract_user_facts(content: &str) -> Vec<UserFact> {
    let mut facts = Vec::new();
    let lower = content.to_lowercase();

    // Preference patterns
    for prefix in &[
        "je préfère ",
        "je prefere ",
        "j'aime ",
        "je veux ",
        "j'aimerais ",
        "utilise toujours ",
        "envoie-moi ",
        "envoie moi ",
        "fais toujours ",
        "ne fais jamais ",
        "ne fait jamais ",
        "evite de ",
        "évite de ",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let pref = rest
                .split(&['.', ',', '!', '\n'][..])
                .next()
                .unwrap_or(rest)
                .trim();
            if pref.len() > 3 {
                facts.push(UserFact {
                    subject: "user".into(),
                    predicate: "prefers".into(),
                    object: pref.to_string(),
                });
            }
        }
    }

    // Inline preference patterns
    for marker in &[
        "je préfère ",
        "je prefere ",
        "toujours en ",
        "jamais de ",
        "par défaut ",
        "par defaut ",
    ] {
        if let Some(pos) = lower.find(marker) {
            let rest = &lower[pos + marker.len()..];
            let pref = rest
                .split(&['.', ',', '!', '\n'][..])
                .next()
                .unwrap_or(rest)
                .trim();
            if pref.len() > 3 && pref.len() < 100 {
                // Avoid duplicates from prefix match
                let fact = UserFact {
                    subject: "user".into(),
                    predicate: "prefers".into(),
                    object: pref.to_string(),
                };
                if !facts.contains(&fact) {
                    facts.push(fact);
                }
            }
        }
    }

    // Info patterns: "mon X est Y", "ma X est Y"
    for prefix in &["mon ", "ma ", "mes "] {
        if lower.contains(prefix) {
            if let Some(rest) = lower.split_once(prefix).map(|(_, r)| r) {
                if let Some((key, val)) = rest.split_once(" est ") {
                    let key = key.trim();
                    let val = val
                        .split(&['.', ',', '!', '\n'][..])
                        .next()
                        .unwrap_or(val)
                        .trim();
                    if key.len() > 1 && val.len() > 1 && val.len() < 100 {
                        facts.push(UserFact {
                            subject: key.to_string(),
                            predicate: "is".into(),
                            object: val.to_string(),
                        });
                    }
                }
            }
        }
    }

    facts
}

// ── Heuristic entity extraction ─────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ExtractedEntity {
    name: String,
    entity_type: String,
}

/// Extract semantic entities from text using heuristics (no LLM).
///
/// Detects:
/// - Tool names (`shell_exec`, `file_read`, etc.)
/// - Capitalized multi-word names (proper nouns, projects)
/// - Technical terms (URLs, file paths, code identifiers)
fn extract_entities(content: &str) -> Vec<ExtractedEntity> {
    let mut entities: Vec<ExtractedEntity> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. Tool names: word_word pattern (snake_case identifiers)
    for cap in regex_lite::Regex::new(r"\b([a-z][a-z0-9]*(?:_[a-z0-9]+)+)\b")
        .unwrap()
        .find_iter(content)
    {
        let name = cap.as_str();
        if name.len() >= 4 && !is_common_word(name) && seen.insert(name.to_lowercase()) {
            entities.push(ExtractedEntity {
                name: name.to_string(),
                entity_type: "tool".to_string(),
            });
        }
    }

    // 2. Capitalized names: 2-4 consecutive capitalized words (proper nouns, projects)
    for cap in regex_lite::Regex::new(r"\b([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+){0,3})\b")
        .unwrap()
        .find_iter(content)
    {
        let name = cap.as_str().trim();
        if name.len() >= 3 && !is_stop_capitalized(name) && seen.insert(name.to_lowercase()) {
            entities.push(ExtractedEntity {
                name: name.to_string(),
                entity_type: "concept".to_string(),
            });
        }
    }

    // 3. File paths
    for cap in regex_lite::Regex::new(r"(?:^|[\s(])([./~][a-zA-Z0-9_./-]{4,})")
        .unwrap()
        .find_iter(content)
    {
        let path = cap.as_str().trim();
        if path.contains('/') && seen.insert(path.to_lowercase()) {
            entities.push(ExtractedEntity {
                name: path.to_string(),
                entity_type: "path".to_string(),
            });
        }
    }

    // Cap at 10 entities per turn to avoid noise
    entities.truncate(10);
    entities
}

fn is_common_word(w: &str) -> bool {
    matches!(
        w,
        "the_"
            | "and_"
            | "for_"
            | "with_"
            | "from_"
            | "into_"
            | "this_"
            | "that_"
            | "self_"
            | "true_"
            | "false_"
    )
}

fn is_stop_capitalized(w: &str) -> bool {
    matches!(
        w,
        "The"
            | "This"
            | "That"
            | "These"
            | "Those"
            | "Here"
            | "There"
            | "What"
            | "When"
            | "Where"
            | "Which"
            | "Who"
            | "How"
            | "Why"
            | "Yes"
            | "No"
            | "Ok"
            | "None"
            | "Some"
            | "True"
            | "False"
            | "Error"
            | "Result"
            | "Option"
            | "String"
            | "Vec"
            | "If"
            | "Else"
            | "Then"
            | "But"
            | "And"
            | "Or"
            | "Not"
            | "Je"
            | "Tu"
            | "Il"
            | "Elle"
            | "On"
            | "Nous"
            | "Vous"
            | "Oui"
            | "Non"
            | "Merci"
            | "Bonjour"
            | "Salut"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_and_recall() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "What is the weather in Paris?")
            .unwrap();
        mem.store_turn(
            "captain",
            "assistant",
            "It is sunny and 22 degrees in Paris today.",
        )
        .unwrap();

        let results = mem.recall("weather Paris", 5);
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Paris"));
    }

    #[test]
    fn test_dedup_within_60s() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "hello world test")
            .unwrap();
        let count_after_first = mem.stats().entities;
        mem.store_turn("captain", "user", "hello world test")
            .unwrap();
        // Second identical message should not create new turn entity (dedup)
        assert_eq!(mem.stats().entities, count_after_first);
    }

    #[test]
    fn test_skip_short_messages() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "ok").unwrap();
        assert_eq!(mem.stats().entities, 0);
    }

    #[test]
    fn test_record_event_creates_entity() {
        let mem = GraphMemory::new(None).unwrap();
        let id = mem
            .record_event(
                "_sys::tool_call",
                "shell_exec",
                vec![
                    ("agent", "captain"),
                    ("status", "ok"),
                    ("duration_ms", "42"),
                ],
                None,
            )
            .unwrap();
        assert!(id > 0);
        assert_eq!(mem.stats().entities, 1);

        let entities = mem.list_entities(10);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "_sys::tool_call");
        assert_eq!(entities[0].name, "shell_exec");
        assert_eq!(
            entities[0]
                .properties
                .get("status")
                .and_then(|v| v.as_str()),
            Some("ok")
        );
        assert_eq!(
            entities[0]
                .properties
                .get("duration_ms")
                .and_then(|v| v.as_str()),
            Some("42")
        );
    }

    #[test]
    fn test_record_event_with_relation() {
        let mem = GraphMemory::new(None).unwrap();
        let agent_id = mem
            .add_doc_entity("agent", "captain", "Agent captain", &["agent"])
            .unwrap();
        let tool_id = mem
            .record_event(
                "_sys::tool_call",
                "shell_exec",
                vec![("status", "ok")],
                Some((agent_id, "exécuté_par", "captain")),
            )
            .unwrap();
        assert!(tool_id > 0);
        assert_eq!(mem.stats().entities, 2);
        assert!(mem.stats().edges >= 1);
    }

    #[test]
    fn test_find_entity_by_name() {
        let mem = GraphMemory::new(None).unwrap();
        mem.add_doc_entity("agent", "captain", "Agent captain", &["agent"])
            .unwrap();
        mem.add_doc_entity("agent", "worker", "Agent worker", &["agent"])
            .unwrap();

        let found = mem.find_entity_by_name("agent", "captain");
        assert!(found.is_some());

        let not_found = mem.find_entity_by_name("agent", "nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_record_multiple_event_types() {
        let mem = GraphMemory::new(None).unwrap();
        mem.record_event(
            "_sys::tool_call",
            "shell_exec",
            vec![("status", "ok")],
            None,
        )
        .unwrap();
        mem.record_event(
            "_sys::usage",
            "usage:captain",
            vec![("input_tokens", "1000"), ("cost_usd", "0.01")],
            None,
        )
        .unwrap();
        mem.record_event(
            "_sys::error",
            "error:captain",
            vec![("error", "timeout"), ("severity", "critical")],
            None,
        )
        .unwrap();

        let entities = mem.list_entities(20);
        assert!(entities.len() >= 3);
        assert!(entities.iter().any(|e| e.entity_type == "_sys::tool_call"));
        assert!(entities.iter().any(|e| e.entity_type == "_sys::usage"));
        assert!(entities.iter().any(|e| e.entity_type == "_sys::error"));
    }

    // ── C.1 Consciousness tests ─────────────────────────────────────────

    #[test]
    fn test_store_turn_extracts_entities() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn(
            "captain",
            "user",
            "Run shell_exec to check the Captain status",
        )
        .unwrap();

        let entities = mem.list_entities(50);
        // Should have: 1 turn + 1 tool (shell_exec) + 1 concept (Captain) + 1 agent (captain)
        assert!(entities.iter().any(|e| e.entity_type == "_conv::turn"));
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "tool" && e.name == "shell_exec"));
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "agent" && e.name == "captain"));
    }

    #[test]
    fn test_store_turn_creates_relations() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "Use file_read on the config")
            .unwrap();

        let facts = mem.list_facts(50);
        // Should have mention relations + agent relation
        assert!(facts.iter().any(|f| f.relation_type == "mentions"));
        assert!(facts
            .iter()
            .any(|f| f.relation_type == "sent_to" || f.relation_type == "produced_by"));
    }

    #[test]
    fn test_store_turn_creates_episode() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "Deploy the application to production")
            .unwrap();

        let stats = mem.extended_stats();
        assert!(stats["episodes"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_recall_with_spreading() {
        let mem = GraphMemory::new(None).unwrap();
        // Store two related turns mentioning the same tool
        mem.store_turn("captain", "user", "Run shell_exec to check disk space")
            .unwrap();
        // Force different hash by changing content
        std::thread::sleep(std::time::Duration::from_millis(10));
        mem.store_turn(
            "captain",
            "assistant",
            "The shell_exec command shows 50GB free",
        )
        .unwrap();

        // Recall should find both via BM25 + spreading (they share shell_exec entity)
        let results = mem.recall("disk space", 5);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_extract_entities_heuristic() {
        let entities = super::extract_entities(
            "Use shell_exec and file_read to check the Captain config at ./config/settings.toml",
        );
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"file_read"));
        assert!(entities.iter().any(|e| e.entity_type == "tool"));
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "concept" && e.name == "Captain"));
    }

    #[test]
    fn test_extract_entities_caps_limit() {
        // Should not extract stop words or common English starters
        let entities =
            super::extract_entities("The quick brown fox. This is a test. What about None?");
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();

        assert!(!names.contains(&"The"));
        assert!(!names.contains(&"This"));
        assert!(!names.contains(&"What"));
        assert!(!names.contains(&"None"));
    }

    #[test]
    fn test_neural_pulse_empty() {
        let mem = GraphMemory::new(None).unwrap();
        // No recent entities → no thoughts
        let thoughts = mem.neural_pulse(0.5);
        assert!(thoughts.is_empty());
    }

    #[test]
    fn test_neural_pulse_with_data() {
        let mem = GraphMemory::new(None).unwrap();
        // Store several turns to build up graph connections
        mem.store_turn("captain", "user", "Use shell_exec to deploy Captain")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        mem.store_turn(
            "captain",
            "assistant",
            "Running shell_exec for Captain deployment",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        mem.store_turn("captain", "user", "Check the Captain logs with file_read")
            .unwrap();

        // Neural pulse should run without panic
        let thoughts = mem.neural_pulse(0.0); // Low threshold to catch any signal
                                              // We can't guarantee thoughts emerge (depends on graph topology),
                                              // but the function should run cleanly
        assert!(thoughts.len() <= 5); // Capped at 5
    }

    #[test]
    fn test_recent_entities_buffer() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "Test message with some_tool mention")
            .unwrap();

        let recent = mem.recent_entities.lock().unwrap();
        assert!(!recent.is_empty());
    }

    #[test]
    fn test_drain_recent_entities() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "A message for the buffer test")
            .unwrap();
        assert!(!mem.recent_entities.lock().unwrap().is_empty());

        mem.drain_recent_entities();
        assert!(mem.recent_entities.lock().unwrap().is_empty());
    }

    #[test]
    fn test_filter_discards_weak() {
        let mem = GraphMemory::new(None).unwrap();
        let weak = vec![super::EmergentThought {
            trigger_entities: vec![1],
            trigger_names: vec!["test".into()],
            activation_score: 0.1, // Below 0.3 threshold
            thought_type: super::ThoughtType::Insight,
            summary: "Weak signal".into(),
            created_at: chrono::Utc::now().timestamp_millis(),
        }];
        let surfaced = mem.filter_thoughts(weak);
        assert!(surfaced.is_empty());
        assert_eq!(mem.queued_thought_count(), 0); // Discarded, not queued
    }

    #[test]
    fn test_filter_queues_medium() {
        let mem = GraphMemory::new(None).unwrap();
        let medium = vec![super::EmergentThought {
            trigger_entities: vec![1],
            trigger_names: vec!["test".into()],
            activation_score: 0.5, // Above 0.3, below 0.8
            thought_type: super::ThoughtType::Insight,
            summary: "Medium signal".into(),
            created_at: chrono::Utc::now().timestamp_millis(),
        }];
        let surfaced = mem.filter_thoughts(medium);
        assert!(surfaced.is_empty()); // Not strong enough to surface immediately
        assert_eq!(mem.queued_thought_count(), 1); // Queued instead
    }

    #[test]
    fn test_filter_surfaces_strong() {
        let mem = GraphMemory::new(None).unwrap();
        let strong = vec![super::EmergentThought {
            trigger_entities: vec![1],
            trigger_names: vec!["test".into()],
            activation_score: 0.9, // Above 0.8
            thought_type: super::ThoughtType::Insight,
            summary: "Strong signal".into(),
            created_at: chrono::Utc::now().timestamp_millis(),
        }];
        let surfaced = mem.filter_thoughts(strong);
        assert_eq!(surfaced.len(), 1);
    }

    #[test]
    fn test_filter_anomaly_always_surfaces() {
        let mem = GraphMemory::new(None).unwrap();
        let anomaly = vec![super::EmergentThought {
            trigger_entities: vec![1],
            trigger_names: vec!["test".into()],
            activation_score: 0.4, // Below 0.8 but anomaly = urgent
            thought_type: super::ThoughtType::Anomaly,
            summary: "Contradiction detected".into(),
            created_at: chrono::Utc::now().timestamp_millis(),
        }];
        let surfaced = mem.filter_thoughts(anomaly);
        assert_eq!(surfaced.len(), 1);
    }

    #[test]
    fn test_consume_queued_thoughts() {
        let mem = GraphMemory::new(None).unwrap();
        // Queue some medium thoughts
        for i in 0..5 {
            mem.filter_thoughts(vec![super::EmergentThought {
                trigger_entities: vec![1],
                trigger_names: vec!["test".into()],
                activation_score: 0.4 + (i as f64) * 0.05,
                thought_type: super::ThoughtType::Insight,
                summary: format!("Thought {i}"),
                created_at: chrono::Utc::now().timestamp_millis(),
            }]);
        }
        assert_eq!(mem.queued_thought_count(), 5);

        let consumed = mem.consume_queued_thoughts(3);
        assert_eq!(consumed.len(), 3);
        // Should be sorted by score descending
        assert!(consumed[0].activation_score >= consumed[1].activation_score);
        assert_eq!(mem.queued_thought_count(), 0); // Queue drained
    }

    // ── C.5 Self-model & Reflection tests ───────────────────────────────

    #[test]
    fn test_seed_self_model() {
        let mem = GraphMemory::new(None).unwrap();
        mem.seed_self_model(
            &["shell_exec", "file_read", "web_search"],
            &["clip", "transcriber"],
            &["captain"],
        );

        let entities = mem.list_entities(100);
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "_self::system" && e.name == "captain"));
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "_self::tool" && e.name == "shell_exec"));
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "_self::hand" && e.name == "clip"));
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "_self::agent" && e.name == "captain"));

        let facts = mem.list_facts(100);
        assert!(facts.iter().any(|f| f.relation_type == "has_capability"));
        assert!(facts.iter().any(|f| f.relation_type == "has_hand"));
        assert!(facts.iter().any(|f| f.relation_type == "runs_agent"));
    }

    #[test]
    fn test_reflect_success() {
        let mem = GraphMemory::new(None).unwrap();
        mem.reflect("captain", &["shell_exec", "file_read"], true, 3)
            .unwrap();

        let entities = mem.list_entities(100);
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "_self::reflection"));

        let facts = mem.list_facts(100);
        assert!(facts.iter().any(|f| f.relation_type == "succeeded_with"));
        assert!(facts.iter().any(|f| f.relation_type == "reflected_by"));

        let stats = mem.extended_stats();
        assert!(stats["episodes"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_reflect_failure() {
        let mem = GraphMemory::new(None).unwrap();
        mem.reflect("worker", &["web_fetch"], false, 1).unwrap();

        let facts = mem.list_facts(100);
        assert!(facts.iter().any(|f| f.relation_type == "failed_with"));
    }

    // ── D.3 User State tests ────────────────────────────────────────────

    #[test]
    fn test_user_state_rushed() {
        let mem = GraphMemory::new(None).unwrap();
        // Multiple short messages to converge the EMA
        mem.update_user_state("fix");
        mem.update_user_state("ok");
        let state = mem.update_user_state("go");
        assert!(state.pace < 0.4);
    }

    #[test]
    fn test_user_state_frustration() {
        let mem = GraphMemory::new(None).unwrap();
        let _ = mem.update_user_state("non pas ça, corrige le bug");
        let state = mem.get_user_state();
        assert!(state.frustration > 0.0);
    }

    #[test]
    fn test_user_state_mode_debug() {
        let mem = GraphMemory::new(None).unwrap();
        let state = mem.update_user_state("there's a bug in the auth module, need to debug");
        assert_eq!(state.mode, "debug");
    }

    #[test]
    fn test_user_state_prompt_empty_at_start() {
        let mem = GraphMemory::new(None).unwrap();
        assert!(mem.user_state_prompt().is_empty()); // < 2 interactions
    }

    // ── D.4 Prediction tests ────────────────────────────────────────────

    #[test]
    fn test_prediction_lifecycle() {
        let mem = GraphMemory::new(None).unwrap();
        let id = mem
            .predict("shell_exec will timeout on large repos", 0.7, 3600)
            .unwrap();
        assert!(id > 0);

        mem.resolve_prediction(id, true).unwrap();
        let (accuracy, correct, total) = mem.prediction_accuracy();
        assert_eq!(correct, 1);
        assert_eq!(total, 1);
        assert!((accuracy - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_prediction_accuracy_mixed() {
        let mem = GraphMemory::new(None).unwrap();
        let id1 = mem
            .predict("shell_exec will timeout on large repos", 0.5, 60)
            .unwrap();
        let id2 = mem
            .predict("web_fetch fails on rate-limited APIs", 0.5, 60)
            .unwrap();
        let id3 = mem
            .predict("file_read handles unicode correctly", 0.5, 60)
            .unwrap();
        mem.resolve_prediction(id1, true).unwrap();
        mem.resolve_prediction(id2, false).unwrap();
        mem.resolve_prediction(id3, true).unwrap();

        let (accuracy, correct, total) = mem.prediction_accuracy();
        assert_eq!(correct, 2);
        assert_eq!(total, 3);
        assert!((accuracy - 2.0 / 3.0).abs() < 0.01);
    }

    // ── D.5 System Mood tests ───────────────────────────────────────────

    #[test]
    fn test_mood_success_streak() {
        let mem = GraphMemory::new(None).unwrap();
        for _ in 0..10 {
            mem.update_mood(true);
        }
        let mood = mem.get_mood();
        assert!(mood.confidence > 0.5);
        assert!(mood.streak >= 10);
    }

    #[test]
    fn test_mood_failure_drops_confidence() {
        let mem = GraphMemory::new(None).unwrap();
        for _ in 0..5 {
            mem.update_mood(true);
        }
        let before = mem.get_mood().confidence;
        mem.update_mood(false);
        let after = mem.get_mood().confidence;
        assert!(after < before);
    }

    #[test]
    fn test_mood_prompt_cautious() {
        let mem = GraphMemory::new(None).unwrap();
        for _ in 0..15 {
            mem.update_mood(false);
        } // sustained failures
        let mood = mem.get_mood();
        assert!(
            mood.confidence < 0.3,
            "Expected < 0.3, got {}",
            mood.confidence
        );
        let prompt = mem.mood_prompt();
        assert!(prompt.contains("prudent"));
    }

    // ── D.6 Temporal Pattern tests ──────────────────────────────────────

    #[test]
    fn test_temporal_record_and_detect() {
        let mem = GraphMemory::new(None).unwrap();
        for _ in 0..5 {
            mem.record_temporal_action("deploy");
        }
        let patterns = mem.detect_patterns(3);
        assert!(!patterns.is_empty());
        assert!(patterns[0].action == "deploy");
        assert!(patterns[0].occurrences >= 5);
    }

    #[test]
    fn test_temporal_below_threshold() {
        let mem = GraphMemory::new(None).unwrap();
        mem.record_temporal_action("rare_action");
        let patterns = mem.detect_patterns(3);
        assert!(patterns.is_empty());
    }

    // ── D.8 Narration tests ─────────────────────────────────────────────

    #[test]
    fn test_narration_creates_entity() {
        let mem = GraphMemory::new(None).unwrap();
        mem.narrate("captain", "check the logs", &["shell_exec"], true)
            .unwrap();

        let entities = mem.list_entities(100);
        assert!(entities.iter().any(|e| e.entity_type == "_self::narration"));
    }

    #[test]
    fn test_narration_records_temporal() {
        let mem = GraphMemory::new(None).unwrap();
        mem.narrate("captain", "run tests", &["shell_exec"], true)
            .unwrap();
        mem.narrate("captain", "run build", &["shell_exec"], true)
            .unwrap();
        mem.narrate("captain", "run deploy", &["shell_exec"], true)
            .unwrap();

        // Should have recorded temporal actions
        let patterns = mem.detect_patterns(3);
        assert!(!patterns.is_empty());
    }

    // ── D.7 Curiosity tests ─────────────────────────────────────────────

    #[test]
    fn test_curiosity_detects_unused_tools() {
        let mem = GraphMemory::new(None).unwrap();
        mem.seed_self_model(&["shell_exec", "web_search", "rare_tool"], &[], &[]);
        // Reflect only on shell_exec — web_search and rare_tool are unexplored
        mem.reflect("captain", &["shell_exec"], true, 1).unwrap();

        let items = mem.curiosity_scan();
        let topics: Vec<&str> = items.iter().map(|i| i.topic.as_str()).collect();
        assert!(topics
            .iter()
            .any(|t| t.contains("web_search") || t.contains("rare_tool")));
    }

    // ── D.9 Neuromodulator tests ────────────────────────────────────────

    #[test]
    fn test_neuromodulators_default() {
        let mem = GraphMemory::new(None).unwrap();
        let nm = mem.get_neuromodulators();
        assert!((nm.dopamine - 0.5).abs() < f64::EPSILON);
        assert!(nm.cortisol < 0.2);
    }

    #[test]
    fn test_neuromodulators_adapt() {
        let mem = GraphMemory::new(None).unwrap();
        // Simulate stress: many failures + frustrated user
        for _ in 0..10 {
            mem.update_mood(false);
        }
        mem.update_user_state("non pas ça corrige le bug maintenant");

        let nm = mem.recompute_neuromodulators();
        assert!(nm.norepinephrine > 0.3, "Expected alert state");
        assert!(nm.cortisol > 0.0, "Expected some stress");
    }

    #[test]
    fn test_adjusted_threshold() {
        let mem = GraphMemory::new(None).unwrap();
        let base = mem.adjusted_saillance_threshold();
        assert!(
            (0.2..=0.8).contains(&base),
            "Base threshold out of range: {}",
            base
        );

        // After recomputing with state changes, threshold should still be valid
        for _ in 0..10 {
            mem.update_mood(false);
        }
        mem.update_user_state("non pas ça corrige le bug");
        mem.recompute_neuromodulators();
        let adjusted = mem.adjusted_saillance_threshold();
        assert!(
            (0.2..=0.8).contains(&adjusted),
            "Adjusted threshold out of range: {}",
            adjusted
        );
    }

    // ── E.1 Auto-Prediction tests ───────────────────────────────────────

    #[test]
    fn test_auto_predict_from_mood_streak() {
        let mem = GraphMemory::new(None).unwrap();
        for _ in 0..6 {
            mem.update_mood(true);
        }
        let preds = mem.auto_predict();
        assert!(preds.iter().any(|p| p.contains("succeed")));
    }

    #[test]
    fn test_auto_verify_clears_pending() {
        let mem = GraphMemory::new(None).unwrap();
        // Create a prediction with verify_after in the past
        let _id = mem.predict("test prediction", 0.5, 0).unwrap(); // verify_after = now
        std::thread::sleep(std::time::Duration::from_millis(10));
        mem.auto_verify_predictions();
        let (_, _, total) = mem.prediction_accuracy();
        assert!(total >= 1, "Should have resolved the overdue prediction");
    }

    // ── E.3 Dream Insight tests ─────────────────────────────────────────

    #[test]
    fn test_dream_with_insights_runs() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "Run shell_exec to check status")
            .unwrap();
        let result = mem.dream_with_insights();
        assert!(result.is_ok());
    }

    // ── E.4 Emotional Boost tests ───────────────────────────────────────

    #[test]
    fn test_emotional_boost_no_panic() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "Fix this bug immediately")
            .unwrap();
        // Simulate frustration
        mem.update_user_state("non pas ça corrige le bug");
        mem.update_user_state("non pas ça je t'ai dit");
        mem.emotional_boost(); // Should not panic
    }

    #[test]
    fn test_emotional_boost_skips_neutral() {
        let mem = GraphMemory::new(None).unwrap();
        mem.store_turn("captain", "user", "Just checking things")
            .unwrap();
        mem.emotional_boost(); // Neutral state — should be a no-op
    }

    // ── extract_user_facts tests ───────────────────────────────────────────

    #[test]
    fn test_extract_preference_direct() {
        let facts = extract_user_facts("je préfère le mode sombre");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].subject, "user");
        assert_eq!(facts[0].predicate, "prefers");
        assert_eq!(facts[0].object, "le mode sombre");
    }

    #[test]
    fn test_extract_preference_inline() {
        let facts = extract_user_facts("Pour les notifs, je préfère telegram");
        assert!(facts
            .iter()
            .any(|f| f.predicate == "prefers" && f.object == "telegram"));
    }

    #[test]
    fn test_extract_personal_info() {
        let facts = extract_user_facts("mon fuseau horaire est Europe/Paris");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].subject, "fuseau horaire");
        assert_eq!(facts[0].predicate, "is");
        assert_eq!(facts[0].object, "europe/paris");
    }

    #[test]
    fn test_extract_nothing_from_neutral() {
        let facts = extract_user_facts("quel temps fait-il ?");
        assert!(facts.is_empty());
    }

    #[test]
    fn test_extract_negative_preference() {
        let facts = extract_user_facts("ne fais jamais de résumé à la fin");
        // Matches both "ne fais jamais " (direct) and "jamais de " (inline)
        assert!(!facts.is_empty());
        assert!(facts.iter().all(|f| f.predicate == "prefers"));
        assert!(facts.iter().any(|f| f.object.contains("résumé")));
    }

    #[test]
    fn test_no_duplicates() {
        let facts = extract_user_facts("je préfère le français");
        // "je préfère" matches both direct and inline — should dedup
        let pref_count = facts.iter().filter(|f| f.object == "le français").count();
        assert_eq!(pref_count, 1);
    }
}
