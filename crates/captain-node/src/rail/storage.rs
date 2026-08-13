use super::*;
use crate::pairing::{
    NodeStateBinding, NODE_RAIL_SHM_FILE, NODE_RAIL_STATE_FILE, NODE_RAIL_WAL_FILE,
};
use captain_types::durable_fs;
use captain_wire::{DeviceRole, ProtocolVersion, HUB_NODE_PROTOCOL_VERSION};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

mod approvals;
mod cancellation;
mod execution;
mod execution_claims;
mod runs;
pub(super) use approvals::{apply_run_approval_decision, approved_action_digest};
pub(super) use cancellation::apply_cancel_run;
pub(super) use execution::{cancellation_requested, claim_run, complete_run};
pub(super) use runs::{
    active_run_ids, apply_run_offer, claimable_runs, get_run, reject_run_before_effect,
};

const EXPECTED_TABLES: [&str; 6] = [
    "node_rail_inbox",
    "node_rail_meta",
    "node_rail_outbox",
    "node_run_approvals",
    "node_run_claims",
    "node_runs",
];

#[derive(Debug)]
struct RailMeta {
    hub_sha256: String,
    device_id: String,
    protocol_version: ProtocolVersion,
    connection_id: String,
    last_node_sequence: u64,
    acknowledged_node_sequence: u64,
    last_hub_sequence: u64,
    confirmed_hub_ack_sequence: u64,
    pruned_hub_sequence: u64,
}

