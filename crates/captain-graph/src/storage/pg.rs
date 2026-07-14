//! PostgreSQL storage backend for hora-graph-core.
//!
//! Activated via the `postgres` feature flag. Uses the sync `postgres` crate
//! (same author as tokio-postgres) so no async runtime is required.
//!
//! Properties and embeddings are serialised as BYTEA using the same binary
//! format as the embedded/SQLite backends. Full-text search uses PostgreSQL
//! native `tsvector` with a GIN index.

use std::cell::RefCell;
use std::io::Cursor;

use postgres::{Client, NoTls};

use crate::core::edge::Edge;
use crate::core::entity::Entity;
use crate::core::episode::Episode;
use crate::core::types::{EdgeId, EntityId, EpisodeSource, Properties, StorageStats};
use crate::error::{HoraError, Result};
use crate::storage::format::{read_properties, write_properties};
use crate::storage::traits::StorageOps;

// ── Helpers ──────────────────────────────────────────────────

fn pg_err(e: postgres::Error) -> HoraError {
    HoraError::Postgres(e.to_string())
}

fn serialize_properties(props: &Properties) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    write_properties(&mut buf, props).map_err(HoraError::Io)?;
    Ok(buf)
}

fn deserialize_properties(blob: &[u8]) -> Result<Properties> {
    let mut cursor = Cursor::new(blob);
    read_properties(&mut cursor).map_err(HoraError::Io)
}

fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

fn deserialize_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn serialize_u64_vec(ids: &[u64]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ids.len() * 8);
    for &id in ids {
        buf.extend_from_slice(&id.to_le_bytes());
    }
    buf
}

fn deserialize_u64_vec(blob: &[u8]) -> Vec<u64> {
    blob.chunks_exact(8)
        .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
        .collect()
}

fn episode_source_to_str(s: &EpisodeSource) -> &'static str {
    match s {
        EpisodeSource::Conversation => "conversation",
        EpisodeSource::Document => "document",
        EpisodeSource::Api => "api",
    }
}

fn episode_source_from_str(s: &str) -> EpisodeSource {
    match s {
        "document" => EpisodeSource::Document,
        "api" => EpisodeSource::Api,
        _ => EpisodeSource::Conversation,
    }
}

// ── Schema ───────────────────────────────────────────────────

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entities (
    id          BIGINT PRIMARY KEY,
    entity_type TEXT   NOT NULL,
    name        TEXT   NOT NULL,
    properties  BYTEA,
    embedding   BYTEA,
    created_at  BIGINT NOT NULL,
    search_vector tsvector GENERATED ALWAYS AS (
        to_tsvector('simple', coalesce(name, '') || ' ' || coalesce(entity_type, ''))
    ) STORED
);

CREATE TABLE IF NOT EXISTS edges (
    id            BIGINT PRIMARY KEY,
    source_id     BIGINT NOT NULL REFERENCES entities(id),
    target_id     BIGINT NOT NULL REFERENCES entities(id),
    relation_type TEXT   NOT NULL,
    description   TEXT   NOT NULL DEFAULT '',
    confidence    REAL   NOT NULL DEFAULT 1.0,
    valid_at      BIGINT NOT NULL,
    invalid_at    BIGINT NOT NULL DEFAULT 0,
    created_at    BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS episodes (
    id                  BIGINT PRIMARY KEY,
    source              TEXT   NOT NULL,
    session_id          TEXT   NOT NULL DEFAULT '',
    entity_ids          BYTEA,
    fact_ids            BYTEA,
    created_at          BIGINT NOT NULL,
    consolidation_count INTEGER NOT NULL DEFAULT 0
);
";

// Indexes created separately — CREATE INDEX IF NOT EXISTS can't be in the same
// batch as CREATE TABLE on all PG versions, so we run them one by one.
const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_edges_source   ON edges(source_id)",
    "CREATE INDEX IF NOT EXISTS idx_edges_target   ON edges(target_id)",
    "CREATE INDEX IF NOT EXISTS idx_edges_valid_at ON edges(valid_at)",
    "CREATE INDEX IF NOT EXISTS idx_entities_name  ON entities(name)",
    "CREATE INDEX IF NOT EXISTS idx_entities_type  ON entities(entity_type)",
    "CREATE INDEX IF NOT EXISTS idx_entities_search ON entities USING gin(search_vector)",
];

