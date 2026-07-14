use std::path::{Path, PathBuf};

use super::setup_access::{setup_bootstrap_access, SetupAccessOutcome};
use super::setup_docker::{
    setup_check_docker, setup_docker_pull, setup_launch_docker, setup_pick_docker_profile,
    setup_pick_install_mode,
};
use super::setup_integrations::{
    setup_configure_stt_non_interactive, setup_configure_telegram_non_interactive,
    setup_configure_tts_non_interactive,
};
use super::setup_model::{
    setup_configure_base_model, setup_configure_base_model_non_interactive,
    NonInteractiveBaseModel, SetupBaseModel,
};
use super::setup_options::{setup_offer_telegram, setup_offer_voice_stack, SetupVoiceOutcome};
use super::setup_profile::{
    apply_first_run_profile_to_config, persist_first_run_profile_best_effort,
    setup_personalize_assistant, setup_profile_from_non_interactive, write_first_run_user_profile,
    FirstRunProfile,
};
use super::setup_support::setup_load_answers;
use super::setup_surface::{setup_configure_product_surface, SetupDeploymentOutcome};
use crate::{
    bundled_agents, cli_captain_home, cmd_init, prompt_input, restrict_dir_permissions, ui,
};

#[derive(Clone, Copy)]
struct InteractiveInstallChoice {
    mode: &'static str,
    docker_profile: Option<&'static str>,
}

struct InteractiveSetupSummary<'a> {
    captain_dir: &'a Path,
    base_model: &'a SetupBaseModel,
    first_run_profile: &'a FirstRunProfile,
    access: &'a SetupAccessOutcome,
    deployment: &'a SetupDeploymentOutcome,
    install: InteractiveInstallChoice,
    channel_done: bool,
    voice_outcome: &'a SetupVoiceOutcome,
}

struct NonInteractiveIntegrationOutcome {
    telegram_done: bool,
    stt_done: bool,
    tts_done: bool,
}

struct NonInteractiveSetupSummary<'a> {
    profile: &'a str,
    base_model: &'a NonInteractiveBaseModel,
    access: SetupAccessOutcome,
    deployment: SetupDeploymentOutcome,
    integrations: NonInteractiveIntegrationOutcome,
}

/// `captain setup` — one-shot wizard. Replaces the previous alias-to-init.
pub(crate) fn cmd_setup_minimal(profile: Option<&str>) {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        ui::hint("Non-interactive terminal detected — running `captain setup --quick` instead");
        ui::hint("For the guided wizard, run: captain setup (in a terminal)");
        cmd_setup_non_interactive(profile, true, false, None);
        return;
    }

    setup_print_interactive_intro();
    let setup_profile = setup_validate_profile_or_exit(profile);

    let captain_dir = setup_prepare_captain_dir_or_exit();
    let base_model = setup_configure_base_model(&captain_dir);
    let access = setup_bootstrap_access_or_exit(&captain_dir, None, true);
    let deployment = setup_configure_surface_or_exit(&captain_dir, setup_profile, None, true);
    let install = setup_choose_interactive_install(profile, setup_profile);

    let mut first_run_profile = setup_personalize_assistant(&captain_dir);
    setup_persist_interactive_profile(&captain_dir, &first_run_profile);

    let (channel_done, voice_outcome) =
        setup_configure_interactive_channels(&captain_dir, &mut first_run_profile);
    setup_persist_interactive_profile(&captain_dir, &first_run_profile);

    setup_print_interactive_summary(InteractiveSetupSummary {
        captain_dir: &captain_dir,
        base_model: &base_model,
        first_run_profile: &first_run_profile,
        access: &access,
        deployment: &deployment,
        install,
        channel_done,
        voice_outcome: &voice_outcome,
    });

    if !base_model.ready {
        setup_warn_provider_not_ready();
        return;
    }

    setup_offer_interactive_launch(install);
}