pub(super) fn open_database(binding: &NodeStateBinding) -> Result<Connection, NodeRailError> {
    let root =
        fs::canonicalize(binding.root.path()).map_err(|_| NodeRailError::StateUnavailable)?;
    let database_path = root.join(NODE_RAIL_STATE_FILE);
    let existed = reject_unsafe_state_file(&database_path)?;
    for sidecar in [NODE_RAIL_WAL_FILE, NODE_RAIL_SHM_FILE] {
        reject_unsafe_state_file(&root.join(sidecar))?;
    }
    if !existed
        && !durable_fs::create_new(&database_path, &[])
            .map_err(|_| NodeRailError::StateUnavailable)?
    {
        return Err(NodeRailError::StateUnavailable);
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let mut connection = Connection::open_with_flags(&database_path, flags)
        .map_err(|_| NodeRailError::StateUnavailable)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(NodeRailError::from)?;

    if existed {
        quick_check(&connection)?;
        let version = user_version(&connection).map_err(|_| NodeRailError::StateCorrupt)?;
        match version {
            NODE_RAIL_SCHEMA_VERSION => {}
            4 => migrate_v4_to_v5(&connection)?,
            3 => {
                migrate_v3_to_v4(&connection)?;
                migrate_v4_to_v5(&connection)?;
            }
            2 => {
                migrate_v2_to_v3(&connection)?;
                migrate_v3_to_v4(&connection)?;
                migrate_v4_to_v5(&connection)?;
            }
            1 => {
                migrate_v1_to_v2(&connection)?;
                migrate_v2_to_v3(&connection)?;
                migrate_v3_to_v4(&connection)?;
                migrate_v4_to_v5(&connection)?;
            }
            _ => return Err(NodeRailError::StateVersionUnsupported),
        }
        verify_schema(&connection)?;
    } else {
        configure_database(&connection)?;
        initialize_schema(&connection)?;
    }
    configure_database(&connection)?;
    quick_check(&connection)?;
    bind_identity(&connection, binding)?;
    verify_storage_invariants(&connection)?;
    execution::recover_interrupted_claims(&mut connection, current_time_ms()?)?;
    verify_storage_invariants(&connection)?;
    secure_state_files(&root)?;
    Ok(connection)
}

pub(super) fn bootstrap_hello(
    connection: &mut Connection,
    capabilities: &CapabilityDescriptor,
    active_run_ids: &[String],
    sent_at_ms: i64,
) -> Result<NodeBootstrap, NodeRailError> {
    validate_timestamp(sent_at_ms)?;
    capabilities
        .validate()
        .map_err(|_| NodeRailError::InvalidMessage)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;

    if meta.last_node_sequence > 0 {
        let envelope = current_bootstrap_envelope(&transaction)?;
        let HubNodeMessage::Hello {
            capabilities: stored_capabilities,
            resume_after_sequence,
            ..
        } = &envelope.message
        else {
            return Err(NodeRailError::StateCorrupt);
        };
        if *resume_after_sequence != envelope.ack_sequence.unwrap_or(0)
            || envelope.device_id != meta.device_id
            || envelope.connection_id != meta.connection_id
            || envelope.protocol_version != meta.protocol_version
        {
            return Err(NodeRailError::StateCorrupt);
        }
        if stored_capabilities == capabilities {
            transaction.commit()?;
            return Ok(NodeBootstrap {
                envelope,
                capability_state: NodeBootstrapCapabilityState::Current,
            });
        }
        if !bootstrap_rotation_is_safe(&transaction, &meta, active_run_ids)? {
            transaction.commit()?;
            return Ok(NodeBootstrap {
                envelope,
                capability_state: NodeBootstrapCapabilityState::RotationDeferred,
            });
        }

        transaction.execute(
            "UPDATE node_rail_outbox SET is_bootstrap = 0 WHERE is_bootstrap = 1",
            [],
        )?;
        meta.connection_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let sequence = meta
            .last_node_sequence
            .checked_add(1)
            .ok_or(NodeRailError::SequenceExhausted)?;
        let rotated = HubNodeEnvelope {
            protocol_version: meta.protocol_version,
            device_id: meta.device_id.clone(),
            connection_id: meta.connection_id.clone(),
            sequence,
            ack_sequence: (meta.last_hub_sequence > 0).then_some(meta.last_hub_sequence),
            sent_at_ms,
            message: HubNodeMessage::Hello {
                role: DeviceRole::Node,
                capabilities: capabilities.clone(),
                resume_after_sequence: meta.last_hub_sequence,
                active_run_ids: Vec::new(),
            },
        };
        rotated
            .validate()
            .map_err(|_| NodeRailError::InvalidMessage)?;
        append_outbox(&transaction, &rotated, true)?;
        meta.last_node_sequence = sequence;
        write_meta_identity_and_cursors(&transaction, &meta, sent_at_ms)?;
        transaction.commit()?;
        return Ok(NodeBootstrap {
            envelope: rotated,
            capability_state: NodeBootstrapCapabilityState::Current,
        });
    }

    let envelope = HubNodeEnvelope {
        protocol_version: meta.protocol_version,
        device_id: meta.device_id.clone(),
        connection_id: meta.connection_id.clone(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms,
        message: HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: capabilities.clone(),
            resume_after_sequence: 0,
            active_run_ids: active_run_ids.to_vec(),
        },
    };
    envelope
        .validate()
        .map_err(|_| NodeRailError::InvalidMessage)?;
    append_outbox(&transaction, &envelope, true)?;
    meta.last_node_sequence = 1;
    write_meta_cursors(&transaction, &meta, sent_at_ms)?;
    transaction.commit()?;
    Ok(NodeBootstrap {
        envelope,
        capability_state: NodeBootstrapCapabilityState::Current,
    })
}

pub(super) fn enqueue(
    connection: &mut Connection,
    message: HubNodeMessage,
    sent_at_ms: i64,
) -> Result<HubNodeEnvelope, NodeRailError> {
    validate_timestamp(sent_at_ms)?;
    if !is_node_message(&message) {
        return Err(NodeRailError::InvalidMessage);
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    if meta.acknowledged_node_sequence == 0 {
        return Err(NodeRailError::ConnectionNotReady);
    }
    let envelope = append_next_outbox(&transaction, &mut meta, message, sent_at_ms)?;
    write_meta_cursors(&transaction, &meta, sent_at_ms)?;
    transaction.commit()?;
    Ok(envelope)
}

pub(super) fn ensure_heartbeat(
    connection: &mut Connection,
    active_run_ids: &[String],
    sent_at_ms: i64,
) -> Result<HubNodeEnvelope, NodeRailError> {
    validate_timestamp(sent_at_ms)?;
    let mut active_run_ids = active_run_ids.to_vec();
    active_run_ids.sort();
    let message = HubNodeMessage::Heartbeat {
        active_run_ids: active_run_ids.clone(),
    };
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    if meta.acknowledged_node_sequence == 0 {
        return Err(NodeRailError::ConnectionNotReady);
    }
    let existing = {
        let mut statement = transaction.prepare(
            "SELECT sequence, message_kind, envelope_json, envelope_sha256
             FROM node_rail_outbox
             WHERE sequence > ?1 AND message_kind = 'heartbeat'
             ORDER BY sequence DESC",
        )?;
        let rows = statement.query_map([u64_to_i64(meta.acknowledged_node_sequence)?], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut matched = None;
        for row in rows {
            let (sequence, kind, raw, digest) = row?;
            let envelope = decode_envelope(&raw, &digest, i64_to_u64(sequence)?, &kind)?;
            if matches!(
                &envelope.message,
                HubNodeMessage::Heartbeat {
                    active_run_ids: pending,
                } if pending == &active_run_ids
            ) {
                matched = Some(envelope);
                break;
            }
        }
        matched
    };
    if let Some(existing) = existing {
        transaction.commit()?;
        return Ok(existing);
    }
    let envelope = append_next_outbox(&transaction, &mut meta, message, sent_at_ms)?;
    write_meta_cursors(&transaction, &meta, sent_at_ms)?;
    transaction.commit()?;
    Ok(envelope)
}

pub(super) fn pending_outbound(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<HubNodeEnvelope>, NodeRailError> {
    let meta = read_meta(connection)?;
    let mut statement = connection.prepare(
        "SELECT sequence, message_kind, envelope_json, envelope_sha256
         FROM node_rail_outbox
         WHERE sequence > ?1
         ORDER BY sequence
         LIMIT ?2",
    )?;
    let rows = statement.query_map(
        params![
            u64_to_i64(meta.acknowledged_node_sequence)?,
            page_limit(limit)
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    rows.map(|row| {
        let (sequence, kind, raw, digest) = row?;
        decode_envelope(&raw, &digest, i64_to_u64(sequence)?, &kind)
    })
    .collect()
}

pub(super) fn observe_delivery(
    connection: &mut Connection,
    batch: &HubNodeDeliveryBatch,
    received_at_ms: i64,
) -> Result<NodeDeliveryOutcome, NodeRailError> {
    validate_timestamp(received_at_ms)?;
    batch
        .validate()
        .map_err(|_| NodeRailError::InvalidMessage)?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    ensure_batch_identity(&meta, batch)?;
    if batch.acknowledged_node_sequence < meta.acknowledged_node_sequence
        || batch.acknowledged_node_sequence > meta.last_node_sequence
    {
        return Err(NodeRailError::InvalidAcknowledgement);
    }
    let acknowledgement_advanced =
        batch.acknowledged_node_sequence > meta.acknowledged_node_sequence;
    if acknowledgement_advanced {
        advance_node_acknowledgement(&transaction, &mut meta, batch.acknowledged_node_sequence)?;
    }

    let (mut inbox_records, mut inbox_bytes) = inbox_usage(&transaction)?;
    let mut newly_recorded = 0usize;
    let mut duplicate_messages = 0usize;
    for envelope in &batch.messages {
        let raw = serde_json::to_vec(envelope).map_err(|_| NodeRailError::InvalidMessage)?;
        let digest = sha256_hex(&raw);
        if envelope.sequence <= meta.pruned_hub_sequence {
            return Err(NodeRailError::ReplayConflict);
        }
        if envelope.sequence <= meta.last_hub_sequence {
            let stored_digest = transaction
                .query_row(
                    "SELECT envelope_sha256 FROM node_rail_inbox WHERE sequence = ?1",
                    [u64_to_i64(envelope.sequence)?],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if stored_digest.as_deref() != Some(digest.as_str()) {
                return Err(NodeRailError::ReplayConflict);
            }
            duplicate_messages += 1;
            continue;
        }
        if meta.last_hub_sequence.checked_add(1) != Some(envelope.sequence) {
            return Err(NodeRailError::SequenceGap);
        }
        let raw_len = raw.len();
        if inbox_records >= MAX_LOCAL_RAIL_RECORDS
            || inbox_bytes
                .checked_add(raw_len)
                .is_none_or(|bytes| bytes > MAX_LOCAL_RAIL_BYTES)
        {
            return Err(NodeRailError::InboxFull);
        }
        transaction.execute(
            "INSERT INTO node_rail_inbox (
                 sequence, connection_id, message_kind, envelope_json,
                 envelope_sha256, received_at_ms, applied_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                u64_to_i64(envelope.sequence)?,
                envelope.connection_id,
                message_kind(&envelope.message),
                raw,
                digest,
                received_at_ms,
                matches!(&envelope.message, HubNodeMessage::Superseded { .. })
                    .then_some(received_at_ms),
            ],
        )?;
        meta.last_hub_sequence = envelope.sequence;
        inbox_records += 1;
        inbox_bytes += raw_len;
        newly_recorded += 1;
    }

    let acknowledgement_enqueued = if newly_recorded > 0
        && highest_pending_hub_ack(&transaction, meta.acknowledged_node_sequence)?
            < meta.last_hub_sequence
    {
        append_next_outbox(
            &transaction,
            &mut meta,
            HubNodeMessage::AckOnly,
            received_at_ms,
        )?;
        true
    } else {
        false
    };
    write_meta_cursors(&transaction, &meta, received_at_ms)?;
    transaction.commit()?;
    Ok(NodeDeliveryOutcome {
        newly_recorded,
        duplicate_messages,
        acknowledgement_advanced,
        acknowledgement_enqueued,
        acknowledged_node_sequence: meta.acknowledged_node_sequence,
        last_hub_sequence: meta.last_hub_sequence,
    })
}

pub(super) fn pending_inbound(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<NodeInboundRecord>, NodeRailError> {
    let mut statement = connection.prepare(
        "SELECT sequence, message_kind, envelope_json, envelope_sha256, received_at_ms
         FROM node_rail_inbox
         WHERE applied_at_ms IS NULL
         ORDER BY sequence
         LIMIT ?1",
    )?;
    let rows = statement.query_map([page_limit(limit)], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (sequence, kind, raw, digest, received_at_ms) = row?;
        Ok(NodeInboundRecord {
            envelope: decode_envelope(&raw, &digest, i64_to_u64(sequence)?, &kind)?,
            received_at_ms,
        })
    })
    .collect()
}

pub(super) fn mark_inbound_applied(
    connection: &mut Connection,
    sequence: u64,
    applied_at_ms: i64,
) -> Result<(), NodeRailError> {
    validate_timestamp(applied_at_ms)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    mark_inbound_applied_in_tx(&transaction, &mut meta, sequence, applied_at_ms)?;
    write_meta_cursors(&transaction, &meta, applied_at_ms)?;
    transaction.commit()?;
    Ok(())
}

fn mark_inbound_applied_in_tx(
    transaction: &Transaction<'_>,
    meta: &mut RailMeta,
    sequence: u64,
    applied_at_ms: i64,
) -> Result<(), NodeRailError> {
    if sequence <= meta.pruned_hub_sequence {
        return Ok(());
    }
    let applied = transaction
        .query_row(
            "SELECT applied_at_ms FROM node_rail_inbox WHERE sequence = ?1",
            [u64_to_i64(sequence)?],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?;
    match applied {
        Some(Some(_)) => {
            return Ok(());
        }
        Some(None) => {}
        None => return Err(NodeRailError::ApplyOrderConflict),
    }
    let oldest = transaction.query_row(
        "SELECT MIN(sequence) FROM node_rail_inbox WHERE applied_at_ms IS NULL",
        [],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    if oldest != Some(u64_to_i64(sequence)?) {
        return Err(NodeRailError::ApplyOrderConflict);
    }
    transaction.execute(
        "UPDATE node_rail_inbox SET applied_at_ms = ?2 WHERE sequence = ?1",
        params![u64_to_i64(sequence)?, applied_at_ms],
    )?;
    prune_confirmed_inbox(transaction, meta)?;
    Ok(())
}

pub(super) fn snapshot(connection: &Connection) -> Result<NodeRailSnapshot, NodeRailError> {
    let meta = read_meta(connection)?;
    let pending_outbound = connection.query_row(
        "SELECT COUNT(*) FROM node_rail_outbox WHERE sequence > ?1",
        [u64_to_i64(meta.acknowledged_node_sequence)?],
        |row| row.get::<_, i64>(0),
    )?;
    let pending_inbound = connection.query_row(
        "SELECT COUNT(*) FROM node_rail_inbox WHERE applied_at_ms IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(NodeRailSnapshot {
        device_id: meta.device_id,
        connection_id: meta.connection_id,
        last_node_sequence: meta.last_node_sequence,
        acknowledged_node_sequence: meta.acknowledged_node_sequence,
        last_hub_sequence: meta.last_hub_sequence,
        confirmed_hub_ack_sequence: meta.confirmed_hub_ack_sequence,
        pending_outbound: i64_to_usize(pending_outbound)?,
        pending_inbound: i64_to_usize(pending_inbound)?,
    })
}

pub(super) fn ensure_hub_identity(
    connection: &Connection,
    hub_sha256: &str,
) -> Result<(), NodeRailError> {
    let meta = read_meta(connection)?;
    if meta.hub_sha256 == hub_sha256 {
        Ok(())
    } else {
        Err(NodeRailError::IdentityConflict)
    }
}

fn configure_database(connection: &Connection) -> Result<(), NodeRailError> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000)?;
    let journal_mode = connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
        row.get::<_, String>(0)
    })?;
    if journal_mode != "wal" {
        return Err(NodeRailError::StateUnavailable);
    }
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), NodeRailError> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE node_rail_meta (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             hub_sha256 TEXT NOT NULL,
             device_id TEXT NOT NULL,
             protocol_major INTEGER NOT NULL,
             protocol_minor INTEGER NOT NULL,
             connection_id TEXT NOT NULL,
             last_node_sequence INTEGER NOT NULL CHECK (last_node_sequence >= 0),
             acknowledged_node_sequence INTEGER NOT NULL
                 CHECK (acknowledged_node_sequence >= 0
                        AND acknowledged_node_sequence <= last_node_sequence),
             last_hub_sequence INTEGER NOT NULL CHECK (last_hub_sequence >= 0),
             confirmed_hub_ack_sequence INTEGER NOT NULL
                 CHECK (confirmed_hub_ack_sequence >= 0
                        AND confirmed_hub_ack_sequence <= last_hub_sequence),
             pruned_hub_sequence INTEGER NOT NULL
                 CHECK (pruned_hub_sequence >= 0
                        AND pruned_hub_sequence <= confirmed_hub_ack_sequence),
             created_at_ms INTEGER NOT NULL CHECK (created_at_ms > 0),
             updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms > 0)
         );
         CREATE TABLE node_rail_outbox (
             sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
             connection_id TEXT NOT NULL,
             message_kind TEXT NOT NULL,
             envelope_json BLOB NOT NULL,
             envelope_sha256 TEXT NOT NULL,
             hub_ack_sequence INTEGER NOT NULL CHECK (hub_ack_sequence >= 0),
             is_bootstrap INTEGER NOT NULL DEFAULT 0 CHECK (is_bootstrap IN (0, 1)),
             created_at_ms INTEGER NOT NULL CHECK (created_at_ms > 0)
         );
         CREATE UNIQUE INDEX one_node_rail_bootstrap
             ON node_rail_outbox(is_bootstrap) WHERE is_bootstrap = 1;
         CREATE TABLE node_rail_inbox (
             sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
             connection_id TEXT NOT NULL,
             message_kind TEXT NOT NULL,
             envelope_json BLOB NOT NULL,
             envelope_sha256 TEXT NOT NULL,
             received_at_ms INTEGER NOT NULL CHECK (received_at_ms > 0),
             applied_at_ms INTEGER CHECK (applied_at_ms > 0)
         );
         PRAGMA user_version = 1;
         COMMIT;",
    )?;
    migrate_v1_to_v2(connection)?;
    migrate_v2_to_v3(connection)?;
    migrate_v3_to_v4(connection)?;
    migrate_v4_to_v5(connection)
}

