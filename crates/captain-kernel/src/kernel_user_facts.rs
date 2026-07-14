use super::CaptainKernel;
use std::sync::Arc;
use tracing::info;

impl CaptainKernel {
    /// Phase N.5 — bootstrap factual user info into MemPalace on first boot
    /// when the table is empty. Source = config + std::env (no invented
    /// data). Inserted as `pending` rows so the resync worker syncs them
    /// to MemPalace on its next tick. Best-effort: any failure is logged
    /// but never blocks boot.
    pub(crate) fn bootstrap_user_facts_if_empty(self: &Arc<Self>) {
        let conn = self.memory.usage_conn();

        let is_empty = match conn.lock() {
            Ok(guard) => guard
                .query_row("SELECT count(*) FROM memory_writes", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map(|n| n == 0)
                .unwrap_or(false),
            Err(e) => {
                tracing::warn!(error = %e, "bootstrap_user_facts: cannot lock memory_writes");
                return;
            }
        };
        if !is_empty {
            return;
        }

        let mut facts: Vec<(&'static str, String)> = vec![
            ("os", std::env::consts::OS.to_string()),
            ("arch", std::env::consts::ARCH.to_string()),
            ("timezone", self.config.timezone.clone()),
            ("language", self.config.language.clone()),
            (
                "default_provider",
                self.config.default_model.provider.clone(),
            ),
            ("default_model", self.config.default_model.model.clone()),
            ("captain_version", captain_types::version::captain_version()),
            ("home_dir", self.config.home_dir.display().to_string()),
        ];
        if let Some(tg) = self.config.channels.telegram.as_ref() {
            if let Some(chat_id) = tg.default_chat_id.as_ref() {
                if !chat_id.is_empty() {
                    facts.push(("telegram_chat_id", chat_id.clone()));
                }
            }
        }

        let inserted = match conn.lock() {
            Ok(guard) => {
                let mut count = 0usize;
                for (predicate, object) in &facts {
                    if object.is_empty() {
                        continue;
                    }
                    let record = captain_memory::memory_writer::NewMemoryWrite {
                        subject: "user".into(),
                        predicate: (*predicate).into(),
                        object: object.clone(),
                        wing: Some("system".into()),
                        room: Some("bootstrap".into()),
                        source: "bootstrap.config".into(),
                    };
                    match captain_memory::memory_writer::append(&guard, record) {
                        Ok(_) => count += 1,
                        Err(e) => tracing::warn!(
                            error = %e,
                            predicate = %predicate,
                            "bootstrap_user_facts: insert failed"
                        ),
                    }
                }
                count
            }
            Err(e) => {
                tracing::warn!(error = %e, "bootstrap_user_facts: cannot lock for insert");
                return;
            }
        };

        if inserted > 0 {
            info!(
                facts = inserted,
                "Bootstrap user facts inserted into memory_writes"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::memory_writer::{list_recent, SyncStatus};
    use captain_types::config::KernelConfig;

    #[test]
    fn bootstrap_user_facts_inserts_pending_config_facts_once() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-bootstrap-user-facts");
        let mut config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        config.language = "fr".to_string();
        config.timezone = "Europe/Paris".to_string();
        config.default_model.provider = "codex".to_string();
        config.default_model.model = "gpt-5.5".to_string();
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).expect("kernel boot"));

        kernel.bootstrap_user_facts_if_empty();
        let rows = {
            let conn = kernel.memory.usage_conn();
            let guard = conn.lock().expect("usage conn lock");
            list_recent(&guard, Some("bootstrap.config"), 20).expect("list bootstrap rows")
        };

        assert!(rows.len() >= 8, "expected bootstrap facts, got {rows:?}");
        assert!(rows.iter().all(|row| row.subject == "user"));
        assert!(rows
            .iter()
            .all(|row| row.sync_status == SyncStatus::Pending));
        assert!(rows
            .iter()
            .any(|row| row.predicate == "language" && row.object == "fr"));
        assert!(rows
            .iter()
            .any(|row| row.predicate == "timezone" && row.object == "Europe/Paris"));
        assert!(rows
            .iter()
            .any(|row| row.predicate == "default_provider" && row.object == "codex"));
        assert!(
            rows.iter()
                .any(|row| row.predicate == "home_dir"
                    && row.object == home_dir.display().to_string())
        );

        kernel.bootstrap_user_facts_if_empty();
        let after_second_call = {
            let conn = kernel.memory.usage_conn();
            let guard = conn.lock().expect("usage conn lock");
            list_recent(&guard, Some("bootstrap.config"), 40).expect("list bootstrap rows")
        };
        assert_eq!(
            after_second_call.len(),
            rows.len(),
            "bootstrap must not duplicate facts once memory_writes is non-empty"
        );
        kernel.shutdown();
    }
}
