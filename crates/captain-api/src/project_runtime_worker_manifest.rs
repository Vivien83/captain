use crate::project_runtime_worker_model::runtime_worker_model_config;
use crate::project_runtime_worker_tools::runtime_worker_authorized_tools_for_runtime;
use crate::project_runtime_workers::RuntimeWorkerSpec;
use crate::routes::AppState;
use captain_memory::project;
use captain_types::agent::{AgentManifest, ModelConfig, Priority};
use captain_types::config::DefaultModelConfig;
use std::path::{Path, PathBuf};

pub(crate) struct RuntimeWorkerPreparedManifest {
    pub(crate) manifest: AgentManifest,
    pub(crate) authorized_tools: Vec<String>,
}

pub(crate) fn runtime_worker_manifest_for_state(
    state: &AppState,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    runtime: &serde_json::Value,
    project_source_workspace: Option<PathBuf>,
) -> RuntimeWorkerPreparedManifest {
    runtime_worker_manifest_for_runtime(
        project,
        spec,
        runtime,
        state.kernel.effective_default_model(),
        |provider, model| runtime_model_exists(state, provider, model),
        state.kernel.config.effective_workspaces_dir(),
        project_source_workspace,
    )
}

pub(crate) fn runtime_worker_manifest_for_runtime<F>(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    runtime: &serde_json::Value,
    default_model: DefaultModelConfig,
    model_exists: F,
    workspaces_dir: PathBuf,
    project_source_workspace: Option<PathBuf>,
) -> RuntimeWorkerPreparedManifest
where
    F: Fn(&str, &str) -> bool,
{
    let authorized_tools =
        runtime_worker_authorized_tools_for_runtime(&spec.profile, spec.phase, runtime);
    let model =
        runtime_worker_model_config(default_model, spec.phase, &authorized_tools, model_exists);
    let workspace = runtime_worker_workspace(
        project,
        spec,
        &workspaces_dir,
        project_source_workspace.as_deref(),
    );
    let manifest = runtime_worker_manifest_for_project(
        project,
        spec,
        model,
        authorized_tools.clone(),
        workspace,
        project_source_workspace,
    );

    RuntimeWorkerPreparedManifest {
        manifest,
        authorized_tools,
    }
}

fn runtime_worker_workspace(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    workspaces_dir: &Path,
    project_source_workspace: Option<&Path>,
) -> PathBuf {
    project_source_workspace
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            workspaces_dir
                .join("project-runs")
                .join(&project.slug)
                .join(spec.phase)
        })
}

fn runtime_model_exists(state: &AppState, provider: &str, model: &str) -> bool {
    state
        .kernel
        .model_catalog
        .read()
        .ok()
        .and_then(|catalog| {
            catalog
                .find_model(&format!("{provider}/{model}"))
                .or_else(|| catalog.find_model(model))
                .map(|entry| entry.provider.eq_ignore_ascii_case(provider))
        })
        .unwrap_or(false)
}

pub(crate) fn runtime_worker_manifest_for_project(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    model: ModelConfig,
    authorized_tools: Vec<String>,
    workspace: PathBuf,
    project_source_workspace: Option<PathBuf>,
) -> AgentManifest {
    let mut manifest = AgentManifest {
        name: runtime_worker_agent_name(project, spec),
        version: "0.1.0".to_string(),
        description: format!(
            "Captain project runtime worker for {} phase on {}",
            spec.phase, project.slug
        ),
        author: "captain-project-runtime".to_string(),
        module: "builtin:chat".to_string(),
        model,
        priority: Priority::High,
        profile: Some(spec.profile.clone()),
        workspace: Some(workspace),
        generate_identity_files: false,
        ..Default::default()
    };
    let mut implied = spec.profile.implied_capabilities();
    implied.tools = authorized_tools.clone();
    manifest.capabilities = implied;
    manifest.tool_allowlist = authorized_tools.clone();
    manifest.fallback_models = Vec::new();
    manifest.tags = vec![
        "project-runtime".to_string(),
        format!("project:{}", project.slug),
        format!("phase:{}", spec.phase),
    ];
    manifest
        .metadata
        .insert("project_id".to_string(), serde_json::json!(project.id));
    manifest
        .metadata
        .insert("project_slug".to_string(), serde_json::json!(project.slug));
    manifest
        .metadata
        .insert("runtime_phase".to_string(), serde_json::json!(spec.phase));
    if let Some(project_source_workspace) = project_source_workspace {
        manifest.metadata.insert(
            "workspace_kind".to_string(),
            serde_json::json!("project_source"),
        );
        manifest.metadata.insert(
            "project_source_workspace".to_string(),
            serde_json::json!(project_source_workspace.display().to_string()),
        );
    }
    manifest.metadata.insert(
        "model_selection".to_string(),
        serde_json::json!({
            "policy": "same_provider_fit_to_phase",
            "phase": spec.phase,
            "provider": manifest.model.provider.clone(),
            "model": manifest.model.model.clone(),
        }),
    );
    manifest.metadata.insert(
        "authorized_tools".to_string(),
        serde_json::json!(authorized_tools),
    );
    manifest
}

