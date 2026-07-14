use std::path::Path;

use super::read_identity_file;

pub(super) const GLOBAL_USER_PROFILE_START: &str = "<!-- captain:first-run-profile:start -->";
pub(super) const GLOBAL_USER_PROFILE_END: &str = "<!-- captain:first-run-profile:end -->";
pub(super) const FIRST_USE_ONBOARDING_STATE_FILE: &str = "onboarding.json";
pub(super) const FIRST_USE_ONBOARDING_QUESTIONS: &[(&str, &str, &str)] = &[
    (
        "preferred_name",
        "Comment dois-je t'appeler ?",
        "What should I call you?",
    ),
    (
        "language",
        "Dans quelle langue préfères-tu que je réponde ?",
        "Which language should I use for replies?",
    ),
    (
        "timezone",
        "Quel fuseau horaire dois-je utiliser ?",
        "Which timezone should I use?",
    ),
    (
        "answer_style",
        "Quel style de réponse préfères-tu ?",
        "Which answer style do you prefer?",
    ),
    (
        "voice_preference",
        "Pour l'audio, préfères-tu OpenAI Nova, ElevenLabs, ou pas de voix ?",
        "For audio, do you prefer OpenAI Nova, ElevenLabs, or no voice?",
    ),
    (
        "notifications",
        "Comment veux-tu que je gère les notifications ?",
        "How should I handle notifications?",
    ),
    (
        "privacy",
        "Y a-t-il des limites de confidentialité ou des sujets à ne jamais mémoriser ?",
        "Are there privacy boundaries or topics I should never memorize?",
    ),
];

pub(super) fn user_profile_has_product_content(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && trimmed != "# User"
            && trimmed != "# User profile"
            && !trimmed.starts_with("<!--")
            && !trimmed.ends_with("-->")
            && !trimmed.contains("Updated by the agent")
            && !trimmed.contains("first-run-profile")
    })
}

pub(super) fn read_global_user_profile(home_dir: &Path) -> Option<String> {
    read_identity_file(home_dir, "USER.md")
        .filter(|content| user_profile_has_product_content(content))
}

pub(super) fn first_use_locale(config_language: &str, message: &str) -> &'static str {
    let lang = config_language.trim().to_ascii_lowercase();
    if lang.starts_with("fr") {
        return "fr";
    }
    let msg = message.to_ascii_lowercase();
    if msg.contains("bonjour")
        || msg.contains("salut")
        || msg.contains("merci")
        || msg.contains("préfér")
        || msg.contains("repond")
        || msg.contains("répond")
    {
        "fr"
    } else {
        "en"
    }
}

pub(super) fn first_use_skip_requested(message: &str) -> bool {
    matches!(
        message.trim().to_ascii_lowercase().as_str(),
        "skip"
            | "passer"
            | "passe"
            | "plus tard"
            | "later"
            | "ignore"
            | "terminer"
            | "stop"
            | "/skip"
            | "/skip_onboarding"
    )
}

pub(super) fn first_use_trivial_greeting(message: &str) -> bool {
    matches!(
        message.trim().to_ascii_lowercase().as_str(),
        "" | "hey" | "hi" | "hello" | "bonjour" | "salut" | "yo" | "coucou"
    )
}

pub(super) fn first_use_clean_answer(message: &str) -> String {
    message
        .trim()
        .lines()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect::<String>()
        .trim()
        .to_string()
}

fn first_use_question(locale: &str, step: usize) -> Option<&'static str> {
    FIRST_USE_ONBOARDING_QUESTIONS
        .get(step)
        .map(|(_, fr, en)| if locale == "fr" { *fr } else { *en })
}

pub(super) fn first_use_intro(locale: &str, pending_request: Option<&str>) -> String {
    let pending = pending_request
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            if locale == "fr" {
                format!(
                    "\n\nJe garde ta première demande en attente : \"{}\".",
                    s.trim()
                )
            } else {
                format!(
                    "\n\nI will keep your first request pending: \"{}\".",
                    s.trim()
                )
            }
        })
        .unwrap_or_default();
    if locale == "fr" {
        format!(
            "Avant de continuer, je termine l'entretien de première utilisation. \
             Je te pose 7 questions courtes, une par message. Réponds `passer` \
             pour terminer sans personnalisation.{pending}\n\n1/7 — {}",
            first_use_question(locale, 0).unwrap_or("Comment dois-je t'appeler ?")
        )
    } else {
        format!(
            "Before we continue, I need to finish the first-use interview. \
             I will ask 7 short questions, one message at a time. Reply `skip` \
             to finish without personalization.{pending}\n\n1/7 - {}",
            first_use_question(locale, 0).unwrap_or("What should I call you?")
        )
    }
}

pub(super) fn first_use_next_prompt(locale: &str, next_step: usize) -> String {
    let total = FIRST_USE_ONBOARDING_QUESTIONS.len();
    let question = first_use_question(locale, next_step).unwrap_or("Preference?");
    if locale == "fr" {
        format!("{}/{} — {}", next_step + 1, total, question)
    } else {
        format!("{}/{} - {}", next_step + 1, total, question)
    }
}

pub(super) fn first_use_completed_response(
    locale: &str,
    skipped: bool,
    pending_request: Option<&str>,
) -> String {
    let pending = pending_request
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            if locale == "fr" {
                format!("\n\nTa première demande était : \"{}\". Tu peux me la renvoyer ou continuer directement.", s.trim())
            } else {
                format!("\n\nYour first request was: \"{}\". You can resend it or continue directly.", s.trim())
            }
        })
        .unwrap_or_default();
    if locale == "fr" {
        if skipped {
            format!(
                "C'est noté. J'ai terminé l'entretien sans personnalisation et je garde `~/.captain/USER.md` comme profil global.{pending}"
            )
        } else {
            format!(
                "Entretien terminé. J'ai enregistré ton profil global dans `~/.captain/USER.md` et aligné la config persistante.{pending}"
            )
        }
    } else if skipped {
        format!(
            "Done. I finished onboarding without personalization and kept `~/.captain/USER.md` as the global profile.{pending}"
        )
    } else {
        format!(
            "Onboarding complete. I saved your global profile in `~/.captain/USER.md` and aligned the persistent config.{pending}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_user_profile_content_ignores_placeholder() {
        let placeholder = "# User\n<!-- Updated by the agent as it learns about the user -->\n";
        assert!(!user_profile_has_product_content(placeholder));

        let profile = "# User profile\n- Preferred name: Alex\n";
        assert!(user_profile_has_product_content(profile));
    }

    #[test]
    fn first_use_onboarding_skip_words_are_detected() {
        assert!(first_use_skip_requested("passer"));
        assert!(first_use_skip_requested("/skip_onboarding"));
        assert!(first_use_skip_requested("skip"));
        assert!(!first_use_skip_requested("je prefere ElevenLabs"));
    }
}
