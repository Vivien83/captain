//! Telegram Rich rendering for Captain runtime update decisions.

use captain_types::release_update::{
    RuntimeUpdateCard, RuntimeUpdateInstallMode, RuntimeUpdateNoticeKind,
    RuntimeUpdateOperatorResolution, RuntimeUpdateResolutionStatus,
};

pub fn build_runtime_update_keyboard(
    card: &RuntimeUpdateCard,
    language: &str,
) -> serde_json::Value {
    if card.notice == RuntimeUpdateNoticeKind::Installed {
        return serde_json::json!({"inline_keyboard": []});
    }
    let french = language.to_ascii_lowercase().starts_with("fr");
    let install_label = match (french, card.install_mode) {
        (true, RuntimeUpdateInstallMode::SelfUpdate) => "Mettre à jour",
        (true, RuntimeUpdateInstallMode::Container) => "Procédure Docker",
        (true, RuntimeUpdateInstallMode::Manual) => "Procédure manuelle",
        (false, RuntimeUpdateInstallMode::SelfUpdate) => "Update now",
        (false, RuntimeUpdateInstallMode::Container) => "Docker procedure",
        (false, RuntimeUpdateInstallMode::Manual) => "Manual procedure",
    };
    let defer_label = if french {
        "Reporter 24 h"
    } else {
        "Remind me in 24h"
    };
    let refuse_label = if french {
        "Refuser cette version"
    } else {
        "Decline this version"
    };
    serde_json::json!({
        "inline_keyboard": [
            [{
                "text": install_label,
                "callback_data": format!(
                    "runtime_update:install:{}:{}",
                    card.token, card.decision_version
                )
            }],
            [
                {
                    "text": defer_label,
                    "callback_data": format!(
                        "runtime_update:defer:{}:{}",
                        card.token, card.decision_version
                    )
                },
                {
                    "text": refuse_label,
                    "callback_data": format!(
                        "runtime_update:refuse:{}:{}",
                        card.token, card.decision_version
                    )
                }
            ]
        ]
    })
}

pub fn format_runtime_update_card(card: &RuntimeUpdateCard, language: &str) -> String {
    if language.to_ascii_lowercase().starts_with("fr") {
        format_runtime_update_card_fr(card)
    } else {
        format_runtime_update_card_en(card)
    }
}

fn format_runtime_update_card_fr(card: &RuntimeUpdateCard) -> String {
    let title = match card.notice {
        RuntimeUpdateNoticeKind::Available => "Mise à jour Captain disponible",
        RuntimeUpdateNoticeKind::Reminder => "Rappel de mise à jour Captain",
        RuntimeUpdateNoticeKind::InstallFailed => "Mise à jour Captain interrompue",
        RuntimeUpdateNoticeKind::Installed => "Captain a été mis à jour",
    };
    let channel = if card.prerelease {
        "Préversion publique"
    } else {
        "Stable"
    };
    let mut text = format!(
        "## {title}\n\n| | Version |\n|---|---|\n| Installée | `{}` |\n| Disponible | `{}` |\n\n**Canal :** {channel}  \n**Vérifiée :** {}  \n**Prochain contrôle :** {}",
        card.current_version, card.available_version, card.checked_at, card.next_check_at
    );
    if let Some(detail) = card.detail.as_deref().filter(|detail| !detail.is_empty()) {
        text.push_str(&format!("\n\n> {detail}"));
    }
    if card.notice != RuntimeUpdateNoticeKind::Installed {
        match card.install_mode {
            RuntimeUpdateInstallMode::Container => text.push_str(
                "\n\nCette instance tourne dans un conteneur. Le bouton fournira la procédure orchestrateur ; Captain ne montera jamais le socket Docker pour se remplacer lui-même.",
            ),
            RuntimeUpdateInstallMode::Manual => text.push_str(
                "\n\nL’auto-mise à jour n’est pas disponible sur cette plateforme. Le bouton fournira la procédure manuelle sans modifier le système.",
            ),
            RuntimeUpdateInstallMode::SelfUpdate => text.push_str(
                "\n\nAucune installation automatique : une décision explicite est requise. Le bundle et son SHA-256 seront vérifiés avant le remplacement atomique.",
            ),
        }
    }
    text.push_str(&format!("\n\n[Consulter la release]({})", card.release_url));
    text
}