fn migrate_v1_to_v2(connection: &Connection) -> Result<(), NodeRailError> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE node_runs (
             run_id TEXT NOT NULL,
             attempt INTEGER NOT NULL,
             idempotency_key TEXT NOT NULL,
             workspace_id TEXT NOT NULL,
             tool_name TEXT NOT NULL,
             input_json BLOB NOT NULL,
             input_sha256 TEXT NOT NULL,
             effect TEXT NOT NULL,
             lease_expires_at_ms INTEGER NOT NULL,
             status TEXT NOT NULL,
             effect_started INTEGER NOT NULL DEFAULT 0 CHECK (effect_started IN (0, 1)),
             inbound_sequence INTEGER NOT NULL UNIQUE,
             decision_json BLOB NOT NULL,
             decision_sha256 TEXT NOT NULL,
             decision_outbound_sequence INTEGER,
             terminal_outbound_sequence INTEGER,
             created_at_ms INTEGER NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             terminal_at_ms INTEGER,
             PRIMARY KEY(run_id, attempt),
             CHECK(length(run_id) BETWEEN 1 AND 128),
             CHECK(run_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(attempt BETWEEN 1 AND 4294967295),
             CHECK(length(idempotency_key) BETWEEN 1 AND 128),
             CHECK(idempotency_key NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(workspace_id) BETWEEN 1 AND 128),
             CHECK(workspace_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(tool_name) BETWEEN 1 AND 128),
             CHECK(tool_name NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(input_json) BETWEEN 2 AND 1048576),
             CHECK(length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(effect IN ('read_only', 'local_mutation', 'external_effect')),
             CHECK(lease_expires_at_ms > 0),
             CHECK(status IN (
                 'approval_pending', 'accepted', 'running', 'cancel_requested',
                 'rejected', 'succeeded', 'failed', 'cancelled', 'uncertain'
             )),
             CHECK(inbound_sequence > 0),
             CHECK(length(decision_json) BETWEEN 2 AND 8192),
             CHECK(length(decision_sha256) = 64 AND decision_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(decision_outbound_sequence IS NULL OR decision_outbound_sequence > 0),
             CHECK(terminal_outbound_sequence IS NULL OR terminal_outbound_sequence > 0),
             CHECK(created_at_ms > 0),
             CHECK(updated_at_ms >= created_at_ms),
             CHECK(terminal_at_ms IS NULL OR terminal_at_ms >= created_at_ms),
             CHECK(
                 (status IN ('rejected', 'succeeded', 'failed', 'cancelled', 'uncertain'))
                 = (terminal_at_ms IS NOT NULL)
             )
         );
         CREATE INDEX node_runs_active
             ON node_runs(status, lease_expires_at_ms, run_id, attempt);
         CREATE TABLE node_run_approvals (
             approval_id TEXT PRIMARY KEY,
             run_id TEXT NOT NULL,
             attempt INTEGER NOT NULL,
             action_digest TEXT NOT NULL,
             request_json BLOB NOT NULL,
             request_sha256 TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'pending',
             decision_json BLOB,
             decision_sha256 TEXT,
             requested_at_ms INTEGER NOT NULL,
             expires_at_ms INTEGER NOT NULL,
             decided_at_ms INTEGER,
             FOREIGN KEY(run_id, attempt) REFERENCES node_runs(run_id, attempt) ON DELETE RESTRICT,
             UNIQUE(run_id, attempt),
             CHECK(length(approval_id) BETWEEN 1 AND 128),
             CHECK(approval_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(action_digest) = 64 AND action_digest NOT GLOB '*[^0-9a-f]*'),
             CHECK(length(request_json) BETWEEN 2 AND 8192),
             CHECK(length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(status IN ('pending', 'approved', 'denied', 'timed_out')),
             CHECK(decision_json IS NULL OR length(decision_json) BETWEEN 2 AND 4096),
             CHECK(decision_sha256 IS NULL OR (
                 length(decision_sha256) = 64 AND decision_sha256 NOT GLOB '*[^0-9a-f]*'
             )),
             CHECK(requested_at_ms > 0),
             CHECK(expires_at_ms > requested_at_ms),
             CHECK(decided_at_ms IS NULL OR decided_at_ms >= requested_at_ms),
             CHECK((decision_json IS NULL) = (decision_sha256 IS NULL)),
             CHECK(
                 (status = 'pending' AND decision_json IS NULL AND decided_at_ms IS NULL)
                 OR
                 (status <> 'pending' AND decision_json IS NOT NULL AND decided_at_ms IS NOT NULL)
             )
         );
         PRAGMA user_version = 2;
         COMMIT;",
    )?;
    Ok(())
}

fn migrate_v2_to_v3(connection: &Connection) -> Result<(), NodeRailError> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE node_runs ADD COLUMN approval_decision_inbound_sequence INTEGER
             CHECK(approval_decision_inbound_sequence IS NULL
                   OR approval_decision_inbound_sequence > 0);
         ALTER TABLE node_runs ADD COLUMN acceptance_outbound_sequence INTEGER
             CHECK(acceptance_outbound_sequence IS NULL
                   OR acceptance_outbound_sequence > 0);
         ALTER TABLE node_runs ADD COLUMN terminal_json BLOB
             CHECK(terminal_json IS NULL OR length(terminal_json) BETWEEN 2 AND 1048576);
         ALTER TABLE node_runs ADD COLUMN terminal_sha256 TEXT
             CHECK(terminal_sha256 IS NULL OR (
                 length(terminal_sha256) = 64
                 AND terminal_sha256 NOT GLOB '*[^0-9a-f]*'
             ));
         CREATE UNIQUE INDEX node_runs_approval_decision_inbound
             ON node_runs(approval_decision_inbound_sequence)
             WHERE approval_decision_inbound_sequence IS NOT NULL;
         CREATE UNIQUE INDEX node_runs_acceptance_outbound
             ON node_runs(acceptance_outbound_sequence)
             WHERE acceptance_outbound_sequence IS NOT NULL;
         UPDATE node_runs
         SET acceptance_outbound_sequence = decision_outbound_sequence
         WHERE status = 'accepted';
         UPDATE node_runs
         SET terminal_outbound_sequence = decision_outbound_sequence,
             terminal_json = decision_json,
             terminal_sha256 = decision_sha256
         WHERE status = 'rejected';
         PRAGMA user_version = 3;
         COMMIT;",
    )?;
    Ok(())
}