fn setup_print_interactive_intro() {
    ui::banner();
    ui::blank();
    ui::section("Captain Setup — installation rapide");
    println!("  Provider prêt → préférences → canaux → lancement.");
    ui::blank();
}

fn setup_prepare_captain_dir_or_exit() -> PathBuf {
    let captain_dir = cli_captain_home();
    if let Err(e) = std::fs::create_dir_all(&captain_dir) {
        ui::error_with_fix(
            &format!("Création {} : {e}", captain_dir.display()),
            "Vérifie les permissions sur ton dossier home",
        );
        std::process::exit(1);
    }
    restrict_dir_permissions(&captain_dir);
    for sub in ["data", "agents"] {
        let _ = std::fs::create_dir_all(captain_dir.join(sub));
    }
    bundled_agents::install_bundled_agents(&captain_dir.join("agents"));
    captain_dir
}

fn setup_bootstrap_access_or_exit(
    captain_dir: &Path,
    answers: Option<&toml::Value>,
    interactive: bool,
) -> SetupAccessOutcome {
    setup_bootstrap_access(captain_dir, answers, interactive).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Configuration auth/web : {e}"),
            "Vérifie les permissions de ~/.captain/config.toml",
        );
        std::process::exit(1);
    })
}

fn setup_configure_surface_or_exit(
    captain_dir: &Path,
    profile: &str,
    answers: Option<&toml::Value>,
    interactive: bool,
) -> SetupDeploymentOutcome {
    setup_configure_product_surface(captain_dir, profile, answers, interactive).unwrap_or_else(
        |e| {
            ui::error_with_fix(
                &format!("Configuration surface web : {e}"),
                "Vérifie les permissions de ~/.captain/config.toml",
            );
            std::process::exit(1);
        },
    )
}

fn setup_choose_interactive_install(
    requested_profile: Option<&str>,
    setup_profile: &str,
) -> InteractiveInstallChoice {
    if requested_profile.is_some() {
        ui::blank();
        ui::section("Mode d'installation");
        ui::check_ok(&format!("Profil installateur : {setup_profile}"));
        ui::hint("Le binaire précompilé est déjà installé ; aucun build local n'est requis.");
        return InteractiveInstallChoice {
            mode: "local",
            docker_profile: None,
        };
    }

    let docker_present = setup_check_docker();
    let mode = setup_pick_install_mode(docker_present);
    let docker_profile = if mode == "docker" {
        let profile = setup_pick_docker_profile();
        setup_docker_pull();
        Some(profile)
    } else {
        None
    };
    InteractiveInstallChoice {
        mode,
        docker_profile,
    }
}

fn setup_persist_interactive_profile(captain_dir: &Path, first_run_profile: &FirstRunProfile) {
    persist_first_run_profile_best_effort(
        captain_dir,
        first_run_profile,
        "Tu peux la renseigner plus tard dans [assistant] de ~/.captain/config.toml",
        "Tu peux le renseigner plus tard dans ~/.captain/USER.md",
    );
}

fn setup_configure_interactive_channels(
    captain_dir: &Path,
    first_run_profile: &mut FirstRunProfile,
) -> (bool, SetupVoiceOutcome) {
    ui::blank();
    let channel_done = setup_offer_telegram(captain_dir);
    ui::blank();
    let voice_outcome = setup_offer_voice_stack(captain_dir, &first_run_profile.voice_preference);
    setup_apply_voice_provider(first_run_profile, voice_outcome.tts_provider);
    (channel_done, voice_outcome)
}

fn setup_apply_voice_provider(
    first_run_profile: &mut FirstRunProfile,
    provider: Option<&'static str>,
) {
    if let Some(provider) = provider {
        first_run_profile.voice_preference = match provider {
            "elevenlabs" => "ElevenLabs".to_string(),
            "openai" => "OpenAI Nova".to_string(),
            _ => provider.to_string(),
        };
    }
}

