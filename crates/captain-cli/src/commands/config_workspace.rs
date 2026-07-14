use crate::{cli_captain_home, workspace_config};

pub(crate) fn cmd_config_workspace() {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading cwd: {e}");
            std::process::exit(1);
        }
    };
    let captain_home_dir = cli_captain_home();
    match render_workspace_report(&cwd, &captain_home_dir, None) {
        Ok(report) => print!("{report}"),
        Err(e) => {
            eprintln!("# Error reading workspace config: {e}");
            std::process::exit(1);
        }
    }
}

fn render_workspace_report(
    cwd: &std::path::Path,
    captain_home_dir: &std::path::Path,
    home_boundary: Option<&std::path::Path>,
) -> Result<String, String> {
    use std::fmt::Write;

    let mut out = String::new();
    writeln!(out, "# cwd: {}", cwd.display()).map_err(|e| e.to_string())?;
    let discovered = workspace_config::discover_with_boundary(cwd, home_boundary)?;
    match discovered {
        Some(found) => {
            writeln!(out, "# .captain.toml: {}", found.config_path.display())
                .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "agent           = {:?}",
                found.config.captain.agent.as_deref().unwrap_or("(none)")
            )
            .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "agent_name      = {:?}",
                found
                    .config
                    .captain
                    .agent_name
                    .as_deref()
                    .unwrap_or("(none)")
            )
            .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "project_slug    = {:?}",
                found
                    .config
                    .captain
                    .project_slug
                    .as_deref()
                    .unwrap_or("(none)")
            )
            .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "tool_profile    = {:?}",
                found
                    .config
                    .captain
                    .tool_profile
                    .as_deref()
                    .unwrap_or("(none)")
            )
            .map_err(|e| e.to_string())?;
            write_extra_paths(&mut out, &found, captain_home_dir)?;
        }
        None => {
            writeln!(out, "# No .captain.toml between cwd and $HOME.")
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

fn write_extra_paths(
    out: &mut String,
    found: &workspace_config::DiscoveredWorkspace,
    captain_home_dir: &std::path::Path,
) -> Result<(), String> {
    use std::fmt::Write;

    if found.config.captain.extra_paths.is_empty() {
        writeln!(out, "extra_paths     = []").map_err(|e| e.to_string())?;
        return Ok(());
    }
    match workspace_config::validate_extra_paths(
        &found.config.captain.extra_paths,
        captain_home_dir,
    ) {
        Ok(canon) => {
            writeln!(out, "extra_paths (validated):").map_err(|e| e.to_string())?;
            for p in canon {
                writeln!(out, "  - {}", p.display()).map_err(|e| e.to_string())?;
            }
        }
        Err(e) => {
            writeln!(out, "extra_paths     = REJECTED ({e})").map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::render_workspace_report;

    #[test]
    fn report_says_no_match_when_no_file_in_subtree() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let captain_home = tmp.path().join("home");
        std::fs::create_dir_all(&captain_home).unwrap();
        let report = render_workspace_report(&nested, &captain_home, Some(tmp.path())).unwrap();
        assert!(report.contains("# cwd:"));
        assert!(report.contains("No .captain.toml"));
    }

    #[test]
    fn report_dumps_resolved_fields() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".captain.toml"),
            "[captain]\nagent_name = \"captain\"\nproject_slug = \"demo\"\n",
        )
        .unwrap();
        let captain_home = tmp.path().join("home");
        std::fs::create_dir_all(&captain_home).unwrap();
        let report = render_workspace_report(tmp.path(), &captain_home, Some(tmp.path())).unwrap();
        assert!(report.contains(".captain.toml:"));
        assert!(report.contains("agent_name      = \"captain\""));
        assert!(report.contains("project_slug    = \"demo\""));
        assert!(report.contains("extra_paths     = []"));
    }

    #[test]
    fn report_marks_extra_paths_rejected_for_blocked_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let captain_home = tmp.path().join("home");
        std::fs::create_dir_all(&captain_home).unwrap();
        let secrets = captain_home.join("secrets.env");
        std::fs::write(&secrets, "K=v\n").unwrap();
        let toml_body = format!(
            "[captain]\nagent_name = \"captain\"\nextra_paths = [\"{}\"]\n",
            secrets.display()
        );
        std::fs::write(tmp.path().join(".captain.toml"), toml_body).unwrap();
        let report = render_workspace_report(tmp.path(), &captain_home, Some(tmp.path())).unwrap();
        assert!(
            report.contains("extra_paths     = REJECTED"),
            "report:\n{report}"
        );
        assert!(report.contains("secrets.env"), "report:\n{report}");
    }
}
