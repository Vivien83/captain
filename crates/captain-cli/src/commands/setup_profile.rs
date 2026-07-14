use std::path::Path;

use colored::Colorize;

use super::setup_support::{setup_config_string, setup_env_or_answer_any, setup_read_config_value};
use crate::{prompt_input, restrict_file_permissions, ui};

#[derive(Debug, Clone)]
pub(crate) struct FirstRunProfile {
    pub(crate) assistant_name: String,
    pub(crate) assistant_style: String,
    pub(crate) user_name: Option<String>,
    pub(crate) user_language: String,
    pub(crate) timezone: String,
    pub(crate) voice_preference: String,
    pub(crate) notifications: String,
    pub(crate) privacy: String,
}

const ASSISTANT_STYLE_CHOICES: &[(&str, &str, &str)] = &[
    ("balanced", "Naturel", "chaleureux, concis, autonome"),
    ("concise", "Concis", "direct, peu de texte"),
    ("professional", "Professionnel", "poli, structuré, précis"),
    (
        "developer",
        "Développeur",
        "technique, fichiers, commandes, vérification",
    ),
    (
        "friendly",
        "Compagnon",
        "amical, détendu, toujours efficace",
    ),
    ("classic", "Assistant", "neutre, sobre, serviable"),
];

pub(crate) fn setup_personalize_assistant(captain_dir: &Path) -> FirstRunProfile {
    ui::blank();
    ui::section("Personnalisation");
    println!("  Ces réglages définissent l'identité visible et le style de réponse.");

    let existing = setup_existing_first_run_profile(captain_dir);
    let assistant_raw = prompt_input(&format!(
        "  Nom de l'assistant [{}] : ",
        existing.assistant_name
    ));
    let assistant_name = sanitize_setup_text(&assistant_raw, &existing.assistant_name, 64);

    ui::blank();
    for (i, (_, label, desc)) in ASSISTANT_STYLE_CHOICES.iter().enumerate() {
        println!("    {}. {:<14} {}", i + 1, label, desc.dimmed());
    }
    let default_style_idx = ASSISTANT_STYLE_CHOICES
        .iter()
        .position(|(id, _, _)| *id == existing.assistant_style)
        .unwrap_or(0);
    let style_answer = prompt_input(&format!("  Style [{}] : ", default_style_idx + 1));
    let style_idx = style_answer
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=ASSISTANT_STYLE_CHOICES.len()).contains(n))
        .map(|n| n - 1)
        .unwrap_or(default_style_idx);
    let assistant_style = ASSISTANT_STYLE_CHOICES[style_idx].0.to_string();

    ui::blank();
    let user_prompt = existing
        .user_name
        .as_deref()
        .map(|name| format!("  Comment dois-je t'appeler ? [{name}] : "))
        .unwrap_or_else(|| "  Comment dois-je t'appeler ? (optionnel) : ".to_string());
    let user_raw = prompt_input(&user_prompt);
    let user_name = sanitize_optional_setup_text(&user_raw, 64).or(existing.user_name);

    let detected_language = detect_setup_language();
    let default_language = if existing.user_language.trim().is_empty() {
        detected_language
    } else {
        existing.user_language
    };
    let language_raw = prompt_input(&format!("  Langue principale [{default_language}] : "));
    let user_language = sanitize_setup_text(&language_raw, &default_language, 16);

    let detected_timezone = detect_setup_timezone();
    let default_timezone = if existing.timezone.trim().is_empty() {
        detected_timezone
    } else {
        existing.timezone
    };
    let timezone_raw = prompt_input(&format!("  Timezone [{default_timezone}] : "));
    let timezone = sanitize_setup_text(&timezone_raw, &default_timezone, 64);

    let voice_raw = prompt_input(&format!(
        "  Préférence audio (openai nova | elevenlabs | none) [{}] : ",
        existing.voice_preference
    ));
    let voice_preference = sanitize_setup_text(&voice_raw, &existing.voice_preference, 80);

    let notifications_raw = prompt_input(&format!(
        "  Notifications (ex: important only, digest, silencieux) [{}] : ",
        existing.notifications
    ));
    let notifications = sanitize_setup_text(&notifications_raw, &existing.notifications, 120);

    let privacy_raw = prompt_input(&format!(
        "  Limites de confidentialité / mémoire [{}] : ",
        existing.privacy
    ));
    let privacy = sanitize_setup_text(&privacy_raw, &existing.privacy, 240);

    FirstRunProfile {
        assistant_name,
        assistant_style,
        user_name,
        user_language,
        timezone,
        voice_preference,
        notifications,
        privacy,
    }
}