// ── PostgresStorage ──────────────────────────────────────────

/// PostgreSQL-backed storage. Suitable for multi-client applications,
/// cloud deployments, or when an existing PostgreSQL instance is available.
pub struct PostgresStorage {
    client: RefCell<Client>,
}

impl PostgresStorage {
    /// Connect to a PostgreSQL database and initialise the schema.
    ///
    /// `url` is a standard connection string, e.g.
    /// `"host=localhost user=hora dbname=hora_test"` or
    /// `"postgresql://hora:password@localhost/hora_test"`.
    pub fn connect(url: &str) -> Result<Self> {
        let client = Client::connect(url, NoTls).map_err(pg_err)?;
        Self::init(client)
    }

    fn init(mut client: Client) -> Result<Self> {
        client.batch_execute(SCHEMA).map_err(pg_err)?;

        for idx in INDEXES {
            client.batch_execute(idx).map_err(pg_err)?;
        }

        Ok(Self {
            client: RefCell::new(client),
        })
    }

    /// Execute raw SQL (for test setup, migrations, etc.).
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.client.borrow_mut().batch_execute(sql).map_err(pg_err)
    }

    /// Full-text search over entity names and types via tsvector.
    ///
    /// Returns matching entity IDs ranked by relevance.
    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<EntityId>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id FROM entities \
                 WHERE search_vector @@ to_tsquery('simple', $1) \
                 ORDER BY ts_rank(search_vector, to_tsquery('simple', $1)) DESC \
                 LIMIT $2",
                &[&query, &(limit as i64)],
            )
            .map_err(pg_err)?;

        Ok(rows
            .iter()
            .map(|r| EntityId(r.get::<_, i64>(0) as u64))
            .collect())
    }
}

// ── Row → struct mappers ─────────────────────────────────────

fn row_to_entity(row: &postgres::Row) -> Entity {
    let id: i64 = row.get(0);
    let entity_type: String = row.get(1);
    let name: String = row.get(2);
    let props_blob: Option<Vec<u8>> = row.get(3);
    let emb_blob: Option<Vec<u8>> = row.get(4);
    let created_at: i64 = row.get(5);

    let properties = match props_blob {
        Some(b) if !b.is_empty() => deserialize_properties(&b).unwrap_or_default(),
        _ => Properties::new(),
    };

    let embedding = emb_blob
        .filter(|b| !b.is_empty())
        .map(|b| deserialize_embedding(&b));

    Entity {
        id: EntityId(id as u64),
        entity_type,
        name,
        properties,
        embedding,
        created_at,
    }
}

fn row_to_edge(row: &postgres::Row) -> Edge {
    Edge {
        id: EdgeId(row.get::<_, i64>(0) as u64),
        source: EntityId(row.get::<_, i64>(1) as u64),
        target: EntityId(row.get::<_, i64>(2) as u64),
        relation_type: row.get(3),
        description: row.get(4),
        confidence: row.get(5),
        valid_at: row.get(6),
        invalid_at: row.get(7),
        created_at: row.get(8),
    }
}

fn row_to_episode(row: &postgres::Row) -> Episode {
    let id: i64 = row.get(0);
    let source_str: String = row.get(1);
    let session_id: String = row.get(2);
    let entity_blob: Option<Vec<u8>> = row.get(3);
    let fact_blob: Option<Vec<u8>> = row.get(4);
    let created_at: i64 = row.get(5);
    let consolidation_count: i32 = row.get(6);

    Episode {
        id: id as u64,
        source: episode_source_from_str(&source_str),
        session_id,
        entity_ids: entity_blob
            .map(|b| deserialize_u64_vec(&b).into_iter().map(EntityId).collect())
            .unwrap_or_default(),
        fact_ids: fact_blob
            .map(|b| deserialize_u64_vec(&b).into_iter().map(EdgeId).collect())
            .unwrap_or_default(),
        created_at,
        consolidation_count: consolidation_count as u32,
    }
}

