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
pub mod provider_quota;
pub mod semantic;
pub mod session;
pub mod structured;
pub mod todo;
pub mod usage;
pub mod workflow_learning;
pub mod workflow_learning_control;
pub mod workflow_learning_installation;
mod workflow_learning_installation_types;
mod workflow_learning_installation_validation;
pub mod workflow_learning_lifecycle;
mod workflow_learning_lifecycle_types;
mod workflow_learning_lifecycle_validation;
pub mod workflow_learning_outbox;
pub mod workflow_learning_pipeline;
mod workflow_learning_pipeline_types;
mod workflow_learning_pipeline_validation;
pub mod workflow_learning_queue;
pub mod workflow_learning_refinement;
pub mod workflow_learning_refinement_capture;
pub mod workflow_learning_refinement_lifecycle;
mod workflow_learning_refinement_types;
pub mod workflow_learning_snooze;
pub mod workflow_learning_test;
mod workflow_learning_types;
mod workflow_learning_validation;

#[cfg(test)]
mod workflow_learning_control_tests;
#[cfg(test)]
mod workflow_learning_installation_tests;
#[cfg(test)]
mod workflow_learning_lifecycle_tests;
#[cfg(test)]
mod workflow_learning_outbox_tests;
#[cfg(test)]
mod workflow_learning_pipeline_tests;
#[cfg(test)]
mod workflow_learning_queue_tests;
#[cfg(test)]
mod workflow_learning_refinement_capture_tests;
#[cfg(test)]
mod workflow_learning_refinement_lifecycle_tests;
#[cfg(test)]
mod workflow_learning_refinement_tests;
#[cfg(test)]
mod workflow_learning_snooze_tests;
#[cfg(test)]
mod workflow_learning_test_tests;

mod substrate;
pub use substrate::MemorySubstrate;
