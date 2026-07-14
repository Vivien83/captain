//! Binary serialization format for .hora files.
//!
//! ## Header layout (48 bytes)
//!
//! ```text
//! Offset  Size  Field               Notes
//! ──────  ────  ──────────────────  ──────────────────────────────────
//! [0..4]    4   magic               "HORA" (0x484F5241)
//! [4..6]    2   format_version      u16 LE — v1 (no checksum), v2 (with checksum)
//! [6..8]    2   embedding_dims      u16 LE — 0 = text-only mode
//! [8..16]   8   next_entity_id      u64 LE — next auto-increment
//! [16..24]  8   next_edge_id        u64 LE
//! [24..32]  8   next_episode_id     u64 LE
//! [32..36]  4   entity_count        u32 LE
//! [36..40]  4   edge_count          u32 LE
//! [40..44]  4   episode_count       u32 LE
//! [44..48]  4   header_checksum     u32 LE — CRC32-IEEE of bytes [0..44] (v2+; zero in v1)
//! ```
//!
//! ## Body
//!
//! ```text
//! [Entity]*entity_count
//! [Edge]*edge_count
//! [Episode]*episode_count
//! ```
//!
//! All multi-byte integers are little-endian. Strings are length-prefixed (u32 LE + UTF-8 bytes).
//!
//! ## Entity layout
//!
//! ```text
//! id: u64, entity_type: str, name: str, properties: props,
//! has_embedding: u8 (0|1), [embedding_len: u32, f32*embedding_len], created_at: i64
//! ```
//!
//! ## Edge layout
//!
//! ```text
//! id: u64, source: u64, target: u64, relation_type: str, description: str,
//! confidence: f32, valid_at: i64, invalid_at: i64, created_at: i64
//! ```
//!
//! ## Episode layout
//!
//! ```text
//! id: u64, source: u8, session_id: str,
//! entity_count: u32, entity_ids: u64*entity_count,
//! fact_count: u32, fact_ids: u64*fact_count,
//! created_at: i64, consolidation_count: u32
//! ```
//!
//! ## Properties layout
//!
//! ```text
//! count: u32, (key: str, tag: u8, value)*count
//! Tags: 0=String(str), 1=Int(i64), 2=Float(f64), 3=Bool(u8)
//! ```
//!
//! ## Version history
//!
//! - **v1**: Original format. No header checksum (bytes [44..48] = reserved zeros).
//! - **v2**: Added CRC32-IEEE header checksum. Body format unchanged.
//!   Files written as v1 are readable (checksum skipped). On next `flush()`, written as v2.

use std::io::{self, Read, Write};
use std::path::Path;

use crate::core::edge::Edge;
use crate::core::entity::Entity;
use crate::core::episode::Episode;
use crate::core::types::{EdgeId, EntityId, EpisodeSource, Properties, PropertyValue};
use crate::error::{HoraError, Result};
use crate::storage::embedded::page::crc32;

const MAGIC: [u8; 4] = *b"HORA";
const FORMAT_VERSION: u16 = 2;
const HEADER_SIZE: usize = 48;

/// Metadata stored in the file header.
#[derive(Debug)]
pub struct FileHeader {
    /// Dimensionality of stored embeddings; 0 means text-only mode.
    pub embedding_dims: u16,
    /// Next auto-increment value for entity IDs.
    pub next_entity_id: u64,
    /// Next auto-increment value for edge IDs.
    pub next_edge_id: u64,
    /// Next auto-increment value for episode IDs.
    pub next_episode_id: u64,
    /// Number of entities serialized in the body.
    pub entity_count: u32,
    /// Number of edges serialized in the body.
    pub edge_count: u32,
    /// Number of episodes serialized in the body.
    pub episode_count: u32,
}

// ── Write helpers ──────────────────────────────────────────

