//! MemPalace integration for project memory (v3.11f).
//!
//! Each project lives in its own MemPalace *wing* named
//! `project:<slug>`. The wing is created implicitly — MemPalace
//! auto-creates wings/rooms on first drawer add — by seeding an
//! `index` drawer with the project's name and goal when the project
//! is created.
//!
//! Reading back uses `mcp_mempalace_mempalace_search` with a
//! wing-qualified query so cross-project noise doesn't leak into the
//! summary.
//!
//! All calls are best-effort: a MemPalace outage must not block
//! project creation itself. Failures are logged at `warn` and return
//! `Ok(false)` so the caller knows the sync didn't happen without
//! treating it as a fatal error.

use crate::mcp::McpConnection;
use tokio::sync::Mutex;

/// Wing slug prefix. Kept as a constant so callers can derive the
/// wing name without re-deriving the naming scheme.
const WING_PREFIX: &str = "project:";

pub fn wing_name(slug: &str) -> String {
    format!("{WING_PREFIX}{slug}")
}

/// Seed the project's wing by writing a tiny index drawer. Returns
/// `Ok(true)` when the write succeeded, `Ok(false)` when MemPalace is
/// not connected (not an error — just a degraded state), and `Err`
/// only on an unexpected MCP-layer failure.
pub async fn ensure_project_wing(
    mcp_connections: Option<&Mutex<Vec<McpConnection>>>,
    project_slug: &str,
    project_name: &str,
    project_goal: &str,
) -> Result<bool, String> {
    let Some(conns_mutex) = mcp_connections else {
        tracing::debug!("ensure_project_wing: no MCP registry configured, skipping");
        return Ok(false);
    };
    let mut conns = conns_mutex.lock().await;
    let Some(conn) = conns.iter_mut().find(|c| c.name() == "mempalace") else {
        tracing::warn!(
            "ensure_project_wing: mempalace MCP server not connected; wing '{project_slug}' will be created lazily"
        );
        return Ok(false);
    };

    let payload = index_drawer_payload(project_slug, project_name, project_goal);
    if let Err(e) = conn
        .call_tool("mcp_mempalace_mempalace_add_drawer", &payload)
        .await
    {
        tracing::warn!(
            slug = project_slug,
            "ensure_project_wing: add_drawer failed ({e}); project will still exist, just without a pre-seeded wing"
        );
        return Err(format!("mempalace add_drawer: {e}"));
    }

    tracing::info!(
        slug = project_slug,
        "MemPalace wing '{}' seeded",
        wing_name(project_slug)
    );
    Ok(true)
}

/// Query the project wing for a summary. Returns the raw MemPalace
/// search response; callers format it for humans.
pub async fn project_summary(
    mcp_connections: Option<&Mutex<Vec<McpConnection>>>,
    project_slug: &str,
    limit: u32,
) -> Result<Option<String>, String> {
    let Some(conns_mutex) = mcp_connections else {
        return Ok(None);
    };
    let mut conns = conns_mutex.lock().await;
    let Some(conn) = conns.iter_mut().find(|c| c.name() == "mempalace") else {
        return Ok(None);
    };

    // MemPalace search accepts a free-text query; we scope by prefixing
    // the wing name so the model can lexically prefer entries from
    // this project. A real wing filter would be ideal — tracked for a
    // future MemPalace protocol upgrade.
    let query = format!("wing:{}", wing_name(project_slug));
    let payload = serde_json::json!({
        "query": query,
        "limit": limit,
    });
    match conn
        .call_tool("mcp_mempalace_mempalace_search", &payload)
        .await
    {
        Ok(result) if !result.is_empty() && result != "[]" && result != "null" => Ok(Some(result)),
        Ok(_) => Ok(None),
        Err(e) => Err(format!("mempalace search: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Payload helpers — split out so tests can assert structure without
// needing a live MCP connection.
// ---------------------------------------------------------------------------

pub fn index_drawer_payload(
    project_slug: &str,
    project_name: &str,
    project_goal: &str,
) -> serde_json::Value {
    let title = if project_name.is_empty() {
        project_slug.to_string()
    } else {
        project_name.to_string()
    };
    let content = if project_goal.is_empty() {
        format!("Project {project_slug} — index drawer seed.")
    } else {
        format!("Goal: {project_goal}")
    };
    serde_json::json!({
        "wing": wing_name(project_slug),
        "room": "index",
        "title": title,
        "content": content,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wing_name_is_prefixed() {
        assert_eq!(wing_name("alpha"), "project:alpha");
        assert_eq!(wing_name(""), "project:");
    }

    #[test]
    fn index_drawer_payload_fills_required_fields() {
        let p = index_drawer_payload("mlx", "MLX Finetune", "Ship a MoE model");
        assert_eq!(p["wing"], "project:mlx");
        assert_eq!(p["room"], "index");
        assert_eq!(p["title"], "MLX Finetune");
        assert_eq!(p["content"], "Goal: Ship a MoE model");
    }

    #[test]
    fn index_drawer_payload_falls_back_on_empty_name() {
        let p = index_drawer_payload("mlx", "", "");
        assert_eq!(p["title"], "mlx");
        assert!(p["content"].as_str().unwrap().contains("mlx"));
    }

    #[tokio::test]
    async fn ensure_project_wing_without_mcp_returns_false_not_error() {
        let got = ensure_project_wing(None, "alpha", "Alpha", "goal").await;
        assert_eq!(got, Ok(false));
    }

    #[tokio::test]
    async fn ensure_project_wing_without_connected_server_returns_false() {
        let conns = Mutex::new(Vec::<McpConnection>::new());
        let got = ensure_project_wing(Some(&conns), "beta", "Beta", "goal").await;
        assert_eq!(got, Ok(false));
    }

    #[tokio::test]
    async fn project_summary_without_mcp_returns_none() {
        let got = project_summary(None, "alpha", 10).await;
        assert_eq!(got, Ok(None));
    }

    #[tokio::test]
    async fn project_summary_without_connected_server_returns_none() {
        let conns = Mutex::new(Vec::<McpConnection>::new());
        let got = project_summary(Some(&conns), "alpha", 10).await;
        assert_eq!(got, Ok(None));
    }
}