fn setup_print_interactive_summary(summary: InteractiveSetupSummary<'_>) {
    ui::blank();
    ui::section("Récapitulatif");
    setup_print_model_profile_summary(summary.base_model, summary.first_run_profile);
    setup_print_access_summary(summary.access);
    setup_print_deployment_summary(summary.deployment);
    setup_print_install_summary(summary.install);
    setup_print_optional_stack_summary(summary.channel_done, summary.voice_outcome);
    ui::kv(
        "Config",
        &summary
            .captain_dir
            .join("config.toml")
            .display()
            .to_string(),
    );
}

fn setup_print_model_profile_summary(
    base_model: &SetupBaseModel,
    first_run_profile: &FirstRunProfile,
) {
    if base_model.ready {
        ui::kv_ok("Provider", base_model.provider.display);
    } else {
        ui::kv(
            "Provider",
            &format!("{} (non prêt)", base_model.provider.display),
        );
    }
    ui::kv("Modèle", &base_model.model);
    ui::kv("Assistant", &first_run_profile.assistant_name);
    ui::kv("Style", &first_run_profile.assistant_style);
    ui::kv("Langue", &first_run_profile.user_language);
    ui::kv("Timezone", &first_run_profile.timezone);
}

fn setup_print_access_summary(access: &SetupAccessOutcome) {
    ui::kv("Admin", &access.username);
    ui::kv(
        "Auth initiale",
        setup_auth_initial_label(
            access.generated_password.is_some(),
            access.generated_api_key,
        ),
    );
    if let Some(path) = &access.credentials_path {
        ui::kv("Identifiants initiaux", &path.display().to_string());
    }
}

fn setup_print_deployment_summary(deployment: &SetupDeploymentOutcome) {
    ui::kv(
        "Terminal web",
        if deployment.shell_enabled {
            "/terminal + Shell"
        } else {
            "/terminal"
        },
    );
    ui::kv("Écoute web/API", &deployment.api_listen);
    if let Some(url) = &deployment.public_url {
        ui::kv("URL publique", url);
    } else if let Some(url) = &deployment.direct_url {
        ui::kv("URL directe VPS", url);
    }
    if let Some(path) = &deployment.caddyfile_path {
        ui::kv("Caddyfile", &path.display().to_string());
    }
}

fn setup_print_install_summary(install: InteractiveInstallChoice) {
    ui::kv("Mode", setup_install_mode_label(install.mode));
    if let Some(profile) = install.docker_profile {
        ui::kv("Profil Docker", profile);
    }
}

fn setup_print_optional_stack_summary(channel_done: bool, voice_outcome: &SetupVoiceOutcome) {
    ui::kv("Telegram", setup_configured_fr(channel_done));
    ui::kv("STT", setup_configured_fr(voice_outcome.stt_done));
    ui::kv("TTS", setup_configured_fr(voice_outcome.tts_done));
}

fn setup_warn_provider_not_ready() {
    ui::blank();
    ui::warn_with_fix(
        "Captain est installé mais le provider n'est pas prêt.",
        "Relance `captain setup` ou `captain auth login <provider>` avant `captain start`.",
    );
}

fn setup_offer_interactive_launch(install: InteractiveInstallChoice) {
    ui::blank();
    let answer = prompt_input("  Lancer Captain maintenant ? [Y/n] ");
    let want_launch = answer.is_empty() || answer.starts_with(['y', 'Y']);

    if want_launch {
        if install.mode == "docker" {
            setup_launch_docker(install.docker_profile.unwrap_or("default"));
        } else {
            setup_launch_native_service();
        }
    } else {
        ui::blank();
        ui::next_steps(&[
            "Démarrer plus tard : captain service install --start",
            "Ouvrir le chat    : captain chat",
            "Ajouter un canal  : captain integration setup telegram --no-test",
        ]);
    }
}

