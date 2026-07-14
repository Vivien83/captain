use std::path::{Path, PathBuf};

use super::setup_support::{
    setup_config_string, setup_env_or_answer_any, setup_env_or_answer_bool, setup_read_config_value,
};
use crate::{prompt_input, restrict_file_permissions, ui};

type ConfigPatch = captain_runtime::integrations::ConfigPatch;

#[derive(Debug, Clone)]
pub(crate) struct SetupDeploymentOutcome {
    pub(crate) public_url: Option<String>,
    pub(crate) direct_url: Option<String>,
    pub(crate) api_listen: String,
    pub(crate) shell_enabled: bool,
    pub(crate) caddyfile_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct SetupSurfaceOptions {
    public_url: Option<String>,
    shell_enabled: bool,
    reverse_proxy: String,
}

#[derive(Debug, Clone)]
struct SetupDeploymentPlan {
    public_url: Option<String>,
    direct_url: Option<String>,
    api_listen: String,
    shell_enabled: bool,
    reverse_proxy: String,
    https: bool,
}

impl SetupDeploymentPlan {
    fn into_outcome(self, caddyfile_path: Option<PathBuf>) -> SetupDeploymentOutcome {
        SetupDeploymentOutcome {
            public_url: self.public_url,
            direct_url: self.direct_url,
            api_listen: self.api_listen,
            shell_enabled: self.shell_enabled,
            caddyfile_path,
        }
    }
}

pub(crate) fn setup_configure_product_surface(
    captain_dir: &Path,
    profile: &str,
    answers: Option<&toml::Value>,
    interactive: bool,
) -> Result<SetupDeploymentOutcome, String> {
    ui::blank();
    ui::section("Surface web");

    let existing_api_listen =
        setup_config_string(setup_read_config_value(captain_dir).as_ref(), "api_listen");
    let options = setup_collect_surface_options(profile, answers, interactive);
    let plan = setup_deployment_plan(profile, answers, options, existing_api_listen);
    setup_apply_surface_config(captain_dir, profile, &plan)?;
    let caddyfile_path = setup_write_surface_artifacts(captain_dir, profile, &plan)?;
    setup_print_surface_summary(&plan, caddyfile_path.as_ref());

    Ok(plan.into_outcome(caddyfile_path))
}

fn setup_collect_surface_options(
    profile: &str,
    answers: Option<&toml::Value>,
    interactive: bool,
) -> SetupSurfaceOptions {
    let configured_url = setup_env_or_answer_any(
        "CAPTAIN_PUBLIC_URL",
        answers,
        &["deployment.public_url", "public_url"],
    )
    .or_else(|| {
        setup_env_or_answer_any("CAPTAIN_DOMAIN", answers, &["deployment.domain", "domain"])
    });
    let mut public_url = configured_url
        .as_deref()
        .and_then(setup_normalize_public_url);

    if interactive && profile == "vps" && public_url.is_none() {
        let answer = prompt_input("  Domaine public VPS (optionnel, ex: captain.example.com) : ");
        public_url = setup_normalize_public_url(&answer);
    }

    let mut shell_enabled = setup_env_or_answer_bool(
        "CAPTAIN_WEB_TERMINAL_SHELL",
        answers,
        &[
            "web_terminal.allow_raw_shell",
            "terminal.allow_shell",
            "shell.enabled",
        ],
    )
    .unwrap_or(false);

    if interactive && profile == "vps" {
        let answer = prompt_input("  Activer le mode Shell dans /terminal ? [y/N] ");
        if !answer.trim().is_empty() {
            shell_enabled = answer.starts_with(['y', 'Y']);
        }
    }

    let reverse_proxy = setup_env_or_answer_any(
        "CAPTAIN_REVERSE_PROXY",
        answers,
        &["deployment.reverse_proxy", "reverse_proxy"],
    )
    .unwrap_or_else(|| "caddy".to_string());

    SetupSurfaceOptions {
        public_url,
        shell_enabled,
        reverse_proxy,
    }
}

fn setup_deployment_plan(
    profile: &str,
    answers: Option<&toml::Value>,
    options: SetupSurfaceOptions,
    existing_api_listen: Option<String>,
) -> SetupDeploymentPlan {
    let SetupSurfaceOptions {
        public_url,
        shell_enabled,
        reverse_proxy,
    } = options;
    let https = public_url
        .as_deref()
        .map(|url| url.starts_with("https://"))
        .unwrap_or(true);
    let direct_ip_mode = profile == "vps" && public_url.is_none();
    let default_api_listen = if direct_ip_mode {
        "0.0.0.0:50051"
    } else {
        "127.0.0.1:50051"
    };
    // Re-running setup must not silently reset a port the user genuinely
    // customized back to the profile default. But the bootstrap config
    // template already writes one of the two profile defaults before this
    // step runs, so only treat a value that differs from *both* known
    // defaults as an intentional customization worth preserving — otherwise
    // profile-driven switches (e.g. dropping a VPS domain to go direct-IP)
    // would never take effect on a second `captain setup` run.
    let api_listen = match existing_api_listen {
        Some(value) if value != "0.0.0.0:50051" && value != "127.0.0.1:50051" => value,
        _ => default_api_listen.to_string(),
    };
    let api_port = api_listen
        .rsplit(':')
        .next()
        .filter(|p| !p.is_empty())
        .unwrap_or("50051");
    let direct_url = if direct_ip_mode {
        Some(format!(
            "http://{}:{api_port}/terminal",
            setup_detect_vps_public_host(answers)
        ))
    } else {
        None
    };

    SetupDeploymentPlan {
        public_url,
        direct_url,
        api_listen,
        shell_enabled,
        reverse_proxy,
        https,
    }
}

fn setup_apply_surface_config(
    captain_dir: &Path,
    profile: &str,
    plan: &SetupDeploymentPlan,
) -> Result<(), String> {
    let patches = setup_surface_config_patches(profile, plan);
    captain_runtime::integrations::apply_config_patch(&captain_dir.join("config.toml"), &patches)?;
    restrict_file_permissions(&captain_dir.join("config.toml"));
    Ok(())
}

fn setup_surface_config_patches(profile: &str, plan: &SetupDeploymentPlan) -> Vec<ConfigPatch> {
    let mut patches = vec![
        ConfigPatch {
            path: vec![],
            key: "api_listen".to_string(),
            value: toml_edit::value(plan.api_listen.as_str()),
        },
        ConfigPatch {
            path: vec!["web_terminal".to_string()],
            key: "enabled".to_string(),
            value: toml_edit::value(true),
        },
        ConfigPatch {
            path: vec!["web_terminal".to_string()],
            key: "default_mode".to_string(),
            value: toml_edit::value("captain"),
        },
        ConfigPatch {
            path: vec!["web_terminal".to_string()],
            key: "allow_raw_shell".to_string(),
            value: toml_edit::value(plan.shell_enabled),
        },
        ConfigPatch {
            path: vec!["web_terminal".to_string()],
            key: "max_sessions".to_string(),
            value: toml_edit::value(4),
        },
        ConfigPatch {
            path: vec!["deployment".to_string()],
            key: "profile".to_string(),
            value: toml_edit::value(profile),
        },
        ConfigPatch {
            path: vec!["deployment".to_string()],
            key: "https".to_string(),
            value: toml_edit::value(plan.https),
        },
        ConfigPatch {
            path: vec!["deployment".to_string()],
            key: "reverse_proxy".to_string(),
            value: toml_edit::value(plan.reverse_proxy.as_str()),
        },
    ];
    if let Some(url) = &plan.public_url {
        patches.push(ConfigPatch {
            path: vec!["deployment".to_string()],
            key: "public_url".to_string(),
            value: toml_edit::value(url.as_str()),
        });
    }
    patches
}

fn setup_write_surface_artifacts(
    captain_dir: &Path,
    profile: &str,
    plan: &SetupDeploymentPlan,
) -> Result<Option<PathBuf>, String> {
    if profile == "vps" {
        plan.public_url
            .as_deref()
            .map(|url| setup_write_vps_caddyfile(captain_dir, url))
            .transpose()
    } else {
        Ok(None)
    }
}

fn setup_print_surface_summary(plan: &SetupDeploymentPlan, caddyfile_path: Option<&PathBuf>) {
    ui::kv_ok("Terminal web", "/terminal");
    ui::kv(
        "Mode Shell",
        if plan.shell_enabled {
            "activé explicitement"
        } else {
            "désactivé par défaut"
        },
    );
    if let Some(url) = &plan.public_url {
        ui::kv("URL publique", url);
    } else if let Some(url) = &plan.direct_url {
        ui::kv("Accès direct", url);
    }
    if let Some(path) = caddyfile_path {
        ui::kv("Caddyfile", &path.display().to_string());
    }
}

pub(crate) fn setup_normalize_public_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("local")
    {
        return None;
    }
    if trimmed.contains("://") {
        Some(trimmed.to_string())
    } else {
        Some(format!("https://{trimmed}"))
    }
}