fn write_u8(w: &mut impl Write, v: u8) -> io::Result<()> {
    w.write_all(&[v])
}
fn write_u32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_u64(w: &mut impl Write, v: u64) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_i64(w: &mut impl Write, v: i64) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_f32(w: &mut impl Write, v: f32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_str(w: &mut impl Write, s: &str) -> io::Result<()> {
    write_u32(w, s.len() as u32)?;
    w.write_all(s.as_bytes())
}

// ── Read helpers ───────────────────────────────────────────

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}
fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}
fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}
fn read_i64(r: &mut impl Read) -> io::Result<i64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(i64::from_le_bytes(buf))
}
fn read_f32(r: &mut impl Read) -> io::Result<f32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(f32::from_le_bytes(buf))
}
/// Maximum single allocation from file data (256 MB).
const MAX_ALLOC: usize = 256 * 1024 * 1024;

fn read_string(r: &mut impl Read) -> io::Result<String> {
    let len = read_u32(r)? as usize;
    if len > MAX_ALLOC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("string length {} exceeds max {}", len, MAX_ALLOC),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ── PropertyValue tags ─────────────────────────────────────

const TAG_STRING: u8 = 0;
const TAG_INT: u8 = 1;
const TAG_FLOAT: u8 = 2;
const TAG_BOOL: u8 = 3;

fn write_property_value(w: &mut impl Write, v: &PropertyValue) -> io::Result<()> {
    match v {
        PropertyValue::String(s) => {
            write_u8(w, TAG_STRING)?;
            write_str(w, s)?;
        }
        PropertyValue::Int(n) => {
            write_u8(w, TAG_INT)?;
            write_i64(w, *n)?;
        }
        PropertyValue::Float(f) => {
            write_u8(w, TAG_FLOAT)?;
            w.write_all(&f.to_le_bytes())?;
        }
        PropertyValue::Bool(b) => {
            write_u8(w, TAG_BOOL)?;
            write_u8(w, u8::from(*b))?;
        }
    }
    Ok(())
}

fn read_property_value(r: &mut impl Read) -> io::Result<PropertyValue> {
    let tag = read_u8(r)?;
    match tag {
        TAG_STRING => Ok(PropertyValue::String(read_string(r)?)),
        TAG_INT => Ok(PropertyValue::Int(read_i64(r)?)),
        TAG_FLOAT => {
            let mut buf = [0u8; 8];
            r.read_exact(&mut buf)?;
            Ok(PropertyValue::Float(f64::from_le_bytes(buf)))
        }
        TAG_BOOL => {
            let b = read_u8(r)?;
            Ok(PropertyValue::Bool(b != 0))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown property tag: {}", tag),
        )),
    }
}

pub(crate) fn write_properties(w: &mut impl Write, props: &Properties) -> io::Result<()> {
    write_u32(w, props.len() as u32)?;
    for (key, val) in props {
        write_str(w, key)?;
        write_property_value(w, val)?;
    }
    Ok(())
}

pub(crate) fn read_properties(r: &mut impl Read) -> io::Result<Properties> {
    let count = read_u32(r)? as usize;
    if count > MAX_ALLOC / 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("property count {} exceeds max", count),
        ));
    }
    let mut map = Properties::with_capacity(count);
    for _ in 0..count {
        let key = read_string(r)?;
        let val = read_property_value(r)?;
        map.insert(key, val);
    }
    Ok(map)
}

// ── EpisodeSource ──────────────────────────────────────────

fn episode_source_to_u8(s: &EpisodeSource) -> u8 {
    match s {
        EpisodeSource::Conversation => 0,
        EpisodeSource::Document => 1,
        EpisodeSource::Api => 2,
    }
}

fn u8_to_episode_source(v: u8) -> io::Result<EpisodeSource> {
    match v {
        0 => Ok(EpisodeSource::Conversation),
        1 => Ok(EpisodeSource::Document),
        2 => Ok(EpisodeSource::Api),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown episode source: {}", v),
        )),
    }
}

// ── Entity ─────────────────────────────────────────────────

