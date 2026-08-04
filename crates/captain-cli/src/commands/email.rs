use std::path::{Path, PathBuf};

use captain_extensions::gmail_oauth::{
    authorize_google_desktop, bundled_google_desktop_client, refresh_google_tokens,
    revoke_google_tokens, verify_google_tokens, GoogleDesktopClient,
};
use captain_kernel::gmail_persistence::{
    delete_gmail_account, new_gmail_token_vault_key, persist_gmail_grant,
};
use captain_memory::gmail_accounts::{preserved_sync_cursor, NewGmailAccount};
use captain_types::email::GmailAccountAlias;

use super::email_render::*;
use super::email_support::*;
use crate::{open_in_browser, ui, EmailCommands, EmailProviderArg, GmailAccessArg};

pub(crate) fn cmd_email(config: Option<PathBuf>, command: EmailCommands) {
    if let Err(error) = run_email(config.as_deref(), command) {
        ui::error(&error);
        std::process::exit(1);
    }
}

fn run_email(config: Option<&Path>, command: EmailCommands) -> Result<(), String> {
    match command {
        EmailCommands::Connect {
            provider,
            alias,
            access,
            client_json,
            login_hint,
            make_default,
            no_browser,
            callback_port,
            json,
        } => connect_gmail(
            config,
            provider,
            alias.as_deref(),
            access,
            client_json.as_deref(),
            login_hint.as_deref(),
            make_default,
            no_browser,
            callback_port,
            json,
        ),
        EmailCommands::Accounts { json } => list_accounts(config, json),
        EmailCommands::Status { alias, json } => show_status(config, alias.as_deref(), json),
        EmailCommands::Test { alias, json } => test_account(config, alias.as_deref(), json),
        EmailCommands::Default { alias } => set_default_account(config, &alias),
        EmailCommands::Disconnect {
            alias,
            revoke,
            yes,
            json,
        } => disconnect_account(config, &alias, revoke, yes, json),
        EmailCommands::Rules { command } => super::email_automation::manage_rules(config, command),
        EmailCommands::Deliveries { command } => {
            super::email_automation::manage_deliveries(config, command)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn connect_gmail(
    config: Option<&Path>,
    provider: EmailProviderArg,
    requested_alias: Option<&str>,
    access: GmailAccessArg,
    client_json: Option<&Path>,
    login_hint: Option<&str>,
    make_default: bool,
    no_browser: bool,
    callback_port: Option<u16>,
    json: bool,
) -> Result<(), String> {
    match provider {
        EmailProviderArg::Gmail => {}
    }
    if std::env::var_os("SSH_CONNECTION").is_some() && callback_port.is_none() {
        return Err(
            "Gmail OAuth over SSH requires --callback-port PORT and an SSH tunnel such as `ssh -L PORT:127.0.0.1:PORT host`."
                .to_string(),
        );
    }
    let parsed_alias = requested_alias
        .map(GmailAccountAlias::parse)
        .transpose()
        .map_err(|error| error.to_string())?;
    let profile = access.profile();

    let (state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let records = state
        .memory
        .gmail_accounts()
        .list()
        .map_err(|error| error.to_string())?;
    let explicit_client = client_json
        .map(|path| {
            let raw = read_google_client_json(path)?;
            GoogleDesktopClient::from_google_client_json(&raw).map_err(|error| error.to_string())
        })
        .transpose()?;
    let bundled_client = bundled_google_desktop_client().map_err(|error| error.to_string())?;
    let resolved_client = resolve_google_oauth_client(
        &records,
        parsed_alias.as_ref(),
        &state.vault,
        explicit_client,
        bundled_client,
    )?;
    drop(state);

    eprintln!("OAuth identity: {}.", resolved_client.source);
    eprintln!("Waiting for Google consent; no email is stored until authorization succeeds.");
    let runtime = oauth_runtime()?;
    let authorization = runtime
        .block_on(authorize_google_desktop(
            &resolved_client.client,
            profile,
            login_hint,
            callback_port,
            |url| {
                eprintln!("Authorize Gmail in your browser:\n{url}");
                if !no_browser {
                    if open_in_browser(url) {
                        eprintln!("Browser opened. Complete consent to continue.");
                    } else {
                        eprintln!("No browser opener was found; open the URL manually.");
                    }
                }
                Ok(())
            },
        ))
        .map_err(|error| error.to_string())?;

    let (mut state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let current = state
        .memory
        .gmail_accounts()
        .list()
        .map_err(|error| error.to_string())?;
    let alias = select_connected_alias(
        parsed_alias,
        &authorization.email_address,
        current.as_slice(),
    )?;
    let token_secret = authorization
        .tokens
        .to_secret_json()
        .map_err(|error| error.to_string())?;
    let account = NewGmailAccount {
        alias: alias.clone(),
        email_address: authorization.email_address,
        access_profile: profile,
        granted_scopes: authorization.tokens.granted_scopes().to_vec(),
        token_vault_key: new_gmail_token_vault_key(),
        client_vault_key: resolved_client.client_vault_key,
        history_id: authorization.history_id.clone(),
        make_default,
    };
    let outcome = persist_gmail_grant(
        &state.lock,
        &mut state.vault,
        state.memory.gmail_accounts(),
        account,
        token_secret,
        resolved_client.new_client_secret,
    )
    .map_err(|error| error.to_string())?;
    report_cleanup_warnings(&outcome.cleanup_warnings);
    state
        .memory
        .gmail_accounts()
        .record_sync_success(&alias, authorization.history_id.as_deref())
        .map_err(|error| error.to_string())?;
    let record = state
        .memory
        .gmail_accounts()
        .get(&alias)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Connected Gmail account disappeared after commit".to_string())?;
    let view = account_view(&mut state, record)?;
    print_account_result(&view, json, "Connected")
}

fn list_accounts(config: Option<&Path>, json: bool) -> Result<(), String> {
    let (mut state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let records = state
        .memory
        .gmail_accounts()
        .list()
        .map_err(|error| error.to_string())?;
    let views = records
        .into_iter()
        .map(|record| account_view(&mut state, record))
        .collect::<Result<Vec<_>, _>>()?;
    print_account_list(&views, json)
}

fn show_status(config: Option<&Path>, alias: Option<&str>, json: bool) -> Result<(), String> {
    let (mut state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let records = select_records(&state, alias, false)?;
    let views = records
        .into_iter()
        .map(|record| account_view(&mut state, record))
        .collect::<Result<Vec<_>, _>>()?;
    print_account_list(&views, json)
}

fn test_account(config: Option<&Path>, alias: Option<&str>, json: bool) -> Result<(), String> {
    let (state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let record = select_records(&state, alias, true)?
        .into_iter()
        .next()
        .ok_or_else(|| "No Gmail account is connected".to_string())?;
    let account_alias = record.summary.alias.clone();
    let client = load_client(&state.vault, &record).map_err(|error| {
        mark_reauth(&state, &account_alias, "credential_invalid");
        error
    })?;
    let tokens = load_tokens(&state.vault, &record).map_err(|error| {
        mark_reauth(&state, &account_alias, "credential_invalid");
        error
    })?;
    let profile = record.summary.access_profile;
    let expected_email = record.summary.email_address.clone();
    let client_key = record.client_vault_key.clone();
    let original_token_key = record.token_vault_key.clone();
    drop(state);

    let runtime = oauth_runtime()?;
    let live = runtime.block_on(async move {
        let refresh_due = tokens.needs_refresh(now_unix_seconds()?);
        let current = if refresh_due {
            refresh_google_tokens(&client, &tokens, profile)
                .await
                .map_err(|error| error.to_string())?
        } else {
            tokens
        };
        let identity = verify_google_tokens(&current, profile)
            .await
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((current, identity, refresh_due))
    });
    let (tokens, identity, refreshed) = match live {
        Ok(live) => live,
        Err(error) => {
            let (state, _) = EmailState::open(config)?;
            state
                .memory
                .gmail_accounts()
                .record_check_failure(&account_alias, "live_check_failed")
                .map_err(|store_error| store_error.to_string())?;
            return Err(format!("Gmail live check failed: {error}"));
        }
    };
    if !identity.email_address.eq_ignore_ascii_case(&expected_email) {
        let (state, _) = EmailState::open(config)?;
        mark_reauth(&state, &account_alias, "identity_mismatch");
        return Err(
            "Google returned a different account identity; reconnect this alias".to_string(),
        );
    }

    let (mut state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let latest = require_unchanged_account(
        &state,
        &account_alias,
        &original_token_key,
        &client_key,
        &expected_email,
        profile,
    )?;
    if refreshed {
        let account = NewGmailAccount {
            alias: account_alias.clone(),
            email_address: expected_email,
            access_profile: profile,
            granted_scopes: tokens.granted_scopes().to_vec(),
            token_vault_key: new_gmail_token_vault_key(),
            client_vault_key: client_key,
            history_id: preserved_sync_cursor(
                latest.summary.history_id.clone(),
                identity.history_id.clone(),
            ),
            make_default: latest.summary.is_default,
        };
        let secret = tokens.to_secret_json().map_err(|error| error.to_string())?;
        let outcome = persist_gmail_grant(
            &state.lock,
            &mut state.vault,
            state.memory.gmail_accounts(),
            account,
            secret,
            None,
        )
        .map_err(|error| error.to_string())?;
        report_cleanup_warnings(&outcome.cleanup_warnings);
    }
    state
        .memory
        .gmail_accounts()
        .record_check_success(&account_alias)
        .map_err(|error| error.to_string())?;
    let updated = state
        .memory
        .gmail_accounts()
        .get(&account_alias)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Gmail account disappeared after live check".to_string())?;
    let view = account_view(&mut state, updated)?;
    print_account_result(&view, json, "Verified")
}

fn set_default_account(config: Option<&Path>, alias: &str) -> Result<(), String> {
    let alias = GmailAccountAlias::parse(alias).map_err(|error| error.to_string())?;
    let (state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    state
        .memory
        .gmail_accounts()
        .set_default(&alias)
        .map_err(|error| error.to_string())?;
    ui::success(&format!("'{}' is now the default email account.", alias));
    Ok(())
}

fn disconnect_account(
    config: Option<&Path>,
    alias: &str,
    revoke: bool,
    yes: bool,
    json: bool,
) -> Result<(), String> {
    let alias = GmailAccountAlias::parse(alias).map_err(|error| error.to_string())?;
    let (mut state, warnings) = EmailState::open(config)?;
    report_cleanup_warnings(&warnings);
    let record = state
        .memory
        .gmail_accounts()
        .get(&alias)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Gmail account '{alias}' was not found"))?;
    if revoke {
        confirm_revocation(&record, yes)?;
        let tokens = load_tokens(&state.vault, &record)?;
        oauth_runtime()?
            .block_on(revoke_google_tokens(&tokens))
            .map_err(|error| format!("Google grant was not revoked: {error}"))?;
    }
    let outcome = delete_gmail_account(
        &state.lock,
        &mut state.vault,
        state.memory.gmail_accounts(),
        &alias,
    )
    .map_err(|error| error.to_string())?;
    report_cleanup_warnings(&outcome.cleanup_warnings);
    let view = DisconnectView {
        provider: "gmail",
        alias,
        disconnected: outcome.deleted.is_some(),
        grant_revoked: revoke,
    };
    print_disconnect_result(&view, json)
}