fn migrate_v3_to_v4(connection: &Connection) -> Result<(), NodeRailError> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE node_runs ADD COLUMN cancel_inbound_sequence INTEGER
             CHECK(cancel_inbound_sequence IS NULL OR cancel_inbound_sequence > 0);
         ALTER TABLE node_runs ADD COLUMN cancel_json BLOB
             CHECK(cancel_json IS NULL OR length(cancel_json) BETWEEN 2 AND 8192);
         ALTER TABLE node_runs ADD COLUMN cancel_sha256 TEXT
             CHECK(cancel_sha256 IS NULL OR (
                 length(cancel_sha256) = 64
                 AND cancel_sha256 NOT GLOB '*[^0-9a-f]*'
             ));
         CREATE UNIQUE INDEX node_runs_cancel_inbound
             ON node_runs(cancel_inbound_sequence)
             WHERE cancel_inbound_sequence IS NOT NULL;
         PRAGMA user_version = 4;
         COMMIT;",
    )?;
    Ok(())
}

fn migrate_v4_to_v5(connection: &Connection) -> Result<(), NodeRailError> {
    let unsupported_active_effects = connection.query_row(
        "SELECT COUNT(*) FROM node_runs
         WHERE effect_started <> 0 OR status IN ('running', 'cancel_requested')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if unsupported_active_effects != 0 {
        return Err(NodeRailError::StateCorrupt);
    }
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE node_runs ADD COLUMN execution_claim_id TEXT
             CHECK(execution_claim_id IS NULL OR length(execution_claim_id) = 36);
         ALTER TABLE node_runs ADD COLUMN execution_claim_started_at_ms INTEGER
             CHECK(execution_claim_started_at_ms IS NULL OR execution_claim_started_at_ms > 0);
         CREATE UNIQUE INDEX node_runs_execution_claim
             ON node_runs(execution_claim_id)
             WHERE execution_claim_id IS NOT NULL;
         CREATE TABLE node_run_claims (
             claim_id TEXT PRIMARY KEY,
             run_id TEXT NOT NULL,
             attempt INTEGER NOT NULL,
             status TEXT NOT NULL,
             started_at_ms INTEGER NOT NULL,
             finished_at_ms INTEGER,
             FOREIGN KEY(run_id, attempt) REFERENCES node_runs(run_id, attempt) ON DELETE RESTRICT,
             CHECK(length(claim_id) = 36),
             CHECK(attempt BETWEEN 1 AND 4294967295),
             CHECK(status IN (
                 'claimed', 'completed', 'interrupted_retryable',
                 'interrupted_cancelled', 'interrupted_uncertain'
             )),
             CHECK(started_at_ms > 0),
             CHECK(finished_at_ms IS NULL OR finished_at_ms >= started_at_ms),
             CHECK((status = 'claimed') = (finished_at_ms IS NULL))
         );
         CREATE UNIQUE INDEX one_node_run_active_claim
             ON node_run_claims(run_id, attempt) WHERE status = 'claimed';
         CREATE INDEX node_run_claim_history
             ON node_run_claims(run_id, attempt, started_at_ms, claim_id);
         PRAGMA user_version = 5;
         COMMIT;",
    )?;
    Ok(())
}

