//! Active tool registry facade.
//!
//! The historical builtin registry still lives in `tool_runner.rs`. This small
//! facade is the migration point toward one family per file while keeping the
//! current dispatch path stable.

use captain_types::tool::ToolDefinition;

#[derive(Debug, Clone)]
pub struct ToolRegistry {
    definitions: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub fn new(definitions: Vec<ToolDefinition>) -> Self {
        Self { definitions }
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn discoverable_definitions(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.definitions
            .iter()
            .filter(|tool| crate::surface_gates::tool_is_discoverable_by_default(&tool.name))
    }

    pub fn deferred_discoverable_definitions<'a>(
        &'a self,
        is_core_tool: impl Fn(&str) -> bool + 'a,
    ) -> impl Iterator<Item = &'a ToolDefinition> + 'a {
        self.discoverable_definitions()
            .filter(move |tool| !is_core_tool(&tool.name))
    }

    pub fn find_discoverable(&self, name: &str) -> Option<&ToolDefinition> {
        self.discoverable_definitions()
            .find(|tool| tool.name == name)
    }

    pub fn is_discoverable_name(name: &str) -> bool {
        crate::surface_gates::tool_is_discoverable_by_default(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn discoverable_definitions_hide_frozen_surfaces() {
        let registry = ToolRegistry::new(vec![
            tool("project_list"),
            tool("hand_activate"),
            tool("a2a_send"),
            tool("peer_list"),
        ]);
        let names: Vec<&str> = registry
            .discoverable_definitions()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(names, vec!["project_list"]);
    }

    #[test]
    fn deferred_discoverable_filters_core_and_frozen() {
        let registry = ToolRegistry::new(vec![
            tool("capability_search"),
            tool("project_list"),
            tool("fleet_metrics"),
        ]);
        let names: Vec<&str> = registry
            .deferred_discoverable_definitions(|name| name == "capability_search")
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(names, vec!["project_list"]);
    }
}
