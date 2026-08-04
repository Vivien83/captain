use captain_types::email::{GmailAccountAlias, GmailAccountSummary};
use serde::Serialize;

use crate::ui;

#[derive(Serialize)]
pub(super) struct EmailAccountView {
    pub(super) provider: &'static str,
    #[serde(flatten)]
    pub(super) summary: GmailAccountSummary,
    pub(super) credential_status: &'static str,
    pub(super) token_expires_at: Option<i64>,
    pub(super) token_refresh_due: bool,
}

#[derive(Serialize)]
pub(super) struct DisconnectView {
    pub(super) provider: &'static str,
    pub(super) alias: GmailAccountAlias,
    pub(super) disconnected: bool,
    pub(super) grant_revoked: bool,
}

pub(super) fn print_account_result(
    view: &EmailAccountView,
    json: bool,
    action: &str,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(view).map_err(|error| error.to_string())?
        );
    } else {
        ui::success(&format!(
            "{action} Gmail account '{}' ({}, access: {}).",
            view.summary.alias, view.summary.email_address, view.summary.access_profile
        ));
    }
    Ok(())
}

pub(super) fn print_account_list(views: &[EmailAccountView], json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(views).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if views.is_empty() {
        println!("No email accounts connected. Run: captain email connect");
        return Ok(());
    }
    println!(
        "{:<18} {:<34} {:<11} {:<16} CREDENTIALS",
        "ALIAS", "ADDRESS", "ACCESS", "STATUS"
    );
    for view in views {
        let marker = if view.summary.is_default { "*" } else { " " };
        println!(
            "{marker}{:<17} {:<34} {:<11} {:<16} {}",
            view.summary.alias,
            view.summary.email_address,
            view.summary.access_profile,
            view.summary.status,
            view.credential_status
        );
    }
    Ok(())
}

pub(super) fn print_disconnect_result(view: &DisconnectView, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(view).map_err(|error| error.to_string())?
        );
    } else {
        ui::success(if view.grant_revoked {
            "Gmail account disconnected and Google grant revoked."
        } else {
            "Gmail account disconnected locally."
        });
    }
    Ok(())
}

pub(super) fn report_cleanup_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("Warning: Gmail credential cleanup pending ({warning}).");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::email::{GmailAccessProfile, GmailAccountStatus};

    #[test]
    fn public_account_json_contains_no_vault_reference_or_token() {
        let now = chrono::Utc::now();
        let view = EmailAccountView {
            provider: "gmail",
            summary: GmailAccountSummary {
                alias: GmailAccountAlias::parse("personal").unwrap(),
                email_address: "person@gmail.com".to_string(),
                access_profile: GmailAccessProfile::Assistant,
                granted_scopes: GmailAccessProfile::Assistant
                    .required_scopes()
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect(),
                history_id: Some("123".to_string()),
                status: GmailAccountStatus::Ready,
                enabled: true,
                is_default: true,
                last_sync_at: None,
                last_error_code: None,
                created_at: now,
                updated_at: now,
            },
            credential_status: "ready",
            token_expires_at: Some(123),
            token_refresh_due: false,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("CAPTAIN_GMAIL"));
        assert!(!json.contains("token_vault_key"));
        assert!(!json.contains("refresh_token"));
        assert!(json.contains("person@gmail.com"));
    }
}