pub(crate) fn runtime_worker_agent_name(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
) -> String {
    format!("project-{}-{}", project.slug, spec.phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_runtime_workers::RUNTIME_WORKER_SPECS;
    use captain_memory::project::ProjectStatus;
    use captain_types::agent::{ModelConfig, ToolProfile};

    fn project() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Ship safely".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    fn model() -> ModelConfig {
        ModelConfig {
            provider: "codex".to_string(),
            model: "gpt-5.4-mini".to_string(),
            system_prompt: "worker prompt".to_string(),
            ..Default::default()
        }
    }

    fn default_model(provider: &str, model: &str) -> DefaultModelConfig {
        DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: String::new(),
            base_url: None,
        }
    }

    #[test]
    fn manifest_for_runtime_prepares_authorized_tools_model_and_fallback_workspace() {
        let project = project();
        let runtime = serde_json::json!({
            "worker_results": {
                "think": {
                    "tool_request": {
                        "status": "approved",
                        "tools": ["extra_review_tool"]
                    }
                }
            }
        });

        let prepared = runtime_worker_manifest_for_runtime(
            &project,
            &RUNTIME_WORKER_SPECS[1],
            &runtime,
            default_model("codex", "gpt-5.4"),
            |_, model| model == "gpt-5.4-mini",
            PathBuf::from("/var/captain-workspaces"),
            None,
        );

        assert_eq!(prepared.manifest.model.model, "gpt-5.4-mini");
        assert!(prepared
            .authorized_tools
            .contains(&"extra_review_tool".to_string()));
        assert_eq!(prepared.manifest.tool_allowlist, prepared.authorized_tools);
        assert_eq!(
            prepared.manifest.workspace.as_deref(),
            Some(std::path::Path::new(
                "/var/captain-workspaces/project-runs/demo/think"
            ))
        );
        assert!(!prepared.manifest.metadata.contains_key("workspace_kind"));
    }

    #[test]
    fn manifest_for_runtime_prefers_project_source_workspace() {
        let project = project();
        let prepared = runtime_worker_manifest_for_runtime(
            &project,
            &RUNTIME_WORKER_SPECS[3],
            &serde_json::json!({}),
            default_model("openai", "gpt-5.1"),
            |_, _| false,
            PathBuf::from("/var/captain-workspaces"),
            Some(PathBuf::from("/src/demo")),
        );

        assert_eq!(prepared.manifest.model.model, "gpt-5.1");
        assert_eq!(
            prepared.manifest.workspace.as_deref(),
            Some(std::path::Path::new("/src/demo"))
        );
        assert_eq!(
            prepared.manifest.metadata["workspace_kind"],
            "project_source"
        );
        assert_eq!(
            prepared.manifest.metadata["project_source_workspace"],
            "/src/demo"
        );
    }

    #[test]
    fn manifest_keeps_worker_identity_scope_and_tags() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let manifest = runtime_worker_manifest_for_project(
            &project,
            spec,
            model(),
            vec!["file_read".to_string(), "shell_exec".to_string()],
            PathBuf::from("/tmp/demo"),
            None,
        );

        assert_eq!(
            runtime_worker_agent_name(&project, spec),
            "project-demo-build"
        );
        assert_eq!(manifest.name, "project-demo-build");
        assert_eq!(
            manifest.description,
            "Captain project runtime worker for build phase on demo"
        );
        assert_eq!(manifest.author, "captain-project-runtime");
        assert_eq!(manifest.module, "builtin:chat");
        assert_eq!(manifest.priority, Priority::High);
        assert_eq!(manifest.profile, Some(ToolProfile::Coding));
        assert_eq!(
            manifest.workspace.as_deref(),
            Some(std::path::Path::new("/tmp/demo"))
        );
        assert!(!manifest.generate_identity_files);
        assert!(manifest.fallback_models.is_empty());
        assert_eq!(
            manifest.tags,
            vec![
                "project-runtime".to_string(),
                "project:demo".to_string(),
                "phase:build".to_string()
            ]
        );
    }

    #[test]
    fn manifest_projects_tool_scope_into_allowlist_and_capabilities() {
        let project = project();
        let tools = vec!["file_read".to_string(), "tool_search".to_string()];
        let manifest = runtime_worker_manifest_for_project(
            &project,
            &RUNTIME_WORKER_SPECS[0],
            model(),
            tools.clone(),
            PathBuf::from("/tmp/demo"),
            None,
        );

        assert_eq!(manifest.tool_allowlist, tools);
        assert_eq!(manifest.capabilities.tools, tools);
        assert_eq!(
            manifest.metadata["authorized_tools"],
            serde_json::json!(tools)
        );
    }

    #[test]
    fn manifest_records_project_and_model_metadata() {
        let project = project();
        let manifest = runtime_worker_manifest_for_project(
            &project,
            &RUNTIME_WORKER_SPECS[1],
            model(),
            vec!["file_read".to_string()],
            PathBuf::from("/tmp/demo"),
            Some(PathBuf::from("/src/demo")),
        );

        assert_eq!(manifest.metadata["project_id"], "project-1");
        assert_eq!(manifest.metadata["project_slug"], "demo");
        assert_eq!(manifest.metadata["runtime_phase"], "think");
        assert_eq!(manifest.metadata["workspace_kind"], "project_source");
        assert_eq!(manifest.metadata["project_source_workspace"], "/src/demo");
        assert_eq!(
            manifest.metadata["model_selection"],
            serde_json::json!({
                "policy": "same_provider_fit_to_phase",
                "phase": "think",
                "provider": "codex",
                "model": "gpt-5.4-mini",
            })
        );
    }

    #[test]
    fn manifest_omits_source_workspace_metadata_when_using_fallback_workspace() {
        let project = project();
        let manifest = runtime_worker_manifest_for_project(
            &project,
            &RUNTIME_WORKER_SPECS[0],
            model(),
            vec!["file_read".to_string()],
            PathBuf::from("/tmp/demo"),
            None,
        );

        assert!(!manifest.metadata.contains_key("workspace_kind"));
        assert!(!manifest.metadata.contains_key("project_source_workspace"));
    }
}
