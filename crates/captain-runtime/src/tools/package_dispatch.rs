//! Package-manager wrapper dispatch.

use std::path::Path;

use captain_types::config::ExecPolicy;

use super::{tool_pkg_wrapper, CARGO_SUBCOMMANDS, NPM_SUBCOMMANDS, PIP_SUBCOMMANDS};

pub(crate) async fn dispatch_package_tool(
    tool_name: &str,
    input: &serde_json::Value,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    exec_policy: Option<&ExecPolicy>,
) -> Result<String, String> {
    let (binary, subcommands) = match tool_name {
        "cargo" => ("cargo", CARGO_SUBCOMMANDS),
        "npm" => ("npm", NPM_SUBCOMMANDS),
        "pip" => ("pip", PIP_SUBCOMMANDS),
        other => return Err(format!("Unknown package tool: {other}")),
    };
    tool_pkg_wrapper(
        binary,
        subcommands,
        input,
        allowed_env_vars,
        workspace_root,
        exec_policy,
    )
    .await
}
