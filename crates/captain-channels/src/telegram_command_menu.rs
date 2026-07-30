//! Native Telegram `/` command-menu synchronization.

use crate::channel_commands::{active_channel_commands, ChannelCommandSpec, CommandLanguage};
use serde::Serialize;
use std::time::Duration;

const COMMAND_MENU_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, Serialize)]
struct TelegramBotCommand {
    command: &'static str,
    description: &'static str,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TelegramCommandMenuSyncReport {
    pub(crate) command_count: usize,
    pub(crate) registered_locales: usize,
    pub(crate) failures: Vec<String>,
}

fn telegram_commands(language: CommandLanguage) -> Vec<TelegramBotCommand> {
    active_channel_commands()
        .map(|command: &ChannelCommandSpec| TelegramBotCommand {
            command: command.name,
            description: command.description(language),
        })
        .collect()
}

async fn register_command_locale(
    client: &reqwest::Client,
    api_base_url: &str,
    token: &str,
    language: CommandLanguage,
    language_code: Option<&str>,
) -> Result<(), String> {
    let mut body = serde_json::json!({
        "commands": telegram_commands(language),
    });
    if let Some(language_code) = language_code {
        body["language_code"] = serde_json::Value::String(language_code.to_string());
    }

    let url = format!("{api_base_url}/bot{token}/setMyCommands");
    let response = tokio::time::timeout(COMMAND_MENU_TIMEOUT, client.post(url).json(&body).send())
        .await
        .map_err(|_| "request timed out".to_string())?
        .map_err(|error| format!("request failed: {}", error.without_url()))?;
    let status = response.status();
    let response_body = response
        .text()
        .await
        .map_err(|error| format!("response unreadable: {}", error.without_url()))?;
    let parsed = serde_json::from_str::<serde_json::Value>(&response_body).unwrap_or_default();
    if status.is_success()
        && parsed["ok"].as_bool() == Some(true)
        && parsed["result"].as_bool() == Some(true)
    {
        return Ok(());
    }

    let description = parsed["description"]
        .as_str()
        .unwrap_or("Telegram rejected the command catalogue");
    Err(format!("HTTP {status}: {description}"))
}

pub(crate) async fn sync_telegram_command_menu(
    client: &reqwest::Client,
    api_base_url: &str,
    token: &str,
) -> TelegramCommandMenuSyncReport {
    let default_registration =
        register_command_locale(client, api_base_url, token, CommandLanguage::English, None);
    let french_registration = register_command_locale(
        client,
        api_base_url,
        token,
        CommandLanguage::French,
        Some("fr"),
    );
    let (default_result, french_result) = tokio::join!(default_registration, french_registration);

    let mut report = TelegramCommandMenuSyncReport {
        command_count: active_channel_commands().count(),
        ..TelegramCommandMenuSyncReport::default()
    };
    for (locale, result) in [("default", default_result), ("fr", french_result)] {
        match result {
            Ok(()) => report.registered_locales += 1,
            Err(error) => report.failures.push(format!("{locale}: {error}")),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn localized_menus_share_names_and_translate_descriptions() {
        let english = telegram_commands(CommandLanguage::English);
        let french = telegram_commands(CommandLanguage::French);

        assert_eq!(english.len(), active_channel_commands().count());
        assert_eq!(french.len(), english.len());
        assert_eq!(english[0].command, french[0].command);
        assert_ne!(english[0].description, french[0].description);
        assert_eq!(english.last().unwrap().command, "gethome");
    }

    #[tokio::test]
    async fn sync_posts_default_and_french_command_catalogues() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/setMyCommands"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": true})),
            )
            .expect(2)
            .mount(&server)
            .await;

        let report =
            sync_telegram_command_menu(&reqwest::Client::new(), &server.uri(), "123:ABC").await;

        assert_eq!(
            report,
            TelegramCommandMenuSyncReport {
                command_count: 47,
                registered_locales: 2,
                failures: Vec::new(),
            }
        );
        let requests = server.received_requests().await.expect("requests");
        let bodies = requests
            .iter()
            .map(|request| {
                serde_json::from_slice::<serde_json::Value>(&request.body).expect("JSON body")
            })
            .collect::<Vec<_>>();
        assert!(bodies
            .iter()
            .any(|body| body.get("language_code").is_none()));
        assert!(bodies
            .iter()
            .any(|body| body["language_code"].as_str() == Some("fr")));
        assert!(bodies.iter().all(|body| {
            body["commands"].as_array().map(Vec::len) == Some(active_channel_commands().count())
        }));
    }

    #[tokio::test]
    async fn sync_reports_rejections_without_returning_a_startup_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/setMyCommands"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "ok": false,
                "description": "command menu unavailable"
            })))
            .expect(2)
            .mount(&server)
            .await;

        let report =
            sync_telegram_command_menu(&reqwest::Client::new(), &server.uri(), "123:ABC").await;

        assert_eq!(report.command_count, 47);
        assert_eq!(report.registered_locales, 0);
        assert_eq!(report.failures.len(), 2);
        assert!(report
            .failures
            .iter()
            .all(|failure| failure.contains("command menu unavailable")));
    }
}
