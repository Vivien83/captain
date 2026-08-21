//! Lightweight, workspace-confined tool engine shared by Captain Node.
//!
//! This crate contains no agent loop, provider, memory, channels, skills or
//! Full runtime state. Its public surface is deliberately limited to the tool
//! catalog that can cross the durable Hub/Node rail.

pub mod apply_patch;
pub mod edit_strategies;
pub mod node_tool_runtime;
pub mod workspace_sandbox;

mod effect;
mod guarded_exec;
mod kernel_handle;
mod output_security;
mod shell_exec;
mod shell_guard;
mod tool_dispatch;
mod tools;