/// Actually starts Captain after setup instead of just printing instructions
/// the user would otherwise have to run in a second terminal (`captain start`
/// blocks its terminal by design, since launchd/systemd run it in foreground).
///
/// Shells out to `captain service install --start` / `captain service start`
/// as child processes rather than calling their in-process implementations
/// directly: those can call `std::process::exit` on failure (e.g. a service
/// definition already exists from a previous run), which would otherwise
/// abort this `captain setup` wizard itself — and `install.sh` runs
/// interactive setup under `set -e`, so that exit would kill the whole
/// installer right after telling the user setup succeeded.
fn setup_launch_native_service() {
    ui::blank();
    if run_captain_subcommand(&["service", "install", "--start"]) {
        return;
    }
    if run_captain_subcommand(&["service", "start"]) {
        return;
    }
    ui::blank();
    ui::warn_with_fix(
        "Impossible de démarrer Captain automatiquement.",
        "Lance `captain service install --start`, ou `captain start &` pour un démarrage manuel en arrière-plan (`captain start` sans `&` reste au premier plan, c'est voulu pour l'usage via un service système).",
    );
}

fn run_captain_subcommand(args: &[&str]) -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| std::process::Command::new(exe).args(args).status().ok())
        .map(|status| status.success())
        .unwrap_or(false)
}

fn setup_auth_initial_label(generated_password: bool, generated_api_key: bool) -> &'static str {
    match (generated_password, generated_api_key) {
        (true, true) => "password + api_key générés",
        (true, false) => "password généré",
        (false, true) => "api_key générée",
        (false, false) => "déjà configurée",
    }
}

fn setup_install_mode_label(install_mode: &str) -> &'static str {
    if install_mode == "docker" {
        "Docker"
    } else {
        "Binaire local"
    }
}

fn setup_configured_fr(done: bool) -> &'static str {
    if done {
        "✓ configuré"
    } else {
        "non"
    }
}

pub(crate) fn cmd_setup_non_interactive(
    profile: Option<&str>,
    confirmed: bool,
    from_env: bool,
    answers_path: Option<&Path>,
) {
    if !confirmed {
        ui::error("Non-interactive setup requires --yes, --quick, or --from-env.");
        std::process::exit(1);
    }
    let profile = setup_validate_profile_or_exit(profile);
    if setup_should_run_quick_non_interactive(from_env, answers_path) {
        setup_run_quick_non_interactive(profile);
        return;
    }

    setup_run_answered_non_interactive(profile, answers_path);
}

fn setup_should_run_quick_non_interactive(from_env: bool, answers_path: Option<&Path>) -> bool {
    !from_env && answers_path.is_none()
}

fn setup_run_quick_non_interactive(profile: &str) {
    cmd_init(true);
    let captain_dir = cli_captain_home();
    let access = setup_bootstrap_access_noninteractive_or_exit(&captain_dir, None);
    let deployment = setup_configure_surface_noninteractive_or_exit(&captain_dir, profile, None);
    setup_print_quick_noninteractive_summary(profile, access, deployment);
}

fn setup_run_answered_non_interactive(profile: &str, answers_path: Option<&Path>) {
    let answers = setup_load_answers(answers_path);
    cmd_init(true);
    let captain_dir = cli_captain_home();
    let base_model = setup_configure_base_model_non_interactive(&captain_dir, answers.as_ref());
    let access = setup_bootstrap_access_noninteractive_or_exit(&captain_dir, answers.as_ref());
    let deployment =
        setup_configure_surface_noninteractive_or_exit(&captain_dir, profile, answers.as_ref());
    let mut first_run_profile = setup_profile_from_non_interactive(answers.as_ref());
    let integrations =
        setup_configure_noninteractive_integrations(answers.as_ref(), &mut first_run_profile);
    setup_persist_noninteractive_profile(&captain_dir, &first_run_profile);
    setup_print_noninteractive_summary(NonInteractiveSetupSummary {
        profile,
        base_model: &base_model,
        access,
        deployment,
        integrations,
    });
}