fn write_entity(w: &mut impl Write, e: &Entity) -> io::Result<()> {
    write_u64(w, e.id.0)?;
    write_str(w, &e.entity_type)?;
    write_str(w, &e.name)?;
    write_properties(w, &e.properties)?;
    match &e.embedding {
        Some(emb) => {
            write_u8(w, 1)?;
            write_u32(w, emb.len() as u32)?;
            let mut bytes = vec![0u8; emb.len() * 4];
            for (i, &v) in emb.iter().enumerate() {
                bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
            }
            w.write_all(&bytes)?;
        }
        None => {
            write_u8(w, 0)?;
        }
    }
    write_i64(w, e.created_at)?;
    Ok(())
}

fn read_entity(r: &mut impl Read) -> io::Result<Entity> {
    let id = EntityId(read_u64(r)?);
    let entity_type = read_string(r)?;
    let name = read_string(r)?;
    let properties = read_properties(r)?;
    let has_embedding = read_u8(r)?;
    let embedding = if has_embedding != 0 {
        let len = read_u32(r)? as usize;
        let byte_len = len.checked_mul(4).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "embedding length overflow")
        })?;
        if byte_len > MAX_ALLOC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("embedding size {} exceeds max {}", byte_len, MAX_ALLOC),
            ));
        }
        let mut bytes = vec![0u8; byte_len];
        r.read_exact(&mut bytes)?;
        let emb: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Some(emb)
    } else {
        None
    };
    let created_at = read_i64(r)?;
    Ok(Entity {
        id,
        entity_type,
        name,
        properties,
        embedding,
        created_at,
    })
}

// ── Edge ───────────────────────────────────────────────────

fn write_edge(w: &mut impl Write, e: &Edge) -> io::Result<()> {
    write_u64(w, e.id.0)?;
    write_u64(w, e.source.0)?;
    write_u64(w, e.target.0)?;
    write_str(w, &e.relation_type)?;
    write_str(w, &e.description)?;
    write_f32(w, e.confidence)?;
    write_i64(w, e.valid_at)?;
    write_i64(w, e.invalid_at)?;
    write_i64(w, e.created_at)?;
    Ok(())
}

fn read_edge(r: &mut impl Read) -> io::Result<Edge> {
    let id = EdgeId(read_u64(r)?);
    let source = EntityId(read_u64(r)?);
    let target = EntityId(read_u64(r)?);
    let relation_type = read_string(r)?;
    let description = read_string(r)?;
    let confidence = read_f32(r)?;
    let valid_at = read_i64(r)?;
    let invalid_at = read_i64(r)?;
    let created_at = read_i64(r)?;
    Ok(Edge {
        id,
        source,
        target,
        relation_type,
        description,
        confidence,
        valid_at,
        invalid_at,
        created_at,
    })
}

// ── Episode ────────────────────────────────────────────────

fn write_episode(w: &mut impl Write, ep: &Episode) -> io::Result<()> {
    write_u64(w, ep.id)?;
    write_u8(w, episode_source_to_u8(&ep.source))?;
    write_str(w, &ep.session_id)?;
    write_u32(w, ep.entity_ids.len() as u32)?;
    for eid in &ep.entity_ids {
        write_u64(w, eid.0)?;
    }
    write_u32(w, ep.fact_ids.len() as u32)?;
    for fid in &ep.fact_ids {
        write_u64(w, fid.0)?;
    }
    write_i64(w, ep.created_at)?;
    write_u32(w, ep.consolidation_count)?;
    Ok(())
}

