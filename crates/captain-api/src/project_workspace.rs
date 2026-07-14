use crate::project_github_view as github_view;
use crate::project_launch_input::LaunchProjectReq;
use crate::routes::AppState;
use captain_runtime::kernel_handle::KernelHandle;
use serde_json::Value;
use std::path::{Path as FsPath, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct ProjectWorkspaceResolution {
    pub(crate) source: Value,
    pub(crate) workspace_path: Option<PathBuf>,
    pub(crate) repo_path: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) authorized: bool,
    pub(crate) authorization_error: Option<String>,
}

pub(crate) async fn prepare_project_workspace(
    state: &Arc<AppState>,
    req: &LaunchProjectReq,
    slug: &str,
) -> Result<ProjectWorkspaceResolution, String> {
    let source_type = requested_source_type(req);
    match source_type.as_str() {
        "github" => prepare_github_workspace(state, req, slug).await,
        "local" => prepare_local_workspace(state, req, slug),
        other => Err(format!("unknown project source_type: {other}")),
    }
}

fn requested_source_type(req: &LaunchProjectReq) -> String {
    req.source_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .or_else(|| {
            if req
                .github_full_name
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                None
            } else {
                Some("github".to_string())
            }
        })
        .unwrap_or_else(|| "local".to_string())
}

fn prepare_local_workspace(
    state: &AppState,
    req: &LaunchProjectReq,
    slug: &str,
) -> Result<ProjectWorkspaceResolution, String> {
    let raw_path = req
        .local_path
        .as_deref()
        .or(req.repo_path.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let path = resolve_project_path(state, raw_path, slug);
    let create_folder = req.create_folder.or(req.create_worktree).unwrap_or(true);
    if create_folder {
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("failed to create project folder '{}': {e}", path.display()))?;
    } else if !path.exists() {
        return Err(format!("project folder does not exist: {}", path.display()));
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("invalid project folder '{}': {e}", path.display()))?;
    let (authorized, authorization_error) = authorize_project_workspace(state, &path);
    Ok(ProjectWorkspaceResolution {
        source: serde_json::json!({
            "type": "local",
            "path": path.display().to_string(),
            "created_or_verified": true,
        }),
        repo_path: Some(path.display().to_string()),
        workspace_path: Some(path),
        branch: req
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        authorized,
        authorization_error,
    })
}

async fn prepare_github_workspace(
    state: &Arc<AppState>,
    req: &LaunchProjectReq,
    slug: &str,
) -> Result<ProjectWorkspaceResolution, String> {
    let full_name = req
        .github_full_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "github_full_name is required for GitHub projects".to_string())?
        .to_string();
    let full_name = github_view::normalize_github_full_name(&full_name)?;
    let clone_url = github_view::github_clone_url_for_full_name(&full_name);
    let branch = req
        .github_branch
        .as_deref()
        .or(req.branch.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let raw_path = req
        .local_path
        .as_deref()
        .or(req.repo_path.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let target = resolve_project_path(state, raw_path, slug);
    let token = github_token(state);
    let full_name_for_task = full_name.clone();
    let clone_url_for_task = clone_url.clone();
    let branch_for_task = branch.clone();
    let target_for_task = target.clone();
    tokio::task::spawn_blocking(move || {
        clone_or_link_github_repo(
            &full_name_for_task,
            &clone_url_for_task,
            branch_for_task.as_deref(),
            &target_for_task,
            token.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("GitHub clone task failed: {e}"))??;
    let path = target
        .canonicalize()
        .map_err(|e| format!("invalid cloned project folder '{}': {e}", target.display()))?;
    let (authorized, authorization_error) = authorize_project_workspace(state, &path);
    Ok(ProjectWorkspaceResolution {
        source: github_view::github_source_metadata(
            &full_name,
            req.github_repo_id.as_ref(),
            branch.as_deref(),
            &path.display().to_string(),
        ),
        repo_path: Some(path.display().to_string()),
        workspace_path: Some(path),
        branch,
        authorized,
        authorization_error,
    })
}

fn resolve_project_path(state: &AppState, raw_path: Option<&str>, slug: &str) -> PathBuf {
    match raw_path {
        Some(raw) => {
            let expanded = expand_user_path(raw);
            if expanded.is_absolute() {
                expanded
            } else {
                project_workspace_root(state).join(expanded)
            }
        }
        None => default_project_path(state, slug),
    }
}

fn expand_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return user_home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = user_home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

pub(crate) fn project_workspace_root(state: &AppState) -> PathBuf {
    state
        .kernel
        .config
        .effective_workspaces_dir()
        .join("projects")
}

pub(crate) fn default_project_path(state: &AppState, slug: &str) -> PathBuf {
    project_workspace_root(state).join(slug)
}

fn authorize_project_workspace(state: &AppState, path: &FsPath) -> (bool, Option<String>) {
    if path.starts_with(&state.kernel.config.home_dir) {
        return (true, None);
    }
    match state.kernel.add_workspace_path(path) {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    }
}

pub(crate) fn github_token(state: &AppState) -> Option<String> {
    state
        .kernel
        .resolve_credential("GITHUB_TOKEN")
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|s| !s.trim().is_empty())
}

fn clone_or_link_github_repo(
    full_name: &str,
    clone_url: &str,
    branch: Option<&str>,
    target: &FsPath,
    token: Option<&str>,
) -> Result<(), String> {
    if target.exists() {
        if target.join(".git").is_dir() {
            return Ok(());
        }
        let mut entries = target
            .read_dir()
            .map_err(|e| format!("failed to inspect '{}': {e}", target.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "target folder already exists and is not a git repo: {}",
                target.display()
            ));
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create project parent folder '{}': {e}",
                parent.display()
            )
        })?;
    }

    let mut attempts = Vec::new();
    if token.is_some() && command_available("gh") {
        let mut cmd = Command::new("gh");
        cmd.arg("repo").arg("clone").arg(full_name).arg(target);
        if let Some(branch) = branch {
            cmd.arg("--").arg("--branch").arg(branch);
        }
        if let Some(token) = token {
            cmd.env("GH_TOKEN", token).env("GITHUB_TOKEN", token);
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        match command_output(cmd) {
            Ok(()) => return Ok(()),
            Err(e) => attempts.push(format!("gh repo clone failed: {e}")),
        }
    }

    let mut cmd = Command::new("git");
    cmd.arg("clone");
    if let Some(branch) = branch {
        cmd.arg("--branch").arg(branch);
    }
    cmd.arg(clone_url).arg(target);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    match command_output(cmd) {
        Ok(()) => Ok(()),
        Err(e) => {
            attempts.push(format!("git clone failed: {e}"));
            Err(attempts.join("; "))
        }
    }
}

fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_output(mut cmd: Command) -> Result<(), String> {
    let output = cmd
        .output()
        .map_err(|e| format!("failed to run command: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {}", output.status)
    };
    Err(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let config = KernelConfig {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        };
        (tmp, Arc::new(state))
    }

    fn launch_req() -> LaunchProjectReq {
        LaunchProjectReq {
            name: None,
            slug: None,
            goal: "ship safely".to_string(),
            repo_path: None,
            local_path: None,
            source_type: None,
            github_full_name: None,
            github_clone_url: None,
            github_branch: None,
            github_repo_id: None,
            branch: None,
            create_worktree: None,
            create_folder: None,
            autonomy_level: None,
            acceptance_criteria: Vec::new(),
            deadline: None,
            goal_check_command: None,
            goal_recovery_command: None,
            goal_interval_secs: None,
        }
    }

    #[test]
    fn requested_source_type_ignores_legacy_github_clone_url() {
        let mut req = launch_req();
        req.github_clone_url = Some("https://token-secret@example.test/owner/repo.git".to_string());
        assert_eq!(requested_source_type(&req), "local");

        req.github_full_name = Some("owner/repo".to_string());
        assert_eq!(requested_source_type(&req), "github");

        req.github_full_name = None;
        req.source_type = Some(" github ".to_string());
        assert_eq!(requested_source_type(&req), "github");
    }

    #[tokio::test]
    async fn prepare_project_workspace_creates_local_folder_and_branch() {
        let (_tmp, state) = test_state();
        let workspace = state.kernel.config.home_dir.join("workspace");
        let mut req = launch_req();
        req.local_path = Some(workspace.display().to_string());
        req.branch = Some(" main ".to_string());

        let resolution = prepare_project_workspace(&state, &req, "demo")
            .await
            .unwrap();

        assert_eq!(resolution.source["type"], "local");
        assert_eq!(resolution.branch.as_deref(), Some("main"));
        let canonical_workspace = workspace.canonicalize().unwrap();
        assert_eq!(
            resolution.workspace_path.as_ref().unwrap(),
            &canonical_workspace
        );
        assert_eq!(
            resolution.repo_path.as_deref(),
            Some(canonical_workspace.to_str().unwrap())
        );
        assert!(resolution.authorized);
        assert!(resolution.authorization_error.is_none());
    }

    #[test]
    fn default_project_path_uses_projects_workspace_root() {
        let (_tmp, state) = test_state();

        let path = default_project_path(&state, "demo");

        assert!(path.ends_with("projects/demo"));
    }
}