pub(crate) fn setup_public_host(public_url: &str) -> Option<String> {
    let without_scheme = public_url
        .strip_prefix("https://")
        .or_else(|| public_url.strip_prefix("http://"))
        .unwrap_or(public_url);
    without_scheme
        .split('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn setup_detect_vps_public_host(answers: Option<&toml::Value>) -> String {
    if let Some(value) = setup_env_or_answer_any(
        "CAPTAIN_PUBLIC_IP",
        answers,
        &["deployment.public_ip", "public_ip"],
    ) {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("sh")
            .arg("-c")
            .arg("ip route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \\([^ ]*\\).*/\\1/p' | head -1")
            .output()
        {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !value.is_empty() && value != "127.0.0.1" {
                return value;
            }
        }
        if let Ok(output) = std::process::Command::new("hostname").arg("-I").output() {
            for token in String::from_utf8_lossy(&output.stdout).split_whitespace() {
                if !token.starts_with("127.") && token.contains('.') {
                    return token.to_string();
                }
            }
        }
    }

    "<IP_DU_VPS>".to_string()
}

fn setup_write_vps_caddyfile(captain_dir: &Path, public_url: &str) -> Result<PathBuf, String> {
    let host = setup_public_host(public_url).ok_or_else(|| "invalid public URL".to_string())?;
    let deploy_dir = captain_dir.join("deploy");
    std::fs::create_dir_all(&deploy_dir).map_err(|e| format!("create deploy dir: {e}"))?;
    let path = deploy_dir.join("Caddyfile");
    let contents = format!("{host} {{\n  encode zstd gzip\n  reverse_proxy 127.0.0.1:50051\n}}\n");
    std::fs::write(&path, contents).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}