fn setup_bootstrap_access_noninteractive_or_exit(
    captain_dir: &Path,
    answers: Option<&toml::Value>,
) -> SetupAccessOutcome {
    setup_bootstrap_access(captain_dir, answers, false).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Auth bootstrap failed: {e}"),
            "Check permissions on ~/.captain/config.toml.",
        );
        std::process::exit(1);
    })
}

fn setup_configure_surface_noninteractive_or_exit(
    captain_dir: &Path,
    profile: &str,
    answers: Option<&toml::Value>,
) -> SetupDeploymentOutcome {
    setup_configure_product_surface(captain_dir, profile, answers, false).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Web surface setup failed: {e}"),
            "Check permissions on ~/.captain/config.toml.",
        );
        std::process::exit(1);
    })
}

fn setup_configure_noninteractive_integrations(
    answers: Option<&toml::Value>,
    first_run_profile: &mut FirstRunProfile,
) -> NonInteractiveIntegrationOutcome {
    let telegram_done = setup_configure_telegram_non_interactive(answers);
    let stt_done = setup_configure_stt_non_interactive(answers);
    let (tts_done, tts_provider) =
        setup_configure_tts_non_interactive(answers, &first_run_profile.voice_preference);
    setup_apply_voice_provider(first_run_profile, tts_provider);
    NonInteractiveIntegrationOutcome {
        telegram_done,
        stt_done,
        tts_done,
    }
}

fn setup_persist_noninteractive_profile(captain_dir: &Path, first_run_profile: &FirstRunProfile) {
    if let Err(e) = apply_first_run_profile_to_config(captain_dir, first_run_profile) {
        ui::warn_with_fix(
            &format!("Personalization config not written: {e}"),
            "Edit [assistant] in ~/.captain/config.toml manually.",
        );
    }
    if let Err(e) = write_first_run_user_profile(captain_dir, first_run_profile) {
        ui::warn_with_fix(
            &format!("USER.md profile not written: {e}"),
            "Edit ~/.captain/USER.md manually.",
        );
    }
}

fn setup_print_quick_noninteractive_summary(
    profile: &str,
    access: SetupAccessOutcome,
    deployment: SetupDeploymentOutcome,
) {
    ui::success(&format!("Quick setup complete (profile={profile})."));
    setup_print_noninteractive_access(&access);
    if let Some(url) = deployment.public_url {
        ui::kv("Public URL", &url);
    }
    ui::hint("Run `captain doctor --full` to verify the installation.");
}

fn setup_print_noninteractive_summary(summary: NonInteractiveSetupSummary<'_>) {
    ui::success(&format!(
        "Non-interactive setup complete (profile={}).",
        summary.profile
    ));
    setup_print_noninteractive_provider(summary.base_model);
    setup_print_noninteractive_integrations(&summary.integrations);
    setup_print_noninteractive_access(&summary.access);
    setup_print_noninteractive_deployment(&summary.deployment);
    ui::hint("Run `captain doctor --full` to verify the installation.");
}

fn setup_print_noninteractive_provider(base_model: &NonInteractiveBaseModel) {
    ui::kv(
        "Provider",
        &format!(
            "{}/{}{}",
            base_model.provider,
            base_model.model,
            if base_model.ready { "" } else { " (not ready)" }
        ),
    );
}

fn setup_print_noninteractive_integrations(integrations: &NonInteractiveIntegrationOutcome) {
    ui::kv("Telegram", setup_configured_en(integrations.telegram_done));
    ui::kv("STT", setup_configured_en(integrations.stt_done));
    ui::kv("TTS", setup_configured_en(integrations.tts_done));
}

fn setup_print_noninteractive_access(access: &SetupAccessOutcome) {
    ui::kv("Admin", &access.username);
    ui::kv(
        "Auth",
        setup_auth_label_en(
            access.generated_password.is_some(),
            access.generated_api_key,
        ),
    );
    if let Some(path) = &access.credentials_path {
        ui::kv("Initial access", &path.display().to_string());
    }
}

