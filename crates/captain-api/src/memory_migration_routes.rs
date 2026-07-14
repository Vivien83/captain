//! Memory migration route handlers.

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use captain_types::config::MemoryBackend;
use std::collections::HashMap;
use std::sync::Arc;

const MEMPALACE_KG_ADD_TOOL: &str = "mcp_mempalace_mempalace_kg_add";

/// POST /api/memory/migrate - One-shot migration of graph entities/facts to MemPalace KG.
pub async fn memory_migrate_to_mempalace(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if state.kernel.config.memory.backend != MemoryBackend::Mempalace {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Memory backend is not mempalace. Set [memory] backend = \"mempalace\" in config.toml"
            })),
        )
            .into_response();
    }

    let entities = state.kernel.graph_memory.list_entities(5000);
    let facts = state.kernel.graph_memory.list_facts(10000);
    let entity_names: HashMap<u64, &str> =
        entities.iter().map(|e| (e.id, e.name.as_str())).collect();

    let mut migrated_kg = 0u32;
    let mut skipped = 0u32;
    let mut conns = state.kernel.mcp_connections.lock().await;
    let Some(conn) = conns.iter_mut().find(|conn| conn.name() == "mempalace") else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "MemPalace MCP server not connected"
            })),
        )
            .into_response();
    };

    for fact in &facts {
        let source_name = entity_names.get(&fact.source).copied().unwrap_or("unknown");
        let target_name = entity_names.get(&fact.target).copied().unwrap_or("unknown");
        if should_skip_fact(
            source_name,
            target_name,
            &fact.relation_type,
            fact.invalid_at,
        ) {
            skipped += 1;
            continue;
        }

        let kg_input = serde_json::json!({
            "subject": source_name,
            "predicate": fact.relation_type,
            "object": target_name,
        });
        match conn.call_tool(MEMPALACE_KG_ADD_TOOL, &kg_input).await {
            Ok(_) => migrated_kg += 1,
            Err(e) => {
                tracing::debug!(
                    "Migration kg_add failed for {}->{}: {e}",
                    source_name,
                    target_name
                );
            }
        }
    }

    let mut migrated_entities = 0u32;
    for entity in &entities {
        if entity.entity_type == "_user::preference" {
            let kg_input = serde_json::json!({
                "subject": "user",
                "predicate": "prefers",
                "object": entity.name,
            });
            if conn
                .call_tool(MEMPALACE_KG_ADD_TOOL, &kg_input)
                .await
                .is_ok()
            {
                migrated_entities += 1;
            }
        } else if entity.entity_type == "_user::info" {
            if let Some((key, value)) = entity.name.split_once(':') {
                let kg_input = serde_json::json!({
                    "subject": key.trim(),
                    "predicate": "is",
                    "object": value.trim(),
                });
                if conn
                    .call_tool(MEMPALACE_KG_ADD_TOOL, &kg_input)
                    .await
                    .is_ok()
                {
                    migrated_entities += 1;
                }
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "migrated_facts": migrated_kg,
            "migrated_entities": migrated_entities,
            "skipped": skipped,
            "total_entities_scanned": entities.len(),
            "total_facts_scanned": facts.len(),
        })),
    )
        .into_response()
}

fn should_skip_fact(source: &str, target: &str, relation: &str, invalid_at: i64) -> bool {
    invalid_at > 0
        || is_internal_entity(source)
        || is_internal_entity(target)
        || is_internal_relation(relation)
}

fn is_internal_entity(name: &str) -> bool {
    name.starts_with("_conv::") || name.starts_with("_sys::")
}

fn is_internal_relation(relation: &str) -> bool {
    matches!(
        relation,
        "mentions" | "sent_to" | "produced_by" | "exécuté_par" | "consommé_par"
    )
}