fn format_runtime_update_card_en(card: &RuntimeUpdateCard) -> String {
    let title = match card.notice {
        RuntimeUpdateNoticeKind::Available => "Captain update available",
        RuntimeUpdateNoticeKind::Reminder => "Captain update reminder",
        RuntimeUpdateNoticeKind::InstallFailed => "Captain update interrupted",
        RuntimeUpdateNoticeKind::Installed => "Captain was updated",
    };
    let channel = if card.prerelease {
        "Public prerelease"
    } else {
        "Stable"
    };
    let mut text = format!(
        "## {title}\n\n| | Version |\n|---|---|\n| Installed | `{}` |\n| Available | `{}` |\n\n**Channel:** {channel}  \n**Checked:** {}  \n**Next check:** {}",
        card.current_version, card.available_version, card.checked_at, card.next_check_at
    );
    if let Some(detail) = card.detail.as_deref().filter(|detail| !detail.is_empty()) {
        text.push_str(&format!("\n\n> {detail}"));
    }
    if card.notice != RuntimeUpdateNoticeKind::Installed {
        match card.install_mode {
            RuntimeUpdateInstallMode::Container => text.push_str(
                "\n\nThis instance runs in a container. The button provides the orchestrator procedure; Captain never mounts the Docker socket to replace itself.",
            ),
            RuntimeUpdateInstallMode::Manual => text.push_str(
                "\n\nSelf-update is unavailable on this platform. The button provides the manual procedure without changing the system.",
            ),
            RuntimeUpdateInstallMode::SelfUpdate => text.push_str(
                "\n\nNothing is installed automatically. The bundle and SHA-256 are verified before the atomic replacement.",
            ),
        }
    }
    text.push_str(&format!("\n\n[Open release]({})", card.release_url));
    text
}

pub fn format_runtime_update_resolution(
    resolution: &RuntimeUpdateOperatorResolution,
    language: &str,
) -> String {
    let french = language.to_ascii_lowercase().starts_with("fr");
    match resolution.status {
        RuntimeUpdateResolutionStatus::InstallStarted => {
            let log = resolution
                .log_path
                .as_deref()
                .unwrap_or("Captain update log");
            if french {
                format!(
                    "## Mise à jour lancée\n\n`{}` → `{}`\n\nLe téléchargement et la vérification SHA-256 tournent dans un processus détaché. Captain redémarrera sur le nouveau binaire.\n\n**Journal :** `{log}`",
                    resolution.current_version, resolution.available_version
                )
            } else {
                format!(
                    "## Update started\n\n`{}` → `{}`\n\nDownload and SHA-256 verification run in a detached process. Captain will restart on the new binary.\n\n**Log:** `{log}`",
                    resolution.current_version, resolution.available_version
                )
            }
        }
        RuntimeUpdateResolutionStatus::Deferred => {
            let next = resolution.next_prompt_at.as_deref().unwrap_or("24 h");
            if french {
                format!(
                    "## Mise à jour reportée\n\nCaptain reproposera `{}` le **{next}**.",
                    resolution.available_version
                )
            } else {
                format!(
                    "## Update deferred\n\nCaptain will offer `{}` again on **{next}**.",
                    resolution.available_version
                )
            }
        }
        RuntimeUpdateResolutionStatus::Refused => {
            if french {
                format!("## Version refusée\n\n`{}` ne sera plus proposée. Une version ultérieure déclenchera une nouvelle notification.", resolution.available_version)
            } else {
                format!("## Version declined\n\n`{}` will not be offered again. A later version will trigger a new notification.", resolution.available_version)
            }
        }
        RuntimeUpdateResolutionStatus::ContainerManual => {
            let next = resolution.next_prompt_at.as_deref().unwrap_or("24 h");
            if french {
                format!(
                    "## Mise à jour Docker à appliquer sur l’hôte\n\nVersion cible : `{}`\n\n```shell\ndocker compose pull\ndocker compose up -d\n```\n\nCaptain n’accède pas au socket Docker de l’hôte et vérifiera de nouveau le **{next}**.",
                    resolution.available_version,
                )
            } else {
                format!(
                    "## Apply the Docker update on the host\n\nTarget version: `{}`\n\n```shell\ndocker compose pull\ndocker compose up -d\n```\n\nCaptain does not access the host Docker socket and will check again on **{next}**.",
                    resolution.available_version,
                )
            }
        }
        RuntimeUpdateResolutionStatus::PlatformManual => {
            let next = resolution.next_prompt_at.as_deref().unwrap_or("24 h");
            if french {
                format!(
                    "## Mise à jour manuelle requise\n\nVersion cible : `{}`\n\nOuvre la release liée dans la carte et utilise l’installeur de ta plateforme. Captain vérifiera de nouveau le **{next}**.",
                    resolution.available_version,
                )
            } else {
                format!(
                    "## Manual update required\n\nTarget version: `{}`\n\nOpen the release linked in the card and use your platform installer. Captain will check again on **{next}**.",
                    resolution.available_version,
                )
            }
        }
    }
}