fn setup_print_noninteractive_deployment(deployment: &SetupDeploymentOutcome) {
    ui::kv(
        "Web terminal",
        setup_web_terminal_label_en(deployment.shell_enabled),
    );
    ui::kv("Web/API listen", &deployment.api_listen);
    if let Some(url) = &deployment.public_url {
        ui::kv("Public URL", url);
    } else if let Some(url) = &deployment.direct_url {
        ui::kv("Direct VPS URL", url);
    }
    if let Some(path) = &deployment.caddyfile_path {
        ui::kv("Caddyfile", &path.display().to_string());
    }
}

fn setup_auth_label_en(generated_password: bool, generated_api_key: bool) -> &'static str {
    match (generated_password, generated_api_key) {
        (true, true) => "generated password + api_key",
        (true, false) => "generated password",
        (false, true) => "generated api_key",
        (false, false) => "already configured",
    }
}

fn setup_configured_en(done: bool) -> &'static str {
    if done {
        "configured"
    } else {
        "not configured"
    }
}

fn setup_web_terminal_label_en(shell_enabled: bool) -> &'static str {
    if shell_enabled {
        "/terminal + shell"
    } else {
        "/terminal"
    }
}

fn setup_validate_profile_or_exit(profile: Option<&str>) -> &str {
    let profile = profile.unwrap_or("core");
    match profile {
        "core" | "vps" | "desktop" | "full-media" => profile,
        _ => {
            ui::error("Unsupported setup profile. Use: core, vps, desktop, full-media");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_run_profile_with_voice(voice_preference: &str) -> FirstRunProfile {
        FirstRunProfile {
            assistant_name: "Captain".to_string(),
            assistant_style: "balanced".to_string(),
            user_name: None,
            user_language: "fr".to_string(),
            timezone: "Europe/Paris".to_string(),
            voice_preference: voice_preference.to_string(),
            notifications: "important only".to_string(),
            privacy: "ask first".to_string(),
        }
    }

    #[test]
    fn setup_auth_initial_label_covers_generation_states() {
        assert_eq!(
            setup_auth_initial_label(true, true),
            "password + api_key générés"
        );
        assert_eq!(setup_auth_initial_label(true, false), "password généré");
        assert_eq!(setup_auth_initial_label(false, true), "api_key générée");
        assert_eq!(setup_auth_initial_label(false, false), "déjà configurée");
    }

    #[test]
    fn setup_summary_labels_keep_french_operator_copy() {
        assert_eq!(setup_install_mode_label("docker"), "Docker");
        assert_eq!(setup_install_mode_label("local"), "Binaire local");
        assert_eq!(setup_configured_fr(true), "✓ configuré");
        assert_eq!(setup_configured_fr(false), "non");
    }

    #[test]
    fn setup_apply_voice_provider_updates_known_display_names() {
        let mut profile = first_run_profile_with_voice("none");
        setup_apply_voice_provider(&mut profile, Some("elevenlabs"));
        assert_eq!(profile.voice_preference, "ElevenLabs");

        setup_apply_voice_provider(&mut profile, Some("openai"));
        assert_eq!(profile.voice_preference, "OpenAI Nova");

        setup_apply_voice_provider(&mut profile, Some("custom"));
        assert_eq!(profile.voice_preference, "custom");

        setup_apply_voice_provider(&mut profile, None);
        assert_eq!(profile.voice_preference, "custom");
    }

    #[test]
    fn noninteractive_summary_labels_keep_existing_english_copy() {
        assert_eq!(
            setup_auth_label_en(true, true),
            "generated password + api_key"
        );
        assert_eq!(setup_auth_label_en(true, false), "generated password");
        assert_eq!(setup_auth_label_en(false, true), "generated api_key");
        assert_eq!(setup_auth_label_en(false, false), "already configured");
        assert_eq!(setup_configured_en(true), "configured");
        assert_eq!(setup_configured_en(false), "not configured");
        assert_eq!(setup_web_terminal_label_en(true), "/terminal + shell");
        assert_eq!(setup_web_terminal_label_en(false), "/terminal");
    }
}
