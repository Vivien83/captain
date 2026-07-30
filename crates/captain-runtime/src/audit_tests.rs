use super::*;
use captain_memory::migration::run_migrations;

fn migrated_db() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    Arc::new(Mutex::new(conn))
}

#[test]
fn audit_chain_integrity_and_tip_advance() {
    let log = AuditLog::new();
    let genesis_tip = log.tip_hash();
    let h1 = log
        .record("agent-1", AuditAction::ToolInvoke, "read_file", "ok")
        .unwrap();
    let h2 = log
        .record("agent-1", AuditAction::ShellExec, "ls -la", "ok")
        .unwrap();

    assert_eq!(log.len(), 2);
    assert_eq!(log.tip_hash(), h2);
    assert_ne!(h1, h2);
    assert_ne!(genesis_tip, h1);
    assert!(log.verify_integrity().is_ok());
    let entries = log.recent(2);
    assert_eq!(entries[0].prev_hash, GENESIS_HASH);
    assert_eq!(entries[1].prev_hash, entries[0].hash);
    assert_eq!(entries[0].hash_version, CURRENT_HASH_VERSION);
}

#[test]
fn length_prefixes_make_field_boundaries_injective() {
    let left = build_entry(
        0,
        0,
        "a".to_string(),
        "bc".to_string(),
        AuditAction::ToolInvoke,
        "detail".to_string(),
        "ok".to_string(),
        GENESIS_HASH.to_string(),
    )
    .unwrap();
    let right = build_entry(
        0,
        0,
        "ab".to_string(),
        "c".to_string(),
        AuditAction::ToolInvoke,
        "detail".to_string(),
        "ok".to_string(),
        GENESIS_HASH.to_string(),
    )
    .unwrap();

    assert_ne!(left.hash, right.hash);
}

#[test]
fn persistence_error_is_returned_without_advancing_memory() {
    let db = migrated_db();
    let log = AuditLog::with_db(Arc::clone(&db)).unwrap();
    let initial_tip = log.tip_hash();
    db.lock()
        .unwrap()
        .execute("DROP TABLE audit_entries", [])
        .unwrap();

    let error = log
        .record("agent-1", AuditAction::AgentSpawn, "spawn", "ok")
        .unwrap_err();

    assert!(matches!(error, AuditError::Database(_)));
    assert_eq!(log.len(), 0);
    assert_eq!(log.tip_hash(), initial_tip);
    let status = log.integrity_status();
    assert!(!status.valid);
    assert!(status.last_error.is_some());
}

#[test]
fn unknown_action_round_trips_without_becoming_tool_invoke() {
    let db = migrated_db();
    let mut entry = AuditEntry {
        seq: 0,
        epoch: 0,
        hash_version: LEGACY_HASH_VERSION,
        timestamp: "2026-07-29T00:00:00Z".to_string(),
        agent_id: "future-agent".to_string(),
        action: AuditAction::Unknown("FutureAction".to_string()),
        detail: "future detail".to_string(),
        outcome: "ok".to_string(),
        prev_hash: GENESIS_HASH.to_string(),
        hash: String::new(),
    };
    entry.hash = compute_entry_hash(&entry).unwrap();
    insert_entry(&db.lock().unwrap(), &entry).unwrap();

    let log = AuditLog::with_db(db).unwrap();
    let loaded = log.recent(1).pop().unwrap();
    assert_eq!(
        loaded.action,
        AuditAction::Unknown("FutureAction".to_string())
    );
    assert_eq!(loaded.action.to_string(), "FutureAction");
    assert!(log.verify_integrity().is_ok());
}

#[test]
fn tampered_epoch_is_sealed_once_and_recovery_epoch_remains_writable() {
    let db = migrated_db();
    let original_tip;
    {
        let log = AuditLog::with_db(Arc::clone(&db)).unwrap();
        log.record("agent-1", AuditAction::AgentSpawn, "original", "ok")
            .unwrap();
        log.record("agent-1", AuditAction::ShellExec, "ls", "ok")
            .unwrap();
        original_tip = log.tip_hash();
    }

    db.lock()
        .unwrap()
        .execute(
            "UPDATE audit_entries SET detail = 'tampered' WHERE seq = 0",
            [],
        )
        .unwrap();

    let recovered = AuditLog::with_db(Arc::clone(&db)).unwrap();
    let status = recovered.integrity_status();
    assert!(!status.valid);
    assert!(status.active_epoch_valid);
    assert_eq!(status.active_epoch, 1);
    assert_eq!(status.invalid_epochs, vec![0]);
    assert!(recovered.verify_integrity().is_err());

    let recovery = recovered.recent(1).pop().unwrap();
    assert_eq!(recovery.action, AuditAction::ChainRecovery);
    assert_eq!(recovery.epoch, 1);
    assert_eq!(recovery.prev_hash, original_tip);
    let recovery_detail: serde_json::Value = serde_json::from_str(&recovery.detail).unwrap();
    assert_eq!(recovery_detail["previous_epoch"], 0);
    assert_eq!(recovery_detail["previous_terminal_hash"], original_tip);

    recovered
        .record(
            "agent-2",
            AuditAction::ToolInvoke,
            "continues in recovery epoch",
            "ok",
        )
        .unwrap();
    assert_eq!(recovered.recent(1)[0].epoch, 1);
    let count_after_recovery = recovered.len();
    drop(recovered);

    let reloaded = AuditLog::with_db(db).unwrap();
    assert_eq!(reloaded.integrity_status().active_epoch, 1);
    assert_eq!(reloaded.len(), count_after_recovery);
}

#[test]
fn tampered_epoch_assignment_is_detected_without_recovery_collision() {
    let db = migrated_db();
    {
        let log = AuditLog::with_db(Arc::clone(&db)).unwrap();
        log.record("agent-1", AuditAction::ConfigChange, "original", "ok")
            .unwrap();
    }
    db.lock()
        .unwrap()
        .execute("UPDATE audit_entries SET epoch = 7 WHERE seq = 0", [])
        .unwrap();

    let recovered = AuditLog::with_db(Arc::clone(&db)).unwrap();
    let status = recovered.integrity_status();
    assert!(!status.valid);
    assert!(status.active_epoch_valid);
    assert_eq!(status.active_epoch, 8);
    assert_eq!(status.invalid_epochs, vec![0]);
    assert!(status
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("belongs to epoch 7")));
    let entries = recovered.recent(2);
    assert_eq!(entries[0].epoch, 7);
    assert_eq!(entries[1].epoch, 8);
    assert_eq!(entries[1].action, AuditAction::ChainRecovery);
    drop(recovered);

    let reloaded = AuditLog::with_db(db).unwrap();
    assert_eq!(reloaded.integrity_status().active_epoch, 8);
    assert_eq!(reloaded.len(), 2);
}

#[test]
fn persistence_survives_restart_with_mixed_hash_versions() {
    let db = migrated_db();
    let log = AuditLog::with_db(Arc::clone(&db)).unwrap();
    log.record("agent-1", AuditAction::AgentSpawn, "spawn test", "ok")
        .unwrap();
    log.record("agent-1", AuditAction::ShellExec, "ls", "ok")
        .unwrap();
    drop(log);

    let reloaded = AuditLog::with_db(db).unwrap();
    assert_eq!(reloaded.len(), 2);
    assert!(reloaded.verify_integrity().is_ok());
    assert!(reloaded
        .recent(2)
        .iter()
        .all(|entry| entry.hash_version == CURRENT_HASH_VERSION));
}
