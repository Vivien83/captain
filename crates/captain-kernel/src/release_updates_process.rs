//! Detached checksum-verified installer and crash/restart reconciliation.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use captain_types::release_update::{
    RuntimeUpdateAttemptResult, RuntimeUpdateAttemptStatus, RuntimeUpdateNoticeKind,
    RUNTIME_UPDATE_RESULT_FILENAME, RUNTIME_UPDATE_RESULT_SCHEMA_VERSION,
};
use captain_types::version::canonical_version;

use super::{
    bound, card_for_candidate, enqueue_card, expire_stale_update_attempt, now_unix_ms,
    remember_consumed_attempt, requeue_candidate_after_failure, suppress_actionable_outbox,
    CaptainKernel, RuntimeUpdateAttempt, RuntimeUpdateState, UPDATE_ATTEMPT_ID_ENV,
    UPDATE_ATTEMPT_STALE_MS, UPDATE_RESTART_GRACE_MS, UPDATE_RESULT_PATH_ENV,
};

pub(super) fn spawn_detached_update(
    kernel: &CaptainKernel,
    attempt: &RuntimeUpdateAttempt,
) -> Result<(), String> {
    let result_path = runtime_update_result_path(&kernel.config.home_dir);
    captain_types::durable_fs::create_dir_all(
        runtime_update_log_path(&kernel.config.home_dir, &attempt.attempt_id)
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    )
    .map_err(|error| format!("create update log directory: {error}"))?;
    captain_types::durable_fs::remove_file(&result_path)
        .map_err(|error| format!("clear previous update result: {error}"))?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&attempt.log_path)
        .map_err(|error| format!("open {}: {error}", attempt.log_path))?;
    let stderr = log
        .try_clone()
        .map_err(|error| format!("clone update log handle: {error}"))?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve current Captain executable: {error}"))?;
    let mut command = Command::new(executable);
    command
        .args([
            "update",
            "--yes",
            "--version",
            attempt.requested_version.as_str(),
        ])
        .env(UPDATE_ATTEMPT_ID_ENV, &attempt.attempt_id)
        .env(UPDATE_RESULT_PATH_ENV, &result_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map_err(|error| format!("spawn detached Captain updater: {error}"))?;
    Ok(())
}

pub(super) fn reconcile_update_attempt_result(kernel: &CaptainKernel) -> Result<(), String> {
    let path = runtime_update_result_path(&kernel.config.home_dir);
    let payload = match std::fs::read(&path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return reconcile_stale_update_attempt(kernel);
        }
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let result: RuntimeUpdateAttemptResult = match serde_json::from_slice(&payload) {
        Ok(result) => result,
        Err(error) => {
            let quarantined = quarantine_update_result(&kernel.config.home_dir, &path)?;
            return Err(format!(
                "parse {}: {error}; quarantined as {}",
                path.display(),
                quarantined.display()
            ));
        }
    };
    if result.schema_version != RUNTIME_UPDATE_RESULT_SCHEMA_VERSION {
        let quarantined = quarantine_update_result(&kernel.config.home_dir, &path)?;
        return Err(format!(
            "unsupported runtime update result schema {}; quarantined as {}",
            result.schema_version,
            quarantined.display()
        ));
    }
    let current = captain_types::version::captain_version();
    let now = now_unix_ms();
    let consumed = kernel
        .mutate_runtime_update_state(|state| apply_attempt_result(state, &result, &current, now))
        .map_err(|error| error.to_string())?;
    if consumed {
        captain_types::durable_fs::remove_file(&path)
            .map_err(|error| format!("remove consumed {}: {error}", path.display()))?;
    }
    Ok(())
}

fn reconcile_stale_update_attempt(kernel: &CaptainKernel) -> Result<(), String> {
    let now = now_unix_ms();
    let stale = kernel
        .load_runtime_update_state()
        .map_err(|error| error.to_string())?
        .and_then(|state| state.active_attempt)
        .is_some_and(|attempt| {
            now.saturating_sub(attempt.started_at_unix_ms) >= UPDATE_ATTEMPT_STALE_MS
        });
    if !stale {
        return Ok(());
    }
    kernel
        .mutate_runtime_update_state(|state| expire_stale_update_attempt(state, now))
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn apply_attempt_result(
    state: &mut RuntimeUpdateState,
    result: &RuntimeUpdateAttemptResult,
    current: &str,
    now: i64,
) -> bool {
    if state
        .consumed_attempt_ids
        .iter()
        .any(|id| id == &result.attempt_id)
    {
        return true;
    }
    let Some(attempt) = state
        .active_attempt
        .as_ref()
        .filter(|attempt| attempt.attempt_id == result.attempt_id)
        .cloned()
    else {
        state.last_error = Some(format!(
            "Ignored unknown runtime update result {}",
            result.attempt_id
        ));
        remember_consumed_attempt(state, &result.attempt_id);
        return true;
    };
    if result.status == RuntimeUpdateAttemptStatus::Succeeded
        && canonical_version(current) != canonical_version(&attempt.requested_version)
        && now.saturating_sub(attempt.started_at_unix_ms) < UPDATE_RESTART_GRACE_MS
    {
        return false;
    }

    state.active_attempt = None;
    remember_consumed_attempt(state, &result.attempt_id);
    if result.status == RuntimeUpdateAttemptStatus::Succeeded
        && canonical_version(current) == canonical_version(&attempt.requested_version)
    {
        if let Some(mut candidate) = state.pending.take() {
            suppress_actionable_outbox(state);
            candidate.current_version = current.to_string();
            let card = card_for_candidate(
                state,
                &candidate,
                RuntimeUpdateNoticeKind::Installed,
                Some(result.message.clone()),
            );
            enqueue_card(state, card, now);
        }
        state.last_error = None;
        return true;
    }

    let detail = if result.status == RuntimeUpdateAttemptStatus::Succeeded {
        format!(
            "Le binaire {} a été installé mais le runtime actif est encore {} après le délai de redémarrage.",
            attempt.requested_version, current
        )
    } else {
        result.message.clone()
    };
    state.last_error = Some(bound(&detail, 2_048));
    requeue_candidate_after_failure(state, &detail, now);
    true
}

pub(super) fn runtime_update_result_path(home: &Path) -> PathBuf {
    home.join(RUNTIME_UPDATE_RESULT_FILENAME)
}

pub(super) fn quarantine_update_result(home: &Path, source: &Path) -> Result<PathBuf, String> {
    let destination = home.join("logs").join(format!(
        "runtime-update-result.invalid.{}.json",
        now_unix_ms()
    ));
    captain_types::durable_fs::atomic_copy(source, &destination)
        .map_err(|error| format!("quarantine {}: {error}", source.display()))?;
    captain_types::durable_fs::remove_file(source)
        .map_err(|error| format!("remove quarantined {}: {error}", source.display()))?;
    Ok(destination)
}

pub(super) fn runtime_update_log_path(home: &Path, attempt_id: &str) -> PathBuf {
    home.join("logs").join(format!(
        "runtime-update-{}.log",
        attempt_id
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .take(64)
            .collect::<String>()
    ))
}
