//! Compatibility alias for the standalone Node's lightweight local driver.

#[cfg(test)]
pub(crate) use captain_node::NodeLocalToolDriver as CliNodeToolDriver;

#[cfg(test)]
#[path = "node_runtime_tests.rs"]
mod tests;
