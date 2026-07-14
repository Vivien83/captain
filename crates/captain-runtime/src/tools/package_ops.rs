//! Package manager shell wrappers.

use crate::tools::tool_shell_exec;
use std::path::Path;

pub(crate) const CARGO_SUBCOMMANDS: &[&str] = &[
    "build", "test", "run", "check", "clippy", "fmt", "doc", "tree", "update", "install",
    "version", "search",
];
pub(crate) const NPM_SUBCOMMANDS: &[&str] = &[
    "install", "ci", "run", "test", "build", "list", "outdated", "audit", "version", "view",
];
pub(crate) const PIP_SUBCOMMANDS: &[&str] = &[
    "install", "list", "freeze", "show", "check", "search", "download",
];

pub(crate) async fn tool_pkg_wrapper(
    binary: &str,
    allowed: &[&str],
    input: &serde_json::Value,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<String, String> {
    let shell_input = package_shell_input(binary, allowed, input)?;
    tool_shell_exec(
        &shell_input,
        allowed_env_vars.unwrap_or(&[]),
        workspace_root,
        exec_policy,
    )
    .await
}

fn package_shell_input(
    binary: &str,
    allowed: &[&str],
    input: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let subcommand = input["subcommand"]
        .as_str()
        .ok_or("Missing 'subcommand' parameter")?;
    if !allowed.contains(&subcommand) {
        return Err(format!(
            "{binary} subcommand '{subcommand}' is not in the allowlist {allowed:?}. \
             Use shell_exec for ad-hoc invocations."
        ));
    }

    let args: Vec<&str> = input["args"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    for a in &args {
        if a.contains(';')
            || a.contains('|')
            || a.contains('&')
            || a.contains('`')
            || a.contains('$')
            || a.contains('>')
            || a.contains('<')
        {
            return Err(format!(
                "{binary} arg '{a}' contains shell metacharacter - refused for safety"
            ));
        }
    }

    let mut cmd_str = format!("{binary} {subcommand}");
    for a in &args {
        cmd_str.push(' ');
        cmd_str.push_str(a);
    }
    let mut shell_input = serde_json::json!({ "command": cmd_str });
    if let Some(timeout_seconds) = input["timeout_seconds"].as_u64().filter(|secs| *secs > 0) {
        shell_input["timeout_seconds"] = serde_json::json!(timeout_seconds);
    }
    Ok(shell_input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_timeout_seconds_is_forwarded_to_shell_exec() {
        let input = serde_json::json!({
            "subcommand": "test",
            "args": ["-p", "captain-runtime"],
            "timeout_seconds": 1_000
        });
        let shell_input = package_shell_input("cargo", CARGO_SUBCOMMANDS, &input).unwrap();
        assert_eq!(shell_input["command"], "cargo test -p captain-runtime");
        assert_eq!(shell_input["timeout_seconds"], 1_000);
    }

    #[test]
    fn package_timeout_seconds_zero_is_not_forwarded() {
        let input = serde_json::json!({
            "subcommand": "run",
            "timeout_seconds": 0
        });
        let shell_input = package_shell_input("npm", NPM_SUBCOMMANDS, &input).unwrap();
        assert_eq!(shell_input["command"], "npm run");
        assert!(shell_input.get("timeout_seconds").is_none());
    }
}