pub fn format_runtime_update_error(reason: &str, language: &str) -> String {
    if language.to_ascii_lowercase().starts_with("fr") {
        format!("## Décision non appliquée\n\n{reason}")
    } else {
        format!("## Decision not applied\n\n{reason}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(
        notice: RuntimeUpdateNoticeKind,
        install_mode: RuntimeUpdateInstallMode,
    ) -> RuntimeUpdateCard {
        RuntimeUpdateCard {
            notice,
            token: "00000000000000000000".to_string(),
            decision_version: 4,
            current_version: "0.1.0-alpha.8".to_string(),
            available_version: "0.1.0-alpha.9".to_string(),
            release_url: "https://example.test/v0.1.0-alpha.9".to_string(),
            published_at: Some("2026-07-20T08:00:00Z".to_string()),
            prerelease: true,
            install_mode,
            checked_at: "2026-07-20T08:05:00Z".to_string(),
            next_check_at: "2026-07-20T20:05:00Z".to_string(),
            detail: None,
        }
    }

    #[test]
    fn available_card_is_rich_explicit_and_has_three_versioned_actions() {
        let card = card(
            RuntimeUpdateNoticeKind::Available,
            RuntimeUpdateInstallMode::SelfUpdate,
        );
        let text = format_runtime_update_card(&card, "fr-FR");
        let keyboard = build_runtime_update_keyboard(&card, "fr-FR");
        assert!(text.contains("| Installée | `0.1.0-alpha.8` |"));
        assert!(text.contains("SHA-256"));
        let buttons = keyboard["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0]["text"], "Mettre à jour");
        assert_eq!(
            buttons[0]["callback_data"],
            "runtime_update:install:00000000000000000000:4"
        );
        assert!(buttons
            .iter()
            .all(|button| { button["callback_data"].as_str().unwrap().len() <= 64 }));
    }

    #[test]
    fn container_card_is_honest_and_installed_card_has_no_buttons() {
        let container = card(
            RuntimeUpdateNoticeKind::Available,
            RuntimeUpdateInstallMode::Container,
        );
        assert!(format_runtime_update_card(&container, "fr").contains("conteneur"));
        assert_eq!(
            build_runtime_update_keyboard(&container, "fr")["inline_keyboard"][0][0]["text"],
            "Procédure Docker"
        );
        let installed = card(
            RuntimeUpdateNoticeKind::Installed,
            RuntimeUpdateInstallMode::SelfUpdate,
        );
        assert!(
            build_runtime_update_keyboard(&installed, "fr")["inline_keyboard"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
