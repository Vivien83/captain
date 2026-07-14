use serde_json::{json, Value};

pub(crate) fn projects_environment_view(github_authenticated: bool) -> Value {
    json!({
        "platform": std::env::consts::OS,
        "family": std::env::consts::FAMILY,
        "default_source_type": "local",
        "local_default_available": true,
        "github_authenticated": github_authenticated,
    })
}

#[cfg(test)]
#[path = "project_environment_view_tests.rs"]
mod project_environment_view_tests;
