//! Core domain types for the hora-graph-core knowledge graph engine.
//!
//! This module re-exports [`entity`], [`edge`], [`episode`], [`types`], and [`dedup`]
//! as the foundational building blocks of the graph.

pub mod dedup;
pub mod edge;
pub mod entity;
pub mod episode;
pub mod types;
