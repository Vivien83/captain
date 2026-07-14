use captain_types::agent::AgentManifest;

/// TS.3 — Pick the capability allowlist enforced at tool-execution time.
///
/// Priority:
///
/// 1. `manifest.tool_allowlist` non-empty and not `["*"]` → strict allowlist
///    (preferred path: explicit exec policy, separate from visibility).
/// 2. else `manifest.capabilities.tools` non-empty and not containing `"*"`
///    → fall back to the legacy declared-tools list. This keeps workers
///    that opted into capability-based scoping pre-TS.3 strictly enforced
///    even if their manifest never set the newer `tool_allowlist` field.
/// 3. else → `None` (deliberate bypass — Captain-style unrestricted, lets
///    `tool_search` surface deferred tools and execute them next turn).
pub(crate) fn effective_tool_policy(manifest: &AgentManifest) -> Option<Vec<String>> {
    let allowlist = &manifest.tool_allowlist;
    if !allowlist.is_empty() && !allowlist.iter().any(|t| t == "*") {
        return Some(allowlist.clone());
    }
    let declared = &manifest.capabilities.tools;
    if !declared.is_empty() && !declared.iter().any(|t| t == "*") {
        return Some(declared.clone());
    }
    None
}

pub(crate) fn manifest_subagent_depth(manifest: &AgentManifest) -> u32 {
    manifest
        .metadata
        .get("subagent_depth")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest_with(
        tool_allowlist: Vec<String>,
        capabilities_tools: Vec<String>,
    ) -> AgentManifest {
        AgentManifest {
            tool_allowlist,
            capabilities: captain_types::agent::ManifestCapabilities {
                tools: capabilities_tools,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn ts3_unrestricted_captain_returns_none() {
        let m = manifest_with(vec![], vec![]);
        assert!(effective_tool_policy(&m).is_none());
    }

    #[test]
    fn ts3_explicit_wildcard_in_allowlist_is_bypass() {
        let m = manifest_with(vec!["*".to_string()], vec![]);
        assert!(effective_tool_policy(&m).is_none());
    }

    #[test]
    fn ts3_capabilities_wildcard_falls_through_to_none() {
        let m = manifest_with(vec![], vec!["*".to_string()]);
        assert!(effective_tool_policy(&m).is_none());
    }

    #[test]
    fn ts3_tool_allowlist_takes_priority_over_capabilities() {
        let m = manifest_with(
            vec!["file_read".to_string(), "ssh_exec".to_string()],
            vec!["*".to_string()],
        );
        let policy = effective_tool_policy(&m).expect("must enforce");
        assert_eq!(
            policy,
            vec!["file_read".to_string(), "ssh_exec".to_string()]
        );
    }

    #[test]
    fn ts3_legacy_capabilities_only_worker_stays_enforced() {
        let m = manifest_with(vec![], vec!["file_read".to_string()]);
        let policy = effective_tool_policy(&m).expect("must enforce");
        assert_eq!(policy, vec!["file_read".to_string()]);
    }

    #[test]
    fn ts3_strict_allowlist_blocks_other_tools() {
        let m = manifest_with(vec!["file_read".to_string()], vec![]);
        let policy = effective_tool_policy(&m).expect("must enforce");
        assert!(policy.contains(&"file_read".to_string()));
        assert!(!policy.contains(&"shell_exec".to_string()));
    }

    /// Regression: TS.2 narrowed `available_tools` to CORE. Pre-TS.3
    /// the enforcement was built from that filtered list, so a Captain
    /// who'd discovered `text_to_speech` via `tool_search` would have been
    /// denied at exec time. With TS.3, an unrestricted manifest yields no
    /// policy — the runtime gates only on `Some` allowlists.
    #[test]
    fn ts3_captain_can_exec_tools_outside_visible_set() {
        let m = manifest_with(vec![], vec![]);
        assert!(
            effective_tool_policy(&m).is_none(),
            "unrestricted Captain must not be enforced — would re-introduce \
             the secret_write/text_to_speech denial bug"
        );
    }

    #[test]
    fn subagent_depth_defaults_to_zero() {
        assert_eq!(manifest_subagent_depth(&AgentManifest::default()), 0);
    }

    #[test]
    fn subagent_depth_reads_bounded_u32_metadata() {
        let mut manifest = AgentManifest::default();
        manifest
            .metadata
            .insert("subagent_depth".to_string(), json!(3));
        assert_eq!(manifest_subagent_depth(&manifest), 3);
    }

    #[test]
    fn subagent_depth_ignores_non_u32_metadata() {
        let mut manifest = AgentManifest::default();
        manifest
            .metadata
            .insert("subagent_depth".to_string(), json!(u64::from(u32::MAX) + 1));
        assert_eq!(manifest_subagent_depth(&manifest), 0);

        manifest
            .metadata
            .insert("subagent_depth".to_string(), json!("3"));
        assert_eq!(manifest_subagent_depth(&manifest), 0);
    }
}
