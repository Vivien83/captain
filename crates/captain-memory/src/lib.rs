//! Memory substrate for the Captain Agent Operating System.
//!
//! Provides a unified memory API over three storage backends:
//! - **Structured store** (SQLite): Key-value pairs, sessions, agent state
//! - **Semantic store**: Text-based search (Phase 1: LIKE matching, Phase 2: Qdrant vectors)
//! - **Knowledge graph** (SQLite): Entities and relations
//!
//! Agents interact with a single `Memory` trait that abstracts over all three stores.

pub mod consolidation;
pub mod detached_tool_runs;
pub mod event_log;
pub mod knowledge;
pub mod learning_review;
pub mod memory_capsule;
pub mod memory_writer;
pub mod migration;
pub mod milestone;
pub mod project;
pub mod project_checkpoint;
pub mod project_task;
pub mod semantic;
pub mod session;
pub mod skill_patterns;
pub mod skill_proposals;
pub mod structured;
pub mod todo;
pub mod usage;

mod substrate;
pub use substrate::MemorySubstrate;
