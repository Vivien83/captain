use super::*;

/// Q.7 — `ssh_exec` is registered.
#[test]
fn test_ssh_exec_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"ssh_exec"));
    let def = tools.iter().find(|t| t.name == "ssh_exec").unwrap();
    let req: Vec<&str> = def.input_schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(req.contains(&"key_name"));
    assert!(req.contains(&"command"));
}

/// Q.7 — critical pattern in remote command is blocked PRE-FLIGHT
/// (before any vault open or network call).
#[tokio::test]
async fn test_tool_ssh_exec_blocks_critical_in_safe_mode() {
    let policy = captain_types::config::ExecPolicy {
        critical_mode: captain_types::config::CriticalMode::Safe,
        ..Default::default()
    };
    let input = serde_json::json!({
        "key_name": "anything",
        "command": "rm -rf /"
    });
    let r = tool_ssh_exec(&input, Some(&policy)).await;
    assert!(r.is_err());
    let err = r.unwrap_err();
    assert!(err.contains("hyper-critical pattern"), "got: {err}");
    assert!(err.contains("Refused before sending"), "got: {err}");
}

/// Q.7 — open mode also refuses (because no interactive bridge yet).
#[tokio::test]
async fn test_tool_ssh_exec_blocks_critical_in_open_mode() {
    let policy = captain_types::config::ExecPolicy::default(); // Open by default
    let input = serde_json::json!({
        "key_name": "anything",
        "command": "DROP DATABASE prod"
    });
    let r = tool_ssh_exec(&input, Some(&policy)).await;
    assert!(r.is_err());
    assert!(
        r.unwrap_err().contains("interactive approval"),
        "open-mode SSH critical must surface the not-yet-implemented note"
    );
}

/// Q.8 — `ssh_upload` and `ssh_download` are registered.
#[test]
fn test_ssh_sftp_tools_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"ssh_upload"), "ssh_upload missing");
    assert!(names.contains(&"ssh_download"), "ssh_download missing");
}

/// Q.8 — missing parameters give a clear error.
#[tokio::test]
async fn test_tool_ssh_upload_rejects_missing_params() {
    let r = tool_ssh_upload(&serde_json::json!({})).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("key_name"));

    let r = tool_ssh_upload(&serde_json::json!({ "key_name": "x" })).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("local_path"));
}

#[tokio::test]
async fn test_tool_ssh_download_rejects_missing_params() {
    let r = tool_ssh_download(&serde_json::json!({})).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("key_name"));
}

/// R.A.3 — Les descriptions des tools qui se chevauchent doivent
/// contenir des marqueurs disambigueurs explicites pour aider le LLM
/// à choisir le bon (file_list vs glob, edit_file vs file_write).
/// Cf. observation live : avant R.A.3, le LLM choisissait file_list
/// quand glob était plus approprié.
#[test]
fn ra3_overlapping_tools_have_disambiguating_markers() {
    let tools = builtin_tool_definitions();
    let by_name: std::collections::HashMap<String, String> = tools
        .iter()
        .map(|t| (t.name.clone(), t.description.clone()))
        .collect();

    let file_list = by_name.get("file_list").expect("file_list registered");
    assert!(
        file_list.contains("NON-RÉCURSIF"),
        "file_list must say NON-RÉCURSIF to distinguish from glob; got: {file_list}"
    );
    assert!(
        file_list.contains("glob"),
        "file_list must point to glob for the recursive case"
    );

    let glob = by_name.get("glob").expect("glob registered");
    assert!(
        glob.contains("RÉCURSIF"),
        "glob must say RÉCURSIF; got: {glob}"
    );
    assert!(
        glob.contains("file_list"),
        "glob must point to file_list for the single-level case"
    );

    let edit_file = by_name.get("edit_file").expect("edit_file registered");
    assert!(
        edit_file.contains("CHOIX PAR DÉFAUT") || edit_file.contains("DEFAULT"),
        "edit_file must be marked as the default modify tool"
    );

    let file_write = by_name.get("file_write").expect("file_write registered");
    assert!(
        file_write.contains("CRÉATION") || file_write.contains("ÉCRASEMENT TOTAL"),
        "file_write must say it overwrites the whole file (creation/total)"
    );
    assert!(
        file_write.contains("edit_file"),
        "file_write must point to edit_file for partial modifications"
    );
}

/// Q.10 — cargo / npm / pip wrappers are registered.
#[test]
fn test_pkg_wrappers_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for n in ["cargo", "npm", "pip"] {
        assert!(names.contains(&n), "{n} must be registered, got: {names:?}");
    }
}

/// Q.10 — disallowed subcommand returns clear error.
#[tokio::test]
async fn test_pkg_wrapper_rejects_unknown_subcommand() {
    let r = tool_pkg_wrapper(
        "cargo",
        CARGO_SUBCOMMANDS,
        &serde_json::json!({ "subcommand": "publish" }),
        None,
        None,
        None,
    )
    .await;
    assert!(r.is_err());
    let err = r.unwrap_err();
    assert!(err.contains("not in the allowlist"), "got: {err}");
}

/// Q.10 — shell metacharacters in args are refused.
#[tokio::test]
async fn test_pkg_wrapper_rejects_metacharacter_args() {
    let r = tool_pkg_wrapper(
        "npm",
        NPM_SUBCOMMANDS,
        &serde_json::json!({
            "subcommand": "install",
            "args": ["--save-dev; rm -rf /"]
        }),
        None,
        None,
        None,
    )
    .await;
    assert!(r.is_err());
    assert!(
        r.unwrap_err().contains("metacharacter"),
        "must catch shell injection in args"
    );
}

/// Q.10 — pip subcommand allowlist is respected.
#[tokio::test]
async fn test_pkg_wrapper_pip_freeze_passes_validation() {
    // We don't actually exec (no exec_policy), but the function should
    // get past validation and try shell_exec — which without policy
    // either succeeds or returns a useful error.
    let r = tool_pkg_wrapper(
        "pip",
        PIP_SUBCOMMANDS,
        &serde_json::json!({ "subcommand": "freeze" }),
        None,
        None,
        None,
    )
    .await;
    // Either Ok or Err here is fine — what we want is that the
    // validation phase didn't reject it.
    if let Err(e) = &r {
        assert!(
            !e.contains("not in the allowlist"),
            "freeze must pass allowlist, got: {e}"
        );
        assert!(!e.contains("metacharacter"));
    }
}