fn read_episode(r: &mut impl Read) -> io::Result<Episode> {
    let id = read_u64(r)?;
    let source = u8_to_episode_source(read_u8(r)?)?;
    let session_id = read_string(r)?;
    let entity_count = read_u32(r)? as usize;
    let mut entity_ids = Vec::with_capacity(entity_count);
    for _ in 0..entity_count {
        entity_ids.push(EntityId(read_u64(r)?));
    }
    let fact_count = read_u32(r)? as usize;
    let mut fact_ids = Vec::with_capacity(fact_count);
    for _ in 0..fact_count {
        fact_ids.push(EdgeId(read_u64(r)?));
    }
    let created_at = read_i64(r)?;
    let consolidation_count = read_u32(r)?;
    Ok(Episode {
        id,
        source,
        session_id,
        entity_ids,
        fact_ids,
        created_at,
        consolidation_count,
    })
}

// ── Top-level serialize / deserialize ──────────────────────

/// Serialize a complete graph state to a writer.
pub fn serialize(
    w: &mut impl Write,
    header: &FileHeader,
    entities: &[Entity],
    edges: &[Edge],
    episodes: &[Episode],
) -> Result<()> {
    // Build the first 44 bytes into a buffer to compute CRC32
    let mut hdr_buf = [0u8; HEADER_SIZE];
    hdr_buf[0..4].copy_from_slice(&MAGIC);
    hdr_buf[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    hdr_buf[6..8].copy_from_slice(&header.embedding_dims.to_le_bytes());
    hdr_buf[8..16].copy_from_slice(&header.next_entity_id.to_le_bytes());
    hdr_buf[16..24].copy_from_slice(&header.next_edge_id.to_le_bytes());
    hdr_buf[24..32].copy_from_slice(&header.next_episode_id.to_le_bytes());
    hdr_buf[32..36].copy_from_slice(&header.entity_count.to_le_bytes());
    hdr_buf[36..40].copy_from_slice(&header.edge_count.to_le_bytes());
    hdr_buf[40..44].copy_from_slice(&header.episode_count.to_le_bytes());
    // CRC32 of bytes [0..44]
    let checksum = crc32(&hdr_buf[0..44]);
    hdr_buf[44..48].copy_from_slice(&checksum.to_le_bytes());

    w.write_all(&hdr_buf)?;

    for entity in entities {
        write_entity(w, entity)?;
    }
    for edge in edges {
        write_edge(w, edge)?;
    }
    for episode in episodes {
        write_episode(w, episode)?;
    }

    Ok(())
}

/// Returned data from deserializing a .hora file.
#[derive(Debug)]
pub struct DeserializedGraph {
    /// Parsed file header with ID counters and record counts.
    pub header: FileHeader,
    /// All deserialized entities from the body.
    pub entities: Vec<Entity>,
    /// All deserialized edges from the body.
    pub edges: Vec<Edge>,
    /// All deserialized episodes from the body.
    pub episodes: Vec<Episode>,
}

/// Deserialize a complete graph state from a reader.
///
/// Accepts format v1 (no checksum) and v2 (with CRC32 checksum).
/// v1 files are read without checksum verification.
pub fn deserialize(r: &mut impl Read) -> Result<DeserializedGraph> {
    // Read the full header as raw bytes for checksum verification
    let mut hdr_buf = [0u8; HEADER_SIZE];
    r.read_exact(&mut hdr_buf)?;

    // Magic
    if hdr_buf[0..4] != MAGIC {
        return Err(HoraError::InvalidFile {
            reason: "not a .hora file (bad magic)",
        });
    }

    // Version
    let version = u16::from_le_bytes([hdr_buf[4], hdr_buf[5]]);
    if version == 0 || version > FORMAT_VERSION {
        return Err(HoraError::VersionMismatch {
            file_version: version,
            min_supported: 1,
            max_supported: FORMAT_VERSION,
        });
    }

    // Checksum verification (v2+ only)
    if version >= 2 {
        let stored = u32::from_le_bytes(hdr_buf[44..48].try_into().unwrap());
        let computed = crc32(&hdr_buf[0..44]);
        if stored != computed {
            return Err(HoraError::InvalidFile {
                reason: "header checksum mismatch",
            });
        }
    }

    let embedding_dims = u16::from_le_bytes([hdr_buf[6], hdr_buf[7]]);
    let next_entity_id = u64::from_le_bytes(hdr_buf[8..16].try_into().unwrap());
    let next_edge_id = u64::from_le_bytes(hdr_buf[16..24].try_into().unwrap());
    let next_episode_id = u64::from_le_bytes(hdr_buf[24..32].try_into().unwrap());
    let entity_count = u32::from_le_bytes(hdr_buf[32..36].try_into().unwrap());
    let edge_count = u32::from_le_bytes(hdr_buf[36..40].try_into().unwrap());
    let episode_count = u32::from_le_bytes(hdr_buf[40..44].try_into().unwrap());

    let header = FileHeader {
        embedding_dims,
        next_entity_id,
        next_edge_id,
        next_episode_id,
        entity_count,
        edge_count,
        episode_count,
    };

    // Cap pre-allocation to avoid OOM from crafted headers (actual count still honoured)
    let cap = |n: u32| (n as usize).min(1_000_000);

    let mut entities = Vec::with_capacity(cap(entity_count));
    for _ in 0..entity_count {
        entities.push(read_entity(r)?);
    }

    let mut edges = Vec::with_capacity(cap(edge_count));
    for _ in 0..edge_count {
        edges.push(read_edge(r)?);
    }

    let mut episodes = Vec::with_capacity(cap(episode_count));
    for _ in 0..episode_count {
        episodes.push(read_episode(r)?);
    }

    Ok(DeserializedGraph {
        header,
        entities,
        edges,
        episodes,
    })
}

/// Result of verifying a .hora file.
#[derive(Debug)]
pub struct VerifyReport {
    /// File format version read from the header.
    pub format_version: u16,
    /// Number of entities declared in the header.
    pub entity_count: u32,
    /// Number of edges declared in the header.
    pub edge_count: u32,
    /// Number of episodes declared in the header.
    pub episode_count: u32,
    /// Embedding dimensionality declared in the header.
    pub embedding_dims: u16,
    /// Whether the CRC32 header checksum was present and matched (false for v1 files).
    pub checksum_verified: bool,
}

/// Verify the integrity of a .hora file without loading all data.
pub fn verify_file(path: impl AsRef<Path>) -> Result<VerifyReport> {
    let data = std::fs::read(path.as_ref())?;
    if data.len() < HEADER_SIZE {
        return Err(HoraError::InvalidFile {
            reason: "file too short for header",
        });
    }

    if data[0..4] != MAGIC {
        return Err(HoraError::InvalidFile {
            reason: "not a .hora file (bad magic)",
        });
    }

    let version = u16::from_le_bytes([data[4], data[5]]);
    if version == 0 || version > FORMAT_VERSION {
        return Err(HoraError::VersionMismatch {
            file_version: version,
            min_supported: 1,
            max_supported: FORMAT_VERSION,
        });
    }

    let checksum_verified = if version >= 2 {
        let stored = u32::from_le_bytes(data[44..48].try_into().unwrap());
        let computed = crc32(&data[0..44]);
        if stored != computed {
            return Err(HoraError::InvalidFile {
                reason: "header checksum mismatch",
            });
        }
        true
    } else {
        false
    };

    // Try to fully deserialize to verify body integrity
    let mut cursor = io::Cursor::new(&data);
    let graph = deserialize(&mut cursor)?;

    Ok(VerifyReport {
        format_version: version,
        entity_count: graph.header.entity_count,
        edge_count: graph.header.edge_count,
        episode_count: graph.header.episode_count,
        embedding_dims: graph.header.embedding_dims,
        checksum_verified,
    })
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_entity(id: u64) -> Entity {
        let mut props = HashMap::new();
        props.insert("lang".to_string(), PropertyValue::String("Rust".into()));
        props.insert("stars".to_string(), PropertyValue::Int(42));
        props.insert("score".to_string(), PropertyValue::Float(9.5));
        props.insert("active".to_string(), PropertyValue::Bool(true));
        Entity {
            id: EntityId(id),
            entity_type: "project".to_string(),
            name: format!("project_{}", id),
            properties: props,
            embedding: Some(vec![0.1, 0.2, 0.3]),
            created_at: 1000,
        }
    }

    fn sample_edge(id: u64, src: u64, tgt: u64) -> Edge {
        Edge {
            id: EdgeId(id),
            source: EntityId(src),
            target: EntityId(tgt),
            relation_type: "related_to".to_string(),
            description: "test relation".to_string(),
            confidence: 0.95,
            valid_at: 1000,
            invalid_at: 0,
            created_at: 1000,
        }
    }

    fn sample_episode(id: u64) -> Episode {
        Episode {
            id,
            source: EpisodeSource::Conversation,
            session_id: "sess_001".to_string(),
            entity_ids: vec![EntityId(1), EntityId(2)],
            fact_ids: vec![EdgeId(1)],
            created_at: 2000,
            consolidation_count: 0,
        }
    }

    fn sample_header(entities: u32, edges: u32, episodes: u32) -> FileHeader {
        FileHeader {
            embedding_dims: 3,
            next_entity_id: entities as u64 + 1,
            next_edge_id: edges as u64 + 1,
            next_episode_id: episodes as u64 + 1,
            entity_count: entities,
            edge_count: edges,
            episode_count: episodes,
        }
    }

    #[test]
    fn test_roundtrip_v2() {
        let entities = vec![sample_entity(1), sample_entity(2)];
        let edges = vec![sample_edge(1, 1, 2)];
        let episodes = vec![sample_episode(1)];
        let header = sample_header(2, 1, 1);

        let mut buf = Vec::new();
        serialize(&mut buf, &header, &entities, &edges, &episodes).unwrap();

        // Verify header is v2
        assert_eq!(buf[0..4], *b"HORA");
        assert_eq!(u16::from_le_bytes([buf[4], buf[5]]), 2);

        // Verify checksum is non-zero
        let checksum = u32::from_le_bytes(buf[44..48].try_into().unwrap());
        assert_ne!(checksum, 0);

        // Deserialize
        let mut cursor = io::Cursor::new(&buf);
        let graph = deserialize(&mut cursor).unwrap();
        assert_eq!(graph.header.entity_count, 2);
        assert_eq!(graph.header.edge_count, 1);
        assert_eq!(graph.header.episode_count, 1);
        assert_eq!(graph.entities.len(), 2);
        assert_eq!(graph.entities[0].name, "project_1");
        assert_eq!(graph.edges[0].relation_type, "related_to");
        assert_eq!(graph.episodes[0].session_id, "sess_001");
    }

    #[test]
    fn test_checksum_detects_corruption() {
        let entities = vec![sample_entity(1)];
        let header = sample_header(1, 0, 0);

        let mut buf = Vec::new();
        serialize(&mut buf, &header, &entities, &[], &[]).unwrap();

        // Corrupt a byte in the header (entity_count field)
        buf[32] = 0xFF;

        let mut cursor = io::Cursor::new(&buf);
        let result = deserialize(&mut cursor);
        assert!(result.is_err());
        match result.unwrap_err() {
            HoraError::InvalidFile { reason } => {
                assert!(reason.contains("checksum"));
            }
            other => panic!("expected InvalidFile, got {:?}", other),
        }
    }

    #[test]
    fn test_v1_backward_compat() {
        // Build a v1 file manually (same layout, version=1, checksum=0)
        let entities = vec![sample_entity(1)];
        let header = sample_header(1, 0, 0);

        let mut buf = Vec::new();
        serialize(&mut buf, &header, &entities, &[], &[]).unwrap();

        // Patch version to 1 and zero out checksum (simulating old v1 file)
        buf[4] = 1;
        buf[5] = 0;
        buf[44] = 0;
        buf[45] = 0;
        buf[46] = 0;
        buf[47] = 0;

        // Should still deserialize (v1 = no checksum check)
        let mut cursor = io::Cursor::new(&buf);
        let graph = deserialize(&mut cursor).unwrap();
        assert_eq!(graph.header.entity_count, 1);
        assert_eq!(graph.entities[0].name, "project_1");
    }

    #[test]
    fn test_bad_magic_rejected() {
        let mut buf = vec![0u8; 48];
        buf[0..4].copy_from_slice(b"NOPE");
        let mut cursor = io::Cursor::new(&buf);
        let result = deserialize(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_future_version_rejected() {
        let mut buf = vec![0u8; 48];
        buf[0..4].copy_from_slice(b"HORA");
        buf[4..6].copy_from_slice(&99u16.to_le_bytes());
        let mut cursor = io::Cursor::new(&buf);
        let result = deserialize(&mut cursor);
        match result.unwrap_err() {
            HoraError::VersionMismatch {
                file_version,
                min_supported,
                max_supported,
            } => {
                assert_eq!(file_version, 99);
                assert_eq!(min_supported, 1);
                assert_eq!(max_supported, 2);
            }
            other => panic!("expected VersionMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_verify_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let entities = vec![sample_entity(1), sample_entity(2)];
        let edges = vec![sample_edge(1, 1, 2)];
        let episodes = vec![sample_episode(1)];
        let header = sample_header(2, 1, 1);

        let mut buf = Vec::new();
        serialize(&mut buf, &header, &entities, &edges, &episodes).unwrap();
        std::fs::write(&path, &buf).unwrap();

        let report = verify_file(&path).unwrap();
        assert_eq!(report.format_version, 2);
        assert_eq!(report.entity_count, 2);
        assert_eq!(report.edge_count, 1);
        assert_eq!(report.episode_count, 1);
        assert_eq!(report.embedding_dims, 3);
        assert!(report.checksum_verified);
    }

    #[test]
    fn test_verify_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.hora");

        let entities = vec![sample_entity(1)];
        let header = sample_header(1, 0, 0);

        let mut buf = Vec::new();
        serialize(&mut buf, &header, &entities, &[], &[]).unwrap();

        // Corrupt one byte in header
        buf[10] = 0xFF;
        std::fs::write(&path, &buf).unwrap();

        let result = verify_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_graph_roundtrip() {
        let header = FileHeader {
            embedding_dims: 0,
            next_entity_id: 1,
            next_edge_id: 1,
            next_episode_id: 1,
            entity_count: 0,
            edge_count: 0,
            episode_count: 0,
        };

        let mut buf = Vec::new();
        serialize(&mut buf, &header, &[], &[], &[]).unwrap();
        assert_eq!(buf.len(), HEADER_SIZE);

        let mut cursor = io::Cursor::new(&buf);
        let graph = deserialize(&mut cursor).unwrap();
        assert_eq!(graph.entities.len(), 0);
        assert_eq!(graph.edges.len(), 0);
        assert_eq!(graph.episodes.len(), 0);
    }

    #[test]
    fn test_reference_file_generation() {
        // Generate a reference file and verify it can be read back identically.
        // This test creates a deterministic graph and verifies byte-level stability.
        let entities = vec![sample_entity(1), sample_entity(2)];
        let edges = vec![sample_edge(1, 1, 2)];
        let episodes = vec![sample_episode(1)];
        let header = sample_header(2, 1, 1);

        let mut buf1 = Vec::new();
        serialize(&mut buf1, &header, &entities, &edges, &episodes).unwrap();

        let mut buf2 = Vec::new();
        serialize(&mut buf2, &header, &entities, &edges, &episodes).unwrap();

        // Same input = same output (deterministic)
        assert_eq!(buf1, buf2);

        // Verify the header checksum matches
        let stored_crc = u32::from_le_bytes(buf1[44..48].try_into().unwrap());
        let computed_crc = crc32(&buf1[0..44]);
        assert_eq!(stored_crc, computed_crc);
    }
}
