//! Runtime product-surface gates.
//!
//! These defaults keep frozen/experimental surfaces compiled but out of active
//! tool discovery. Config-level gates live in `captain-types`; this module is
//! the conservative runtime fallback used before full config plumbing reaches
//! every prompt and discovery path.

pub const ACTIVE_SURFACES: &[&str] = &[
    "cli",
    "web",
    "telegram",
    "projects",
    "memory",
    "skills",
    "automation",
    "status",
];

pub const FROZEN_SURFACES: &[&str] = &[
    "hands",
    "a2a",
    "peers",
    "fleets",
    "desktop",
    "long-tail-channels",
    "roadmap-dashboard",
    "experimental-integrations",
];

pub fn surface_is_active_by_default(surface: &str) -> bool {
    ACTIVE_SURFACES
        .iter()
        .any(|active| active.eq_ignore_ascii_case(surface))
}

pub fn surface_is_frozen_by_default(surface: &str) -> bool {
    FROZEN_SURFACES
        .iter()
        .any(|frozen| frozen.eq_ignore_ascii_case(surface))
}

pub fn tool_surface(tool_name: &str) -> Option<&'static str> {
    if matches!(tool_name, "hand" | "hands") || tool_name.starts_with("hand_") {
        Some("hands")
    } else if tool_name == "a2a" || tool_name.starts_with("a2a_") {
        Some("a2a")
    } else if matches!(tool_name, "peer" | "peers") || tool_name.starts_with("peer_") {
        Some("peers")
    } else if matches!(tool_name, "fleet" | "fleets") || tool_name.starts_with("fleet_") {
        Some("fleets")
    } else {
        None
    }
}

pub fn tool_is_discoverable_by_default(tool_name: &str) -> bool {
    tool_surface(tool_name)
        .map(|surface| !surface_is_frozen_by_default(surface))
        .unwrap_or(true)
}

pub fn source_is_discoverable_by_default(source: &str) -> bool {
    !matches!(
        source,
        "hand" | "hands" | "a2a" | "peer" | "peers" | "fleet" | "fleets"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier1_surfaces_are_active() {
        for surface in ACTIVE_SURFACES {
            assert!(surface_is_active_by_default(surface));
        }
    }

    #[test]
    fn frozen_tools_are_hidden_from_default_discovery() {
        assert!(!tool_is_discoverable_by_default("hand"));
        assert!(!tool_is_discoverable_by_default("hand_activate"));
        assert!(!tool_is_discoverable_by_default("a2a"));
        assert!(!tool_is_discoverable_by_default("a2a_send"));
        assert!(!tool_is_discoverable_by_default("peer"));
        assert!(!tool_is_discoverable_by_default("peer_list"));
        assert!(!tool_is_discoverable_by_default("fleet"));
        assert!(!tool_is_discoverable_by_default("fleet_metrics"));
    }

    #[test]
    fn tier1_tools_remain_discoverable() {
        assert!(tool_is_discoverable_by_default("project_list"));
        assert!(tool_is_discoverable_by_default("cron_create"));
        assert!(tool_is_discoverable_by_default("channel_send"));
        assert!(tool_is_discoverable_by_default("memory_recall"));
    }
}
