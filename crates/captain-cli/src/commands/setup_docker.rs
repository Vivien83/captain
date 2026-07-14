use colored::Colorize;

use crate::{prompt_input, ui};

const DOCKER_PROFILES: &[(&str, &str, &str)] = &[
    (
        "default",
        "Isolation max (recommandé)",
        "Volumes ~/.captain seul, pas de SSH, pas de Docker socket",
    ),
    (
        "personal",
        "Accès Desktop/Documents/Downloads + SSH host",
        "Pour usage personnel sur ta machine",
    ),
    (
        "trusted",
        "Tout $HOME + Docker socket",
        "Tu fais confiance aux agents qui tournent dedans",
    ),
    (
        "yolo",
        "Privileged + network host + FS root",
        "ATTENTION : zéro garde-fou",
    ),
];

const DEFAULT_CAPTAIN_DOCKER_IMAGE: &str = "ghcr.io/vivien83/captain-agent-os:alpha";

fn captain_docker_image() -> String {
    std::env::var("CAPTAIN_DOCKER_IMAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CAPTAIN_DOCKER_IMAGE.to_string())
}

/// Probe `docker info` to decide whether to offer the Docker install path.
/// 5s timeout — if Docker daemon is sleeping, we pretend it's absent.
pub(crate) fn setup_check_docker() -> bool {
    let out = std::process::Command::new("docker")
        .args(["info", "--format", "{{.ID}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(out, Ok(s) if s.success())
}

pub(crate) fn setup_pick_install_mode(docker_present: bool) -> &'static str {
    ui::section("Mode d'installation");
    if !docker_present {
        ui::check_warn("Docker non détecté — installation locale (binaire)");
        ui::hint(
            "Installe Docker pour bénéficier de l'isolation : https://docs.docker.com/get-docker/",
        );
        return "local";
    }
    let image = captain_docker_image();
    println!("    1. Docker  (recommandé — isolation, image officielle {image})");
    println!("    2. Local   (binaire `captain` directement sur ta machine)");
    let answer = prompt_input("  Choix [1] : ");
    if answer.starts_with('2') {
        "local"
    } else {
        "docker"
    }
}

pub(crate) fn setup_pick_docker_profile() -> &'static str {
    ui::blank();
    ui::section("Profil de sécurité Docker");
    for (i, (id, label, desc)) in DOCKER_PROFILES.iter().enumerate() {
        println!("    {}. {:<10} {}", i + 1, id, label.bold());
        println!("       {}", desc.dimmed());
    }
    let answer = prompt_input("  Choix [1] : ");
    let idx = answer
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=DOCKER_PROFILES.len()).contains(n))
        .map(|n| n - 1)
        .unwrap_or(0);
    DOCKER_PROFILES[idx].0
}

/// Best-effort `docker pull`. Streams stdout/stderr so the user sees progress;
/// failure is logged but non-fatal (they can still `captain start` locally,
/// or retry the pull later).
pub(crate) fn setup_docker_pull() {
    ui::blank();
    let image = captain_docker_image();
    ui::step(&format!(
        "docker pull {image} (peut prendre quelques minutes)…"
    ));
    let status = std::process::Command::new("docker")
        .args(["pull", image.as_str()])
        .status();
    match status {
        Ok(s) if s.success() => ui::success("Image Captain téléchargée"),
        Ok(s) => ui::warn_with_fix(
            &format!("docker pull a retourné {s}"),
            &format!("Réessaie : docker pull {image}"),
        ),
        Err(e) => ui::warn_with_fix(
            &format!("Échec exécution docker : {e}"),
            "Vérifie que le daemon Docker tourne",
        ),
    }
}

/// Detach a `docker compose -f docker-compose.yml [-f docker-compose.<profile>.yml] up -d`
/// in the current working directory. Falls back to v1 `docker-compose` if
/// `docker compose` v2 plugin is missing.
///
/// The non-default profile files (personal/trusted/yolo) are overlays: they
/// only set `environment`/`volumes`/etc, with no `image`/`build`, so they are
/// not a valid standalone compose project — they must be layered on top of
/// the base `docker-compose.yml` via multiple `-f` flags, never passed alone.
pub(crate) fn setup_launch_docker(profile: &str) {
    ui::blank();
    let mut compose_files = vec!["docker-compose.yml".to_string()];
    if profile != "default" {
        compose_files.push(format!("docker-compose.{profile}.yml"));
    }

    for file in &compose_files {
        if !std::path::Path::new(file).exists() {
            ui::warn_with_fix(
                &format!("{file} introuvable dans le répertoire courant"),
                "Lance la commande depuis la racine du repo Captain, ou utilise `captain start`",
            );
            return;
        }
    }

    let file_args: Vec<&str> = compose_files
        .iter()
        .flat_map(|f| ["-f", f.as_str()])
        .collect();
    let display_files = compose_files.join(" -f ");
    ui::step(&format!("docker compose -f {display_files} up -d…"));
    let v2 = std::process::Command::new("docker")
        .arg("compose")
        .args(&file_args)
        .args(["up", "-d"])
        .status();
    let ok = match v2 {
        Ok(s) if s.success() => true,
        _ => {
            ui::hint("Tentative avec `docker-compose` (v1)…");
            std::process::Command::new("docker-compose")
                .args(&file_args)
                .args(["up", "-d"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
    };

    if ok {
        ui::success("Captain tourne en Docker en arrière-plan");
        ui::kv("Web terminal", "http://127.0.0.1:50051/terminal");
        ui::kv(
            "Logs",
            &format!("docker compose -f {display_files} logs -f"),
        );
        ui::kv("Stop", &format!("docker compose -f {display_files} down"));
    } else {
        ui::error_with_fix(
            "Échec du lancement Docker",
            "Lance manuellement : `docker compose up -d` puis `captain status`",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_CAPTAIN_DOCKER_IMAGE;

    #[test]
    fn public_setup_image_stays_on_the_alpha_channel() {
        assert_eq!(
            DEFAULT_CAPTAIN_DOCKER_IMAGE,
            "ghcr.io/vivien83/captain-agent-os:alpha"
        );
    }
}