fn bind_identity(connection: &Connection, binding: &NodeStateBinding) -> Result<(), NodeRailError> {
    let now_ms = current_time_ms()?;
    let protocol_version = HUB_NODE_PROTOCOL_VERSION
        .negotiate(binding.protocol_version)
        .map_err(|_| NodeRailError::IdentityConflict)?;
    let stored = read_meta_optional(connection)?;
    if let Some(stored) = stored {
        validate_meta(&stored)?;
        if stored.hub_sha256 != binding.hub_sha256
            || stored.device_id != binding.device_id
            || stored.protocol_version != protocol_version
        {
            return Err(NodeRailError::IdentityConflict);
        }
        return Ok(());
    }
    let connection_id = uuid::Uuid::new_v4().hyphenated().to_string();
    connection.execute(
        "INSERT INTO node_rail_meta (
             singleton, hub_sha256, device_id, protocol_major, protocol_minor,
             connection_id, last_node_sequence, acknowledged_node_sequence,
             last_hub_sequence, confirmed_hub_ack_sequence, pruned_hub_sequence,
             created_at_ms, updated_at_ms
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, 0, 0, 0, 0, 0, ?6, ?6)",
        params![
            binding.hub_sha256,
            binding.device_id,
            i64::from(protocol_version.major),
            i64::from(protocol_version.minor),
            connection_id,
            now_ms,
        ],
    )?;
    Ok(())
}