pub(crate) fn setup_profile_from_non_interactive(answers: Option<&toml::Value>) -> FirstRunProfile {
    let assistant_name = setup_env_or_answer_any(
        "CAPTAIN_ASSISTANT_NAME",
        answers,
        &["assistant_name", "assistant.display_name"],
    )
    .map(|value| sanitize_setup_text(&value, "Captain", 64))
    .unwrap_or_else(|| "Captain".to_string());
    let assistant_style = setup_env_or_answer_any(
        "CAPTAIN_ASSISTANT_STYLE",
        answers,
        &["assistant_style", "assistant.style"],
    )
    .map(|value| sanitize_setup_text(&value, "balanced", 64))
    .unwrap_or_else(|| "balanced".to_string());
    let user_name = setup_env_or_answer_any(
        "CAPTAIN_USER_NAME",
        answers,
        &[
            "user_name",
            "user.preferred_name",
            "assistant.preferred_name",
        ],
    )
    .and_then(|value| sanitize_optional_setup_text(&value, 64));
    let detected_language = detect_setup_language();
    let user_language =
        setup_env_or_answer_any("CAPTAIN_LANGUAGE", answers, &["language", "user.language"])
            .map(|value| sanitize_setup_text(&value, &detected_language, 16))
            .unwrap_or(detected_language);
    let detected_timezone = detect_setup_timezone();
    let timezone =
        setup_env_or_answer_any("CAPTAIN_TIMEZONE", answers, &["timezone", "user.timezone"])
            .map(|value| sanitize_setup_text(&value, &detected_timezone, 64))
            .unwrap_or(detected_timezone);
    let voice_preference = setup_env_or_answer_any(
        "CAPTAIN_VOICE_PREFERENCE",
        answers,
        &[
            "voice_preference",
            "assistant.voice_preference",
            "tts.provider",
        ],
    )
    .map(|value| sanitize_setup_text(&value, "none", 80))
    .unwrap_or_else(|| "none".to_string());
    let notifications = setup_env_or_answer_any(
        "CAPTAIN_NOTIFICATIONS",
        answers,
        &["notifications", "assistant.notifications"],
    )
    .map(|value| sanitize_setup_text(&value, "important only", 120))
    .unwrap_or_else(|| "important only".to_string());
    let privacy = setup_env_or_answer_any(
        "CAPTAIN_PRIVACY",
        answers,
        &["privacy", "assistant.privacy", "user.privacy"],
    )
    .map(|value| {
        sanitize_setup_text(
            &value,
            "ask before memorizing sensitive or private information",
            240,
        )
    })
    .unwrap_or_else(|| "ask before memorizing sensitive or private information".to_string());

    FirstRunProfile {
        assistant_name,
        assistant_style,
        user_name,
        user_language,
        timezone,
        voice_preference,
        notifications,
        privacy,
    }
}

pub(crate) fn persist_first_run_profile_best_effort(
    captain_dir: &Path,
    profile: &FirstRunProfile,
    config_fix: &str,
    user_fix: &str,
) {
    if let Err(e) = apply_first_run_profile_to_config(captain_dir, profile) {
        ui::warn_with_fix(
            &format!("Personnalisation config non écrite : {e}"),
            config_fix,
        );
    }
    if let Err(e) = write_first_run_user_profile(captain_dir, profile) {
        ui::warn_with_fix(&format!("Profil utilisateur non écrit : {e}"), user_fix);
    }
}

pub(crate) fn apply_first_run_profile_to_config(
    captain_dir: &Path,
    profile: &FirstRunProfile,
) -> Result<(), String> {
    let config_path = captain_dir.join("config.toml");
    let patches = vec![
        captain_runtime::integrations::ConfigPatch {
            path: vec![],
            key: "language".to_string(),
            value: toml_edit::value(profile.user_language.as_str()),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec![],
            key: "timezone".to_string(),
            value: toml_edit::value(profile.timezone.as_str()),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["assistant".to_string()],
            key: "display_name".to_string(),
            value: toml_edit::value(profile.assistant_name.as_str()),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["assistant".to_string()],
            key: "style".to_string(),
            value: toml_edit::value(profile.assistant_style.as_str()),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["assistant".to_string()],
            key: "onboarding_completed".to_string(),
            value: toml_edit::value(true),
        },
    ];
    captain_runtime::integrations::apply_config_patch(&config_path, &patches)?;
    restrict_file_permissions(&config_path);
    Ok(())
}

