use crate::kernel_handle::KernelHandle;
use std::path::Path;
use std::sync::Arc;

#[allow(clippy::unnecessary_wraps)]
pub(crate) async fn tool_skill_md_execute(
    input: &serde_json::Value,
    _kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let skill_name = input["skill"].as_str().ok_or("Missing 'skill' parameter")?;
    let capability = input["capability"]
        .as_str()
        .ok_or("Missing 'capability' parameter")?;
    let args = input.get("args").cloned().unwrap_or(serde_json::json!({}));

    let captain_home = crate::skill_execute::captain_skills_dir();
    let search_paths = [
        workspace_root.map(|ws| ws.join("skills").join(format!("{skill_name}.md"))),
        workspace_root.map(|ws| ws.join("skills").join(skill_name).join("SKILL.md")),
        Some(captain_home.join(format!("{skill_name}.md"))),
        Some(captain_home.join(skill_name).join("SKILL.md")),
    ];

    let skill_path = search_paths
        .into_iter()
        .flatten()
        .find(|p| p.exists())
        .ok_or_else(|| format!("Skill '{skill_name}' not found"))?;

    match crate::skill_execute::execute_capability(&skill_path, capability, &[], &args).await {
        Ok(output) => Ok(output),
        Err(error) if crate::skill_execute::is_syntax_preflight_error(&error) => {
            Ok(serde_json::json!({
                "skill": skill_name,
                "capability": capability,
                "status": "blocked",
                "is_error": true,
                "error": error,
                "next_action": "Run skill_check for this skill, then fix or refine the failing bash capability before retrying skill_execute.",
            })
            .to_string())
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn tool_scaffold_skill(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let name = input["name"].as_str().ok_or("Missing 'name'")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description'")?;
    let capabilities: Vec<String> = input["capabilities"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["run".into()]);

    let ws = workspace_root.ok_or("No workspace root — agent must have a workspace")?;
    let skill_dir = ws.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).map_err(|e| format!("mkdir skill: {e}"))?;

    let caps = capabilities
        .iter()
        .map(|c| {
            format!(
                "### {c}\n```bash\n# Replace this scaffold with verified commands for {c}.\necho \"Running {c}...\"\n```\n"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let skill_md = format!(
        "---\n\
         id: {name}\n\
         name: {name}\n\
         description: {description}\n\
         owner: human\n\
         locked: true\n\
         approved: true\n\
         version: 1\n\
         verified_by: human\n\
         success_rate: null\n\
         ---\n\n\
         # {name}\n\n\
         {description}\n\n\
         ## Capabilities\n\n\
         {caps}"
    );
    std::fs::write(skill_dir.join("SKILL.md"), &skill_md)
        .map_err(|e| format!("write SKILL.md: {e}"))?;

    Ok(format!(
        "Skill '{name}' scaffolded at {}/SKILL.md\nCapabilities: {}\n\nNext: implement the bash blocks in each capability section.",
        skill_dir.display(),
        capabilities.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn skill_execute_blocks_invalid_bash_with_actionable_json() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("broken");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "### run\n```bash\necho before\nif true; then\n  echo missing-fi\n```\n",
        )
        .unwrap();

        let output = tool_skill_md_execute(
            &serde_json::json!({"skill": "broken", "capability": "run"}),
            None,
            Some(dir.path()),
        )
        .await
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(json["status"], "blocked");
        assert_eq!(json["is_error"], true);
        assert!(json["error"]
            .as_str()
            .unwrap()
            .contains(crate::skill_execute::SKILL_SYNTAX_PREFLIGHT_ERROR_PREFIX));
        assert!(json["next_action"]
            .as_str()
            .unwrap()
            .contains("skill_check"));
    }

    #[tokio::test]
    async fn scaffold_skill_marks_human_authored_locked_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        tool_scaffold_skill(
            &serde_json::json!({
                "name": "status-checker",
                "description": "Checks a service status",
                "capabilities": ["run"]
            }),
            Some(dir.path()),
        )
        .await
        .unwrap();

        let body = std::fs::read_to_string(dir.path().join("skills/status-checker/SKILL.md"))
            .expect("skill should be written");
        assert!(body.contains("owner: human"));
        assert!(body.contains("locked: true"));
        assert!(body.contains("approved: true"));
        assert!(body.contains("verified_by: human"));
        assert!(body.contains("Replace this scaffold with verified commands for run."));
        assert!(!body.contains(&format!("# {}{}", "TO", "DO")));
    }
}