fn verify_schema(connection: &Connection) -> Result<(), NodeRailError> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if !names.iter().map(String::as_str).eq(EXPECTED_TABLES) {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

fn verify_storage_invariants(connection: &Connection) -> Result<(), NodeRailError> {
    let meta = read_meta(connection)?;
    validate_meta(&meta)?;
    let bootstrap_count = connection.query_row(
        "SELECT COUNT(*) FROM node_rail_outbox
         WHERE is_bootstrap = 1 AND message_kind = 'hello'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if (meta.last_node_sequence == 0 && bootstrap_count != 0)
        || (meta.last_node_sequence > 0 && bootstrap_count != 1)
    {
        return Err(NodeRailError::StateCorrupt);
    }
    if meta.last_node_sequence > 0 {
        let bootstrap = current_bootstrap_envelope(connection)?;
        if bootstrap.device_id != meta.device_id
            || bootstrap.connection_id != meta.connection_id
            || bootstrap.protocol_version != meta.protocol_version
            || !matches!(bootstrap.message, HubNodeMessage::Hello { .. })
        {
            return Err(NodeRailError::StateCorrupt);
        }
    }
    let invalid_outbox = connection.query_row(
        "SELECT COUNT(*) FROM node_rail_outbox
         WHERE sequence > ?1 OR (is_bootstrap = 0 AND sequence <= ?2)",
        params![
            u64_to_i64(meta.last_node_sequence)?,
            u64_to_i64(meta.acknowledged_node_sequence)?,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let invalid_inbox = connection.query_row(
        "SELECT COUNT(*) FROM node_rail_inbox
         WHERE sequence > ?1 OR sequence <= ?2",
        params![
            u64_to_i64(meta.last_hub_sequence)?,
            u64_to_i64(meta.pruned_hub_sequence)?,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if invalid_outbox != 0 || invalid_inbox != 0 {
        return Err(NodeRailError::StateCorrupt);
    }
    runs::verify_run_invariants(connection)
}

fn read_meta(connection: &Connection) -> Result<RailMeta, NodeRailError> {
    read_meta_optional(connection)?.ok_or(NodeRailError::StateCorrupt)
}

fn read_meta_optional(connection: &Connection) -> Result<Option<RailMeta>, NodeRailError> {
    connection
        .query_row(
            "SELECT hub_sha256, device_id, protocol_major, protocol_minor,
                    connection_id, last_node_sequence, acknowledged_node_sequence,
                    last_hub_sequence, confirmed_hub_ack_sequence, pruned_hub_sequence
             FROM node_rail_meta WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()?
        .map(|stored| {
            Ok(RailMeta {
                hub_sha256: stored.0,
                device_id: stored.1,
                protocol_version: ProtocolVersion {
                    major: i64_to_u16(stored.2)?,
                    minor: i64_to_u16(stored.3)?,
                },
                connection_id: stored.4,
                last_node_sequence: i64_to_u64(stored.5)?,
                acknowledged_node_sequence: i64_to_u64(stored.6)?,
                last_hub_sequence: i64_to_u64(stored.7)?,
                confirmed_hub_ack_sequence: i64_to_u64(stored.8)?,
                pruned_hub_sequence: i64_to_u64(stored.9)?,
            })
        })
        .transpose()
}

fn validate_meta(meta: &RailMeta) -> Result<(), NodeRailError> {
    if !valid_sha256(&meta.hub_sha256)
        || uuid::Uuid::parse_str(&meta.connection_id)
            .ok()
            .is_none_or(|id| id.hyphenated().to_string() != meta.connection_id)
        || meta.acknowledged_node_sequence > meta.last_node_sequence
        || meta.confirmed_hub_ack_sequence > meta.last_hub_sequence
        || meta.pruned_hub_sequence > meta.confirmed_hub_ack_sequence
        || HUB_NODE_PROTOCOL_VERSION
            .negotiate(meta.protocol_version)
            .is_err()
    {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

fn write_meta_cursors(
    transaction: &Transaction<'_>,
    meta: &RailMeta,
    updated_at_ms: i64,
) -> Result<(), NodeRailError> {
    validate_meta(meta)?;
    let changed = transaction.execute(
        "UPDATE node_rail_meta
         SET last_node_sequence = ?1, acknowledged_node_sequence = ?2,
             last_hub_sequence = ?3, confirmed_hub_ack_sequence = ?4,
             pruned_hub_sequence = ?5, updated_at_ms = ?6
         WHERE singleton = 1",
        params![
            u64_to_i64(meta.last_node_sequence)?,
            u64_to_i64(meta.acknowledged_node_sequence)?,
            u64_to_i64(meta.last_hub_sequence)?,
            u64_to_i64(meta.confirmed_hub_ack_sequence)?,
            u64_to_i64(meta.pruned_hub_sequence)?,
            updated_at_ms,
        ],
    )?;
    if changed != 1 {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

fn write_meta_identity_and_cursors(
    transaction: &Transaction<'_>,
    meta: &RailMeta,
    updated_at_ms: i64,
) -> Result<(), NodeRailError> {
    let changed = transaction.execute(
        "UPDATE node_rail_meta SET connection_id = ?1 WHERE singleton = 1",
        [&meta.connection_id],
    )?;
    if changed != 1 {
        return Err(NodeRailError::StateCorrupt);
    }
    write_meta_cursors(transaction, meta, updated_at_ms)
}

fn append_next_outbox(
    transaction: &Transaction<'_>,
    meta: &mut RailMeta,
    message: HubNodeMessage,
    sent_at_ms: i64,
) -> Result<HubNodeEnvelope, NodeRailError> {
    let sequence = meta
        .last_node_sequence
        .checked_add(1)
        .ok_or(NodeRailError::SequenceExhausted)?;
    let envelope = HubNodeEnvelope {
        protocol_version: meta.protocol_version,
        device_id: meta.device_id.clone(),
        connection_id: meta.connection_id.clone(),
        sequence,
        ack_sequence: (meta.last_hub_sequence > 0).then_some(meta.last_hub_sequence),
        sent_at_ms,
        message,
    };
    envelope
        .validate()
        .map_err(|_| NodeRailError::InvalidMessage)?;
    append_outbox(transaction, &envelope, false)?;
    meta.last_node_sequence = sequence;
    Ok(envelope)
}

fn append_outbox(
    transaction: &Transaction<'_>,
    envelope: &HubNodeEnvelope,
    is_bootstrap: bool,
) -> Result<(), NodeRailError> {
    let raw = serde_json::to_vec(envelope).map_err(|_| NodeRailError::InvalidMessage)?;
    let digest = sha256_hex(&raw);
    let (records, bytes) = outbox_usage(transaction)?;
    if records >= MAX_LOCAL_RAIL_RECORDS
        || bytes
            .checked_add(raw.len())
            .is_none_or(|total| total > MAX_LOCAL_RAIL_BYTES)
    {
        return Err(NodeRailError::OutboxFull);
    }
    transaction.execute(
        "INSERT INTO node_rail_outbox (
         sequence, connection_id, message_kind, envelope_json,
             envelope_sha256, hub_ack_sequence, is_bootstrap, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            u64_to_i64(envelope.sequence)?,
            envelope.connection_id,
            message_kind(&envelope.message),
            raw,
            digest,
            u64_to_i64(envelope.ack_sequence.unwrap_or(0))?,
            is_bootstrap,
            envelope.sent_at_ms,
        ],
    )?;
    Ok(())
}

fn current_bootstrap_envelope(connection: &Connection) -> Result<HubNodeEnvelope, NodeRailError> {
    let stored = connection
        .query_row(
            "SELECT sequence, message_kind, envelope_json, envelope_sha256
             FROM node_rail_outbox WHERE is_bootstrap = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(NodeRailError::StateCorrupt)?;
    decode_envelope(&stored.2, &stored.3, i64_to_u64(stored.0)?, &stored.1)
}

fn bootstrap_rotation_is_safe(
    connection: &Connection,
    meta: &RailMeta,
    active_run_ids: &[String],
) -> Result<bool, NodeRailError> {
    if !active_run_ids.is_empty()
        || meta.acknowledged_node_sequence != meta.last_node_sequence
        || meta.confirmed_hub_ack_sequence != meta.last_hub_sequence
        || meta.pruned_hub_sequence != meta.last_hub_sequence
    {
        return Ok(false);
    }
    let unapplied = connection.query_row(
        "SELECT COUNT(*) FROM node_rail_inbox WHERE applied_at_ms IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(unapplied == 0)
}

fn decode_envelope(
    raw: &[u8],
    expected_digest: &str,
    expected_sequence: u64,
    expected_kind: &str,
) -> Result<HubNodeEnvelope, NodeRailError> {
    if sha256_hex(raw) != expected_digest {
        return Err(NodeRailError::StateCorrupt);
    }
    let envelope: HubNodeEnvelope =
        serde_json::from_slice(raw).map_err(|_| NodeRailError::StateCorrupt)?;
    envelope
        .validate()
        .map_err(|_| NodeRailError::StateCorrupt)?;
    if envelope.sequence != expected_sequence || message_kind(&envelope.message) != expected_kind {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(envelope)
}

fn advance_node_acknowledgement(
    transaction: &Transaction<'_>,
    meta: &mut RailMeta,
    acknowledged: u64,
) -> Result<(), NodeRailError> {
    let expected_records = acknowledged
        .checked_sub(meta.acknowledged_node_sequence)
        .ok_or(NodeRailError::InvalidAcknowledgement)?;
    let stored_records = transaction.query_row(
        "SELECT COUNT(*) FROM node_rail_outbox WHERE sequence > ?1 AND sequence <= ?2",
        params![
            u64_to_i64(meta.acknowledged_node_sequence)?,
            u64_to_i64(acknowledged)?,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if i64_to_u64(stored_records)? != expected_records {
        return Err(NodeRailError::InvalidAcknowledgement);
    }
    let carried_ack = transaction.query_row(
        "SELECT COALESCE(MAX(hub_ack_sequence), 0)
         FROM node_rail_outbox WHERE sequence <= ?1",
        [u64_to_i64(acknowledged)?],
        |row| row.get::<_, i64>(0),
    )?;
    meta.confirmed_hub_ack_sequence = meta
        .confirmed_hub_ack_sequence
        .max(i64_to_u64(carried_ack)?);
    meta.acknowledged_node_sequence = acknowledged;
    transaction.execute(
        "DELETE FROM node_rail_outbox WHERE is_bootstrap = 0 AND sequence <= ?1",
        [u64_to_i64(acknowledged)?],
    )?;
    prune_confirmed_inbox(transaction, meta)
}

fn prune_confirmed_inbox(
    transaction: &Transaction<'_>,
    meta: &mut RailMeta,
) -> Result<(), NodeRailError> {
    if meta.confirmed_hub_ack_sequence <= meta.pruned_hub_sequence {
        return Ok(());
    }
    let first_unapplied = transaction.query_row(
        "SELECT MIN(sequence) FROM node_rail_inbox
         WHERE sequence > ?1 AND sequence <= ?2 AND applied_at_ms IS NULL",
        params![
            u64_to_i64(meta.pruned_hub_sequence)?,
            u64_to_i64(meta.confirmed_hub_ack_sequence)?,
        ],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    let prune_through = first_unapplied
        .and_then(|sequence| sequence.checked_sub(1))
        .map(i64_to_u64)
        .transpose()?
        .unwrap_or(meta.confirmed_hub_ack_sequence);
    if prune_through <= meta.pruned_hub_sequence {
        return Ok(());
    }
    transaction.execute(
        "DELETE FROM node_rail_inbox WHERE sequence <= ?1 AND applied_at_ms IS NOT NULL",
        [u64_to_i64(prune_through)?],
    )?;
    meta.pruned_hub_sequence = prune_through;
    Ok(())
}

fn highest_pending_hub_ack(
    connection: &Connection,
    acknowledged_node_sequence: u64,
) -> Result<u64, NodeRailError> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(hub_ack_sequence), 0)
             FROM node_rail_outbox WHERE sequence > ?1",
            [u64_to_i64(acknowledged_node_sequence)?],
            |row| row.get::<_, i64>(0),
        )
        .map_err(NodeRailError::from)
        .and_then(i64_to_u64)
}

fn ensure_batch_identity(
    meta: &RailMeta,
    batch: &HubNodeDeliveryBatch,
) -> Result<(), NodeRailError> {
    if batch.device_id != meta.device_id
        || batch.connection_id != meta.connection_id
        || batch.protocol_version != meta.protocol_version
    {
        return Err(NodeRailError::IdentityConflict);
    }
    Ok(())
}

fn outbox_usage(connection: &Connection) -> Result<(usize, usize), NodeRailError> {
    queue_usage(connection, "node_rail_outbox")
}

fn inbox_usage(connection: &Connection) -> Result<(usize, usize), NodeRailError> {
    queue_usage(connection, "node_rail_inbox")
}

fn queue_usage(
    connection: &Connection,
    table: &'static str,
) -> Result<(usize, usize), NodeRailError> {
    let sql = match table {
        "node_rail_outbox" => {
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(envelope_json)), 0) FROM node_rail_outbox"
        }
        "node_rail_inbox" => {
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(envelope_json)), 0) FROM node_rail_inbox"
        }
        _ => return Err(NodeRailError::StateCorrupt),
    };
    let (records, bytes) = connection.query_row(sql, [], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok((i64_to_usize(records)?, i64_to_usize(bytes)?))
}

fn quick_check(connection: &Connection) -> Result<(), NodeRailError> {
    let result = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
        .map_err(|_| NodeRailError::StateCorrupt)?;
    if result == "ok" {
        Ok(())
    } else {
        Err(NodeRailError::StateCorrupt)
    }
}

fn user_version(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

fn reject_unsafe_state_file(path: &Path) -> Result<bool, NodeRailError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(NodeRailError::UnsafeStatePath)
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(NodeRailError::StateUnavailable),
    }
}

#[cfg(unix)]
fn secure_state_files(root: &Path) -> Result<(), NodeRailError> {
    use std::os::unix::fs::PermissionsExt;
    for file_name in [NODE_RAIL_STATE_FILE, NODE_RAIL_WAL_FILE, NODE_RAIL_SHM_FILE] {
        let path = root.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(NodeRailError::UnsafeStatePath);
            }
            Ok(_) => fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|_| NodeRailError::StateUnavailable)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(NodeRailError::StateUnavailable),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_state_files(_root: &Path) -> Result<(), NodeRailError> {
    Ok(())
}

fn current_time_ms() -> Result<i64, NodeRailError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NodeRailError::StateUnavailable)?;
    i64::try_from(duration.as_millis()).map_err(|_| NodeRailError::StateUnavailable)
}

fn validate_timestamp(value: i64) -> Result<(), NodeRailError> {
    if value > 0 {
        Ok(())
    } else {
        Err(NodeRailError::InvalidMessage)
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn page_limit(limit: usize) -> i64 {
    i64::try_from(limit.clamp(1, MAX_LOCAL_RAIL_PAGE)).unwrap_or(MAX_LOCAL_RAIL_PAGE as i64)
}

fn u64_to_i64(value: u64) -> Result<i64, NodeRailError> {
    i64::try_from(value).map_err(|_| NodeRailError::SequenceExhausted)
}

fn i64_to_u64(value: i64) -> Result<u64, NodeRailError> {
    u64::try_from(value).map_err(|_| NodeRailError::StateCorrupt)
}

fn i64_to_u16(value: i64) -> Result<u16, NodeRailError> {
    u16::try_from(value).map_err(|_| NodeRailError::StateCorrupt)
}

fn i64_to_usize(value: i64) -> Result<usize, NodeRailError> {
    usize::try_from(value).map_err(|_| NodeRailError::StateCorrupt)
}