pub(crate) fn write_first_run_user_profile(
    captain_dir: &Path,
    profile: &FirstRunProfile,
) -> Result<(), String> {
    const START: &str = "<!-- captain:first-run-profile:start -->";
    const END: &str = "<!-- captain:first-run-profile:end -->";

    let path = captain_dir.join("USER.md");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let preferred_name = profile
        .user_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("(not set)");
    let block = format!(
        "{START}\n# User profile\n- Preferred name: {preferred_name}\n- Preferred language: {}\n- Timezone: {}\n- Assistant display name: {}\n- Assistant style: {}\n- Voice preference: {}\n- Notification preference: {}\n- Privacy boundaries: {}\n- First interview status: completed during setup\n{END}\n",
        profile.user_language,
        profile.timezone,
        profile.assistant_name,
        profile.assistant_style,
        profile.voice_preference,
        profile.notifications,
        profile.privacy
    );

    let updated = if let (Some(start), Some(end)) = (existing.find(START), existing.find(END)) {
        let end_idx = end + END.len();
        format!(
            "{}{}{}",
            &existing[..start],
            block,
            existing[end_idx..].trim_start_matches('\n')
        )
    } else if existing.trim().is_empty() {
        block
    } else {
        format!("{}\n\n{}", existing.trim_end(), block)
    };

    std::fs::write(&path, updated).map_err(|e| format!("write {}: {e}", path.display()))?;
    restrict_file_permissions(&path);
    Ok(())
}

pub(crate) fn sanitize_setup_text(input: &str, fallback: &str, max_chars: usize) -> String {
    sanitize_optional_setup_text(input, max_chars).unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn sanitize_optional_setup_text(input: &str, max_chars: usize) -> Option<String> {
    let value: String = input
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn setup_existing_first_run_profile(captain_dir: &Path) -> FirstRunProfile {
    let config = setup_read_config_value(captain_dir);
    let user_md = std::fs::read_to_string(captain_dir.join("USER.md")).unwrap_or_default();
    let from_user = |label: &str| setup_user_profile_value(&user_md, label);

    let assistant_name = setup_config_string(config.as_ref(), "assistant.display_name")
        .or_else(|| from_user("Assistant display name"))
        .unwrap_or_else(|| "Captain".to_string());
    let assistant_style = setup_config_string(config.as_ref(), "assistant.style")
        .or_else(|| from_user("Assistant style"))
        .unwrap_or_else(|| "balanced".to_string());
    let user_name = from_user("Preferred name")
        .filter(|value| value != "(not set)")
        .and_then(|value| sanitize_optional_setup_text(&value, 64));
    let user_language = setup_config_string(config.as_ref(), "language")
        .or_else(|| from_user("Preferred language"))
        .unwrap_or_else(detect_setup_language);
    let timezone = setup_config_string(config.as_ref(), "timezone")
        .or_else(|| from_user("Timezone"))
        .unwrap_or_else(detect_setup_timezone);
    let voice_preference = from_user("Voice preference").unwrap_or_else(|| "none".to_string());
    let notifications =
        from_user("Notification preference").unwrap_or_else(|| "important only".to_string());
    let privacy = from_user("Privacy boundaries")
        .unwrap_or_else(|| "ask before memorizing sensitive or private information".to_string());

    FirstRunProfile {
        assistant_name,
        assistant_style,
        user_name,
        user_language,
        timezone,
        voice_preference,
        notifications,
        privacy,
    }
}

fn setup_user_profile_value(contents: &str, label: &str) -> Option<String> {
    let prefix = format!("- {label}:");
    contents
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn detect_setup_language() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|lang| lang.split(['.', '_']).next().map(str::to_string))
        .filter(|lang| !lang.trim().is_empty())
        .unwrap_or_else(|| "en".to_string())
}

fn detect_setup_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim();
        if !tz.is_empty() {
            return tz.to_string();
        }
    }

    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let rendered = link.to_string_lossy();
        if let Some((_, tz)) = rendered.split_once("zoneinfo/") {
            if !tz.trim().is_empty() {
                return tz.to_string();
            }
        }
    }

    "UTC".to_string()
}