// ── StorageOps impl ──────────────────────────────────────────

impl StorageOps for PostgresStorage {
    fn put_entity(&mut self, entity: Entity) -> Result<()> {
        let props_blob = serialize_properties(&entity.properties)?;
        let emb_blob: Option<Vec<u8>> = entity.embedding.as_deref().map(serialize_embedding);

        self.client
            .borrow_mut()
            .execute(
                "INSERT INTO entities (id, entity_type, name, properties, embedding, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (id) DO UPDATE SET \
                     entity_type = EXCLUDED.entity_type, \
                     name = EXCLUDED.name, \
                     properties = EXCLUDED.properties, \
                     embedding = EXCLUDED.embedding, \
                     created_at = EXCLUDED.created_at",
                &[
                    &(entity.id.0 as i64),
                    &entity.entity_type,
                    &entity.name,
                    &props_blob,
                    &emb_blob,
                    &entity.created_at,
                ],
            )
            .map_err(pg_err)?;

        Ok(())
    }

    fn get_entity(&self, id: EntityId) -> Result<Option<Entity>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, entity_type, name, properties, embedding, created_at \
                 FROM entities WHERE id = $1",
                &[&(id.0 as i64)],
            )
            .map_err(pg_err)?;

        Ok(rows.first().map(row_to_entity))
    }

    fn delete_entity(&mut self, id: EntityId) -> Result<bool> {
        let n = self
            .client
            .borrow_mut()
            .execute("DELETE FROM entities WHERE id = $1", &[&(id.0 as i64)])
            .map_err(pg_err)?;
        Ok(n > 0)
    }

    fn put_edge(&mut self, edge: Edge) -> Result<()> {
        self.client
            .borrow_mut()
            .execute(
                "INSERT INTO edges \
                 (id, source_id, target_id, relation_type, description, \
                  confidence, valid_at, invalid_at, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT (id) DO UPDATE SET \
                     source_id = EXCLUDED.source_id, \
                     target_id = EXCLUDED.target_id, \
                     relation_type = EXCLUDED.relation_type, \
                     description = EXCLUDED.description, \
                     confidence = EXCLUDED.confidence, \
                     valid_at = EXCLUDED.valid_at, \
                     invalid_at = EXCLUDED.invalid_at, \
                     created_at = EXCLUDED.created_at",
                &[
                    &(edge.id.0 as i64),
                    &(edge.source.0 as i64),
                    &(edge.target.0 as i64),
                    &edge.relation_type,
                    &edge.description,
                    &edge.confidence,
                    &edge.valid_at,
                    &edge.invalid_at,
                    &edge.created_at,
                ],
            )
            .map_err(pg_err)?;
        Ok(())
    }

    fn get_edge(&self, id: EdgeId) -> Result<Option<Edge>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, source_id, target_id, relation_type, description, \
                 confidence, valid_at, invalid_at, created_at \
                 FROM edges WHERE id = $1",
                &[&(id.0 as i64)],
            )
            .map_err(pg_err)?;

        Ok(rows.first().map(row_to_edge))
    }

    fn get_entity_edges(&self, entity_id: EntityId) -> Result<Vec<Edge>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, source_id, target_id, relation_type, description, \
                 confidence, valid_at, invalid_at, created_at \
                 FROM edges WHERE source_id = $1 OR target_id = $1",
                &[&(entity_id.0 as i64)],
            )
            .map_err(pg_err)?;

        Ok(rows.iter().map(row_to_edge).collect())
    }

    fn get_entity_edge_ids(&self, entity_id: EntityId) -> Result<Vec<EdgeId>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id FROM edges WHERE source_id = $1 OR target_id = $1",
                &[&(entity_id.0 as i64)],
            )
            .map_err(pg_err)?;

        Ok(rows
            .iter()
            .map(|r| EdgeId(r.get::<_, i64>(0) as u64))
            .collect())
    }

    fn delete_edge(&mut self, id: EdgeId) -> Result<bool> {
        let n = self
            .client
            .borrow_mut()
            .execute("DELETE FROM edges WHERE id = $1", &[&(id.0 as i64)])
            .map_err(pg_err)?;
        Ok(n > 0)
    }

    fn put_episode(&mut self, episode: Episode) -> Result<()> {
        let entity_ids_raw: Vec<u64> = episode.entity_ids.iter().map(|e| e.0).collect();
        let fact_ids_raw: Vec<u64> = episode.fact_ids.iter().map(|e| e.0).collect();

        self.client
            .borrow_mut()
            .execute(
                "INSERT INTO episodes \
                 (id, source, session_id, entity_ids, fact_ids, created_at, consolidation_count) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (id) DO UPDATE SET \
                     source = EXCLUDED.source, \
                     session_id = EXCLUDED.session_id, \
                     entity_ids = EXCLUDED.entity_ids, \
                     fact_ids = EXCLUDED.fact_ids, \
                     created_at = EXCLUDED.created_at, \
                     consolidation_count = EXCLUDED.consolidation_count",
                &[
                    &(episode.id as i64),
                    &episode_source_to_str(&episode.source),
                    &episode.session_id,
                    &serialize_u64_vec(&entity_ids_raw),
                    &serialize_u64_vec(&fact_ids_raw),
                    &episode.created_at,
                    &(episode.consolidation_count as i32),
                ],
            )
            .map_err(pg_err)?;
        Ok(())
    }

    fn get_episode(&self, id: u64) -> Result<Option<Episode>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, source, session_id, entity_ids, fact_ids, \
                 created_at, consolidation_count \
                 FROM episodes WHERE id = $1",
                &[&(id as i64)],
            )
            .map_err(pg_err)?;

        Ok(rows.first().map(row_to_episode))
    }

    fn update_episode_consolidation(&mut self, id: u64, count: u32) -> Result<bool> {
        let n = self
            .client
            .borrow_mut()
            .execute(
                "UPDATE episodes SET consolidation_count = $2 WHERE id = $1",
                &[&(id as i64), &(count as i32)],
            )
            .map_err(pg_err)?;
        Ok(n > 0)
    }

    fn scan_all_entities(&self) -> Result<Vec<Entity>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, entity_type, name, properties, embedding, created_at FROM entities",
                &[],
            )
            .map_err(pg_err)?;

        Ok(rows.iter().map(row_to_entity).collect())
    }

    fn scan_all_edges(&self) -> Result<Vec<Edge>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, source_id, target_id, relation_type, description, \
                 confidence, valid_at, invalid_at, created_at FROM edges",
                &[],
            )
            .map_err(pg_err)?;

        Ok(rows.iter().map(row_to_edge).collect())
    }

    fn scan_all_episodes(&self) -> Result<Vec<Episode>> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT id, source, session_id, entity_ids, fact_ids, \
                 created_at, consolidation_count FROM episodes",
                &[],
            )
            .map_err(pg_err)?;

        Ok(rows.iter().map(row_to_episode).collect())
    }

    fn stats(&self) -> StorageStats {
        let count = |table: &str| -> u64 {
            self.client
                .borrow_mut()
                .query(&format!("SELECT COUNT(*) FROM {}", table), &[])
                .ok()
                .and_then(|rows| rows.first().map(|r| r.get::<_, i64>(0) as u64))
                .unwrap_or(0)
        };

        StorageStats {
            entities: count("entities"),
            edges: count("edges"),
            episodes: count("episodes"),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────
//
// These tests require a running PostgreSQL instance. Set the
// `TEST_POSTGRES_URL` environment variable to enable them, e.g.:
//
//   TEST_POSTGRES_URL="host=localhost user=hora dbname=hora_test" cargo test --features postgres
//
// The tests create and DROP tables, so use a dedicated test database.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::PropertyValue;
    use std::collections::HashMap;

    fn test_url() -> Option<String> {
        std::env::var("TEST_POSTGRES_URL").ok()
    }

    /// Connect and wipe all tables for a clean test run.
    fn setup() -> Option<PostgresStorage> {
        let url = test_url()?;
        let storage = PostgresStorage::connect(&url).ok()?;
        // Clean slate
        storage
            .execute_batch("DELETE FROM edges; DELETE FROM episodes; DELETE FROM entities;")
            .ok()?;
        Some(storage)
    }

    fn make_entity(id: u64, name: &str, etype: &str) -> Entity {
        Entity {
            id: EntityId(id),
            entity_type: etype.to_string(),
            name: name.to_string(),
            properties: HashMap::new(),
            embedding: None,
            created_at: 1000,
        }
    }

    fn make_edge(id: u64, source: u64, target: u64) -> Edge {
        Edge {
            id: EdgeId(id),
            source: EntityId(source),
            target: EntityId(target),
            relation_type: "related_to".to_string(),
            description: "test edge".to_string(),
            confidence: 1.0,
            valid_at: 1000,
            invalid_at: 0,
            created_at: 1000,
        }
    }

    fn make_episode(id: u64) -> Episode {
        Episode {
            id,
            source: EpisodeSource::Conversation,
            session_id: "sess-1".to_string(),
            entity_ids: vec![EntityId(1), EntityId(2)],
            fact_ids: vec![EdgeId(10), EdgeId(20)],
            created_at: 2000,
            consolidation_count: 0,
        }
    }

    // --- Entity CRUD ---

    #[test]
    fn put_and_get_entity() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "rust", "language")).unwrap();

        let got = s.get_entity(EntityId(1)).unwrap().unwrap();
        assert_eq!(got.name, "rust");
        assert_eq!(got.entity_type, "language");
        assert_eq!(got.created_at, 1000);
    }

    #[test]
    fn get_entity_not_found() {
        let Some(s) = setup() else { return };
        assert!(s.get_entity(EntityId(999)).unwrap().is_none());
    }

    #[test]
    fn put_entity_replaces_existing() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "rust", "language")).unwrap();
        s.put_entity(make_entity(1, "rust-lang", "programming"))
            .unwrap();

        let got = s.get_entity(EntityId(1)).unwrap().unwrap();
        assert_eq!(got.name, "rust-lang");
        assert_eq!(got.entity_type, "programming");
    }

    #[test]
    fn delete_entity_existing() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "rust", "language")).unwrap();
        assert!(s.delete_entity(EntityId(1)).unwrap());
        assert!(s.get_entity(EntityId(1)).unwrap().is_none());
    }

    #[test]
    fn delete_entity_not_found() {
        let Some(mut s) = setup() else { return };
        assert!(!s.delete_entity(EntityId(999)).unwrap());
    }

    // --- Properties round-trip ---

    #[test]
    fn entity_with_properties() {
        let Some(mut s) = setup() else { return };
        let mut props = HashMap::new();
        props.insert("language".to_string(), PropertyValue::String("Rust".into()));
        props.insert("stars".to_string(), PropertyValue::Int(42));
        props.insert("score".to_string(), PropertyValue::Float(9.5));
        props.insert("active".to_string(), PropertyValue::Bool(true));

        let mut e = make_entity(1, "hora", "project");
        e.properties = props;
        s.put_entity(e).unwrap();

        let got = s.get_entity(EntityId(1)).unwrap().unwrap();
        assert_eq!(
            got.properties.get("language"),
            Some(&PropertyValue::String("Rust".into()))
        );
        assert_eq!(got.properties.get("stars"), Some(&PropertyValue::Int(42)));
        assert_eq!(
            got.properties.get("score"),
            Some(&PropertyValue::Float(9.5))
        );
        assert_eq!(
            got.properties.get("active"),
            Some(&PropertyValue::Bool(true))
        );
    }

    // --- Embedding round-trip ---

    #[test]
    fn entity_with_embedding() {
        let Some(mut s) = setup() else { return };
        let mut e = make_entity(1, "vec", "test");
        e.embedding = Some(vec![1.0, 2.0, 3.0, -0.5]);
        s.put_entity(e).unwrap();

        let got = s.get_entity(EntityId(1)).unwrap().unwrap();
        assert_eq!(got.embedding.unwrap(), vec![1.0, 2.0, 3.0, -0.5]);
    }

    #[test]
    fn entity_without_embedding() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "no-vec", "test")).unwrap();

        let got = s.get_entity(EntityId(1)).unwrap().unwrap();
        assert!(got.embedding.is_none());
    }

    // --- Edge CRUD ---

    #[test]
    fn put_and_get_edge() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();
        s.put_edge(make_edge(10, 1, 2)).unwrap();

        let got = s.get_edge(EdgeId(10)).unwrap().unwrap();
        assert_eq!(got.source, EntityId(1));
        assert_eq!(got.target, EntityId(2));
        assert_eq!(got.relation_type, "related_to");
    }

    #[test]
    fn get_edge_not_found() {
        let Some(s) = setup() else { return };
        assert!(s.get_edge(EdgeId(999)).unwrap().is_none());
    }

    #[test]
    fn delete_edge() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();
        s.put_edge(make_edge(10, 1, 2)).unwrap();

        assert!(s.delete_edge(EdgeId(10)).unwrap());
        assert!(s.get_edge(EdgeId(10)).unwrap().is_none());
        assert!(!s.delete_edge(EdgeId(10)).unwrap());
    }

    // --- Entity edges ---

    #[test]
    fn get_entity_edges_both_directions() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();
        s.put_entity(make_entity(3, "c", "node")).unwrap();
        s.put_edge(make_edge(10, 1, 2)).unwrap();
        s.put_edge(make_edge(11, 3, 1)).unwrap();

        assert_eq!(s.get_entity_edges(EntityId(1)).unwrap().len(), 2);
        assert_eq!(s.get_entity_edge_ids(EntityId(1)).unwrap().len(), 2);
    }

    #[test]
    fn get_entity_edges_empty() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "lonely", "node")).unwrap();
        assert!(s.get_entity_edges(EntityId(1)).unwrap().is_empty());
        assert!(s.get_entity_edge_ids(EntityId(1)).unwrap().is_empty());
    }

    // --- Episode CRUD ---

    #[test]
    fn put_and_get_episode() {
        let Some(mut s) = setup() else { return };
        s.put_episode(make_episode(1)).unwrap();

        let got = s.get_episode(1).unwrap().unwrap();
        assert_eq!(got.id, 1);
        assert_eq!(got.source, EpisodeSource::Conversation);
        assert_eq!(got.session_id, "sess-1");
        assert_eq!(got.entity_ids, vec![EntityId(1), EntityId(2)]);
        assert_eq!(got.fact_ids, vec![EdgeId(10), EdgeId(20)]);
        assert_eq!(got.consolidation_count, 0);
    }

    #[test]
    fn get_episode_not_found() {
        let Some(s) = setup() else { return };
        assert!(s.get_episode(999).unwrap().is_none());
    }

    #[test]
    fn update_episode_consolidation() {
        let Some(mut s) = setup() else { return };
        s.put_episode(make_episode(1)).unwrap();
        assert!(s.update_episode_consolidation(1, 5).unwrap());

        let got = s.get_episode(1).unwrap().unwrap();
        assert_eq!(got.consolidation_count, 5);
    }

    #[test]
    fn update_episode_consolidation_not_found() {
        let Some(mut s) = setup() else { return };
        assert!(!s.update_episode_consolidation(999, 1).unwrap());
    }

    // --- Episode source variants ---

    #[test]
    fn episode_source_round_trip() {
        let Some(mut s) = setup() else { return };
        for (id, source) in [
            (1, EpisodeSource::Conversation),
            (2, EpisodeSource::Document),
            (3, EpisodeSource::Api),
        ] {
            let mut ep = make_episode(id);
            ep.source = source.clone();
            s.put_episode(ep).unwrap();
            let got = s.get_episode(id).unwrap().unwrap();
            assert_eq!(got.source, source);
        }
    }

    // --- Scan ---

    #[test]
    fn scan_all_entities() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();
        assert_eq!(s.scan_all_entities().unwrap().len(), 2);
    }

    #[test]
    fn scan_all_edges() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();
        s.put_edge(make_edge(10, 1, 2)).unwrap();
        s.put_edge(make_edge(11, 2, 1)).unwrap();
        assert_eq!(s.scan_all_edges().unwrap().len(), 2);
    }

    #[test]
    fn scan_all_episodes() {
        let Some(mut s) = setup() else { return };
        s.put_episode(make_episode(1)).unwrap();
        s.put_episode(make_episode(2)).unwrap();
        assert_eq!(s.scan_all_episodes().unwrap().len(), 2);
    }

    // --- Stats ---

    #[test]
    fn stats_counts() {
        let Some(mut s) = setup() else { return };
        assert_eq!(
            s.stats(),
            StorageStats {
                entities: 0,
                edges: 0,
                episodes: 0
            }
        );

        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();
        s.put_edge(make_edge(10, 1, 2)).unwrap();
        s.put_episode(make_episode(1)).unwrap();

        assert_eq!(
            s.stats(),
            StorageStats {
                entities: 2,
                edges: 1,
                episodes: 1
            }
        );
    }

    // --- FTS (tsvector) ---

    #[test]
    fn fts_search_basic() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "authentication service", "service"))
            .unwrap();
        s.put_entity(make_entity(2, "user database", "database"))
            .unwrap();
        s.put_entity(make_entity(3, "auth middleware", "service"))
            .unwrap();

        // tsvector 'simple' uses prefix matching with :*
        let hits = s.fts_search("auth:*", 10).unwrap();
        assert_eq!(hits.len(), 2);
        let ids: Vec<u64> = hits.iter().map(|e| e.0).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
    }

    #[test]
    fn fts_search_after_delete() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "authentication", "service"))
            .unwrap();
        s.delete_entity(EntityId(1)).unwrap();

        let hits = s.fts_search("authentication", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn fts_search_after_update() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "old name", "service")).unwrap();
        s.put_entity(make_entity(1, "new name", "service")).unwrap();

        assert!(s.fts_search("old", 10).unwrap().is_empty());
        assert_eq!(s.fts_search("new", 10).unwrap().len(), 1);
    }

    // --- Bi-temporal edges ---

    #[test]
    fn edge_temporal_fields() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "a", "node")).unwrap();
        s.put_entity(make_entity(2, "b", "node")).unwrap();

        let mut edge = make_edge(10, 1, 2);
        edge.valid_at = 1000;
        edge.invalid_at = 2000;
        edge.confidence = 0.85;
        s.put_edge(edge).unwrap();

        let got = s.get_edge(EdgeId(10)).unwrap().unwrap();
        assert_eq!(got.valid_at, 1000);
        assert_eq!(got.invalid_at, 2000);
        assert!((got.confidence - 0.85).abs() < f32::EPSILON);
    }

    // --- Self-referencing edge ---

    #[test]
    fn self_referencing_edge() {
        let Some(mut s) = setup() else { return };
        s.put_entity(make_entity(1, "self", "node")).unwrap();
        s.put_edge(make_edge(10, 1, 1)).unwrap();

        let edges = s.get_entity_edges(EntityId(1)).unwrap();
        assert_eq!(edges.len(), 1);
    }
}
