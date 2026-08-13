use super::{
    acknowledge_hub_sequence_in_tx, append_outbox_in_tx, ensure_active_node,
    latest_outbox_for_device_kind, record_inbound_receipt_in_tx, sha256_hex, u64_to_i64,
    HubNodeOutboxRecord, HubNodeRailError, HubNodeRailStore, InboundReceipt,
};
use captain_wire::hub_protocol::{
    DeviceGrant, DeviceRole, HubNodeEnvelope, HubNodeMessage, NodeTransport, ProtocolVersion,
    HUB_NODE_PROTOCOL_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubNodeConnectionStatus {
    Active,
    Offline,
}

impl HubNodeConnectionStatus {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "active" => Self::Active,
            "offline" => Self::Offline,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubNodeConnectionRecord {
    pub device_id: String,
    pub connection_id: String,
    pub transport: NodeTransport,
    pub protocol_version: ProtocolVersion,
    pub status: HubNodeConnectionStatus,
    pub connected_at_ms: i64,
    pub last_seen_ms: i64,
    pub updated_at_ms: i64,
    pub disconnected_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHubNodeConnection {
    pub connection: HubNodeConnectionRecord,
    pub welcome: HubNodeOutboxRecord,
    pub last_node_sequence: u64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubNodeDeliverySnapshot {
    pub connection: HubNodeConnectionRecord,
    pub acknowledged_node_sequence: u64,
    pub messages: Vec<HubNodeOutboxRecord>,
}

struct RequeuedOutboxMessage {
    run_id: Option<String>,
    message: HubNodeMessage,
}

#[derive(Debug)]
struct AuthorizedNode {
    platform: String,
    protocol_version: ProtocolVersion,
    grants: DeviceGrant,
}

impl HubNodeRailStore {
    /// Atomically accepts a Node Hello, advances both durable cursors, records
    /// presence, and creates the Welcome payload. Retrying the exact same
    /// bootstrap Hello either returns its pending Welcome or reactivates the
    /// same logical connection without consuming another Node sequence.
    pub fn open_connection(
        &self,
        hello: &HubNodeEnvelope,
        transport: NodeTransport,
        heartbeat_interval_ms: u64,
        lease_duration_ms: u64,
        now_ms: i64,
    ) -> Result<OpenHubNodeConnection, HubNodeRailError> {
        hello
            .validate()
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        super::validate_now(now_ms)?;
        let lease_duration_ms = i64::try_from(lease_duration_ms).map_err(|_| {
            HubNodeRailError::InvalidInput("connection timing is invalid".to_string())
        })?;
        if heartbeat_interval_ms == 0
            || lease_duration_ms <= i64::try_from(heartbeat_interval_ms).unwrap_or(i64::MAX)
            || lease_duration_ms > super::MAX_LEASE_DURATION_MS
        {
            return Err(HubNodeRailError::InvalidInput(
                "connection timing is invalid".to_string(),
            ));
        }
        let lease_expires_at_ms = now_ms
            .checked_add(lease_duration_ms)
            .ok_or_else(|| HubNodeRailError::InvalidInput("lease expiry overflow".to_string()))?;
        let HubNodeMessage::Hello {
            role,
            capabilities,
            resume_after_sequence,
            active_run_ids,
        } = &hello.message
        else {
            return Err(HubNodeRailError::InvalidMessageDirection);
        };
        if *role != DeviceRole::Node || !capabilities.transports.contains(&transport) {
            return Err(HubNodeRailError::InvalidInput(
                "Node did not advertise the selected transport".to_string(),
            ));
        }
        let acknowledged = hello.ack_sequence.unwrap_or(0);
        if *resume_after_sequence != acknowledged {
            return Err(HubNodeRailError::InvalidInput(
                "Hello resume cursor does not match its acknowledgement".to_string(),
            ));
        }
        let capabilities_json = serde_json::to_string(capabilities)
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let envelope_json = serde_json::to_vec(hello)
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let envelope_sha256 = sha256_hex(&envelope_json);

        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_active_node(&tx, &hello.device_id)?;
        let authorized = authorized_node(&tx, &hello.device_id)?;
        if authorized.platform != capabilities.platform {
            return Err(HubNodeRailError::InvalidInput(
                "Node platform differs from its pairing claim".to_string(),
            ));
        }
        authorized
            .grants
            .validate_against(capabilities)
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let negotiated_version = HUB_NODE_PROTOCOL_VERSION
            .negotiate(authorized.protocol_version)
            .and_then(|version| version.negotiate(hello.protocol_version))
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;

        acknowledge_hub_sequence_in_tx(&tx, &hello.device_id, u64_to_i64(acknowledged)?, now_ms)?;
        let receipt = record_inbound_receipt_in_tx(
            &tx,
            &hello.device_id,
            &hello.connection_id,
            u64_to_i64(hello.sequence)?,
            "hello",
            &envelope_sha256,
            now_ms,
        )?;
        if receipt == InboundReceipt::Duplicate {
            let current = connection_by_device(&tx, &hello.device_id)?
                .filter(|row| row.connection_id == hello.connection_id)
                .ok_or(HubNodeRailError::ConnectionConflict)?;
            let pending_welcome = latest_outbox_for_device_kind(
                &tx,
                &hello.device_id,
                "welcome",
                current.connected_at_ms,
            )?;
            if current.status == HubNodeConnectionStatus::Active
                && current.transport == transport
                && pending_welcome
                    .as_ref()
                    .is_some_and(|welcome| welcome_is_pending_for_transport(welcome, transport))
            {
                let welcome = pending_welcome.ok_or(HubNodeRailError::StorageInvariant)?;
                let last_node_sequence = cursor_last_node_sequence(&tx, &hello.device_id)?;
                tx.commit()?;
                return Ok(OpenHubNodeConnection {
                    connection: current,
                    welcome,
                    last_node_sequence,
                    replayed: true,
                });
            }

            // A bootstrap replay can outlive the active runs it originally
            // advertised. Reconciliation therefore preserves only durable
            // pending Hub work here; the Node sends a fresh Heartbeat after
            // Welcome to report the current active-run set.
            let pending_to_requeue = supersede_pending_outbox_for_reconnect(
                &tx,
                &hello.device_id,
                &hello.connection_id,
                &[],
                lease_expires_at_ms,
                now_ms,
            )?;
            upsert_active_connection(
                &tx,
                hello,
                transport,
                negotiated_version,
                &capabilities_json,
                now_ms,
            )?;
            let welcome = append_welcome_and_requeued(
                &tx,
                &hello.device_id,
                transport,
                negotiated_version,
                heartbeat_interval_ms,
                u64::try_from(lease_duration_ms).map_err(|_| HubNodeRailError::StorageInvariant)?,
                pending_to_requeue,
                now_ms,
            )?;
            let connection = connection_by_device(&tx, &hello.device_id)?
                .ok_or(HubNodeRailError::StorageInvariant)?;
            let last_node_sequence = cursor_last_node_sequence(&tx, &hello.device_id)?;
            tx.commit()?;
            return Ok(OpenHubNodeConnection {
                connection,
                welcome,
                last_node_sequence,
                replayed: true,
            });
        }

        if connection_id_owner(&tx, &hello.connection_id)?
            .is_some_and(|owner| owner != hello.device_id)
        {
            return Err(HubNodeRailError::ConnectionConflict);
        }
        let pending_to_requeue = supersede_pending_outbox_for_reconnect(
            &tx,
            &hello.device_id,
            &hello.connection_id,
            active_run_ids,
            lease_expires_at_ms,
            now_ms,
        )?;
        upsert_active_connection(
            &tx,
            hello,
            transport,
            negotiated_version,
            &capabilities_json,
            now_ms,
        )?;
        let welcome = append_welcome_and_requeued(
            &tx,
            &hello.device_id,
            transport,
            negotiated_version,
            heartbeat_interval_ms,
            u64::try_from(lease_duration_ms).map_err(|_| HubNodeRailError::StorageInvariant)?,
            pending_to_requeue,
            now_ms,
        )?;
        let connection = connection_by_device(&tx, &hello.device_id)?
            .ok_or(HubNodeRailError::StorageInvariant)?;
        let last_node_sequence = cursor_last_node_sequence(&tx, &hello.device_id)?;
        tx.commit()?;
        Ok(OpenHubNodeConnection {
            connection,
            welcome,
            last_node_sequence,
            replayed: false,
        })
    }

    pub fn connection(
        &self,
        device_id: &str,
    ) -> Result<Option<HubNodeConnectionRecord>, HubNodeRailError> {
        super::validate_identifier("device id", device_id)?;
        let conn = self.lock()?;
        connection_by_device(&conn, device_id)
    }

    /// Read one coherent delivery page for the currently active connection.
    /// Superseded rows remain in sequence and are projected as explicit wire
    /// tombstones by the kernel, preserving both audit evidence and continuity.
    pub fn delivery_snapshot(
        &self,
        device_id: &str,
        connection_id: &str,
        limit: usize,
    ) -> Result<HubNodeDeliverySnapshot, HubNodeRailError> {
        super::validate_identifier("device id", device_id)?;
        super::validate_identifier("connection id", connection_id)?;
        let conn = self.lock()?;
        super::ensure_active_node(&conn, device_id)?;
        let connection = connection_by_device(&conn, device_id)?
            .filter(|row| {
                row.status == HubNodeConnectionStatus::Active && row.connection_id == connection_id
            })
            .ok_or(HubNodeRailError::ConnectionConflict)?;
        let acknowledged_node_sequence = cursor_last_node_sequence(&conn, device_id)?;
        let mut statement = conn.prepare(
            "SELECT device_id, sequence, message_kind, message_json,
                    message_sha256, run_id, created_at_ms, acked_at_ms,
                    superseded_at_ms
             FROM hub_node_outbox
             WHERE device_id = ?1 AND acked_at_ms IS NULL
             ORDER BY sequence LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![device_id, limit.clamp(1, super::MAX_OUTBOX_PAGE)],
            super::outbox_from_row,
        )?;
        let messages = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(HubNodeDeliverySnapshot {
            connection,
            acknowledged_node_sequence,
            messages,
        })
    }

    pub fn close_connection(
        &self,
        device_id: &str,
        connection_id: &str,
        error_code: Option<&str>,
        now_ms: i64,
    ) -> Result<HubNodeConnectionRecord, HubNodeRailError> {
        super::validate_identifier("device id", device_id)?;
        super::validate_identifier("connection id", connection_id)?;
        super::validate_now(now_ms)?;
        if error_code.is_some_and(|code| super::validate_kind(code).is_err()) {
            return Err(HubNodeRailError::InvalidInput(
                "connection error code is invalid".to_string(),
            ));
        }
        let conn = self.lock()?;
        let current =
            connection_by_device(&conn, device_id)?.ok_or(HubNodeRailError::ConnectionConflict)?;
        if current.connection_id != connection_id {
            return Err(HubNodeRailError::ConnectionConflict);
        }
        if current.status == HubNodeConnectionStatus::Offline {
            return Ok(current);
        }
        conn.execute(
            "UPDATE hub_node_connections
             SET status = 'offline', disconnected_at_ms = MAX(connected_at_ms, ?3),
                 last_error_code = ?4, updated_at_ms = MAX(updated_at_ms, ?3)
             WHERE device_id = ?1 AND connection_id = ?2 AND status = 'active'",
            params![device_id, connection_id, now_ms, error_code],
        )?;
        conn.execute(
            "UPDATE captain_devices
             SET last_error_code = ?2, updated_at_ms = MAX(updated_at_ms, ?3)
             WHERE device_id = ?1",
            params![device_id, error_code, now_ms],
        )?;
        connection_by_device(&conn, device_id)?.ok_or(HubNodeRailError::StorageInvariant)
    }

    pub fn reconcile_connections_after_restart(
        &self,
        now_ms: i64,
    ) -> Result<usize, HubNodeRailError> {
        super::validate_now(now_ms)?;
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE hub_node_connections
             SET status = 'offline', disconnected_at_ms = MAX(connected_at_ms, ?1),
                 last_error_code = 'runtime_restarted',
                 updated_at_ms = MAX(updated_at_ms, ?1)
             WHERE status = 'active'",
            [now_ms],
        )?;
        if changed > 0 {
            conn.execute(
                "UPDATE captain_devices
                 SET last_error_code = 'runtime_restarted',
                     updated_at_ms = MAX(updated_at_ms, ?1)
                 WHERE device_id IN (
                     SELECT device_id FROM hub_node_connections
                     WHERE status = 'offline'
                       AND last_error_code = 'runtime_restarted'
                 )",
                [now_ms],
            )?;
        }
        Ok(changed)
    }
}

fn welcome_is_pending_for_transport(
    record: &HubNodeOutboxRecord,
    transport: NodeTransport,
) -> bool {
    if record.acked_at_ms.is_some() || record.superseded_at_ms.is_some() {
        return false;
    }
    serde_json::from_str::<HubNodeMessage>(&record.message_json).is_ok_and(|message| {
        matches!(
            message,
            HubNodeMessage::Welcome {
                transport: selected,
                ..
            } if selected == transport
        )
    })
}

fn upsert_active_connection(
    tx: &Transaction<'_>,
    hello: &HubNodeEnvelope,
    transport: NodeTransport,
    negotiated_version: ProtocolVersion,
    capabilities_json: &str,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    let HubNodeMessage::Hello { capabilities, .. } = &hello.message else {
        return Err(HubNodeRailError::InvalidMessageDirection);
    };
    tx.execute(
        "INSERT INTO hub_node_connections (
             device_id, connection_id, transport, protocol_major,
             protocol_minor, status, connected_at_ms, last_seen_ms,
             updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6, ?6)
         ON CONFLICT(device_id) DO UPDATE SET
             connection_id = excluded.connection_id,
             transport = excluded.transport,
             protocol_major = excluded.protocol_major,
             protocol_minor = excluded.protocol_minor,
             status = 'active', connected_at_ms = excluded.connected_at_ms,
             last_seen_ms = excluded.last_seen_ms,
             updated_at_ms = excluded.updated_at_ms,
             disconnected_at_ms = NULL, last_error_code = NULL",
        params![
            hello.device_id,
            hello.connection_id,
            transport_str(transport),
            negotiated_version.major,
            negotiated_version.minor,
            now_ms,
        ],
    )?;
    tx.execute(
        "UPDATE captain_devices
         SET captain_version = ?2, protocol_major = ?3, protocol_minor = ?4,
             capabilities_json = ?5, last_transport = ?6,
             last_error_code = NULL, last_seen_ms = MAX(last_seen_ms, ?7),
             updated_at_ms = MAX(updated_at_ms, ?7)
         WHERE device_id = ?1 AND status = 'active' AND role = 'node'",
        params![
            hello.device_id,
            capabilities.captain_version,
            negotiated_version.major,
            negotiated_version.minor,
            capabilities_json,
            transport_str(transport),
            now_ms,
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_welcome_and_requeued(
    tx: &Transaction<'_>,
    device_id: &str,
    transport: NodeTransport,
    negotiated_version: ProtocolVersion,
    heartbeat_interval_ms: u64,
    lease_duration_ms: u64,
    pending_to_requeue: Vec<RequeuedOutboxMessage>,
    now_ms: i64,
) -> Result<HubNodeOutboxRecord, HubNodeRailError> {
    let welcome = append_outbox_in_tx(
        tx,
        device_id,
        None,
        &HubNodeMessage::Welcome {
            negotiated_version,
            transport,
            heartbeat_interval_ms,
            lease_duration_ms,
        },
        now_ms,
    )?;
    for pending in pending_to_requeue {
        append_outbox_in_tx(
            tx,
            device_id,
            pending.run_id.as_deref(),
            &pending.message,
            now_ms,
        )?;
    }
    Ok(welcome)
}

fn supersede_pending_outbox_for_reconnect(
    tx: &rusqlite::Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    active_run_ids: &[String],
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<Vec<RequeuedOutboxMessage>, HubNodeRailError> {
    let records = {
        let mut statement = tx.prepare(
            "SELECT device_id, sequence, message_kind, message_json,
                    message_sha256, run_id, created_at_ms, acked_at_ms,
                    superseded_at_ms
             FROM hub_node_outbox
             WHERE device_id = ?1 AND acked_at_ms IS NULL
               AND superseded_at_ms IS NULL
             ORDER BY sequence",
        )?;
        let records = statement
            .query_map([device_id], super::outbox_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        records
    };
    let mut pending = Vec::with_capacity(records.len());
    for record in &records {
        if record.message_sha256 != super::sha256_hex(record.message_json.as_bytes()) {
            return Err(HubNodeRailError::StorageInvariant);
        }
        let mut message: HubNodeMessage = serde_json::from_str(&record.message_json)
            .map_err(|_| HubNodeRailError::StorageInvariant)?;
        if record.message_kind != super::message_kind(&message) {
            return Err(HubNodeRailError::StorageInvariant);
        }
        let should_requeue = match &mut message {
            HubNodeMessage::Welcome { .. } if record.run_id.is_none() => false,
            HubNodeMessage::RunOffer(lease)
                if record.run_id.as_deref() == Some(lease.run_id.as_str()) =>
            {
                let run = super::run_by_id(tx, &lease.run_id)?
                    .ok_or(HubNodeRailError::StorageInvariant)?;
                if run.device_id != device_id || run.attempt != lease.attempt {
                    return Err(HubNodeRailError::StorageInvariant);
                }
                if run.status == super::HubNodeRunStatus::Leased {
                    let effective_expiry = run
                        .lease_expires_at_ms
                        .unwrap_or(lease_expires_at_ms)
                        .max(lease_expires_at_ms);
                    adopt_run_lease(
                        tx,
                        device_id,
                        &run.run_id,
                        connection_id,
                        effective_expiry,
                        now_ms,
                    )?;
                    lease.lease_expires_at_ms = effective_expiry;
                    true
                } else if matches!(
                    run.status,
                    super::HubNodeRunStatus::Accepted | super::HubNodeRunStatus::CancelRequested
                ) || run.status.is_terminal()
                    || run.status == super::HubNodeRunStatus::Queued
                {
                    false
                } else {
                    return Err(HubNodeRailError::StorageInvariant);
                }
            }
            HubNodeMessage::CancelRun {
                run_id, attempt, ..
            } if record.run_id.as_deref() == Some(run_id.as_str()) => {
                let run =
                    super::run_by_id(tx, run_id)?.ok_or(HubNodeRailError::StorageInvariant)?;
                if run.device_id != device_id || run.attempt != *attempt {
                    return Err(HubNodeRailError::StorageInvariant);
                }
                if run.status == super::HubNodeRunStatus::CancelRequested {
                    let effective_expiry = run
                        .lease_expires_at_ms
                        .unwrap_or(lease_expires_at_ms)
                        .max(lease_expires_at_ms);
                    adopt_run_lease(
                        tx,
                        device_id,
                        &run.run_id,
                        connection_id,
                        effective_expiry,
                        now_ms,
                    )?;
                    true
                } else if run.status.is_terminal() {
                    false
                } else {
                    return Err(HubNodeRailError::StorageInvariant);
                }
            }
            HubNodeMessage::ProtocolError { .. } if record.run_id.is_none() => true,
            _ => return Err(HubNodeRailError::StorageInvariant),
        };
        if should_requeue {
            pending.push(RequeuedOutboxMessage {
                run_id: record.run_id.clone(),
                message,
            });
        }
    }
    for run_id in active_run_ids {
        let run = super::run_by_id(tx, run_id)?.ok_or(HubNodeRailError::LeaseConflict)?;
        if run.device_id != device_id
            || !matches!(
                run.status,
                super::HubNodeRunStatus::Accepted | super::HubNodeRunStatus::CancelRequested
            )
        {
            return Err(HubNodeRailError::LeaseConflict);
        }
        let effective_expiry = run
            .lease_expires_at_ms
            .unwrap_or(lease_expires_at_ms)
            .max(lease_expires_at_ms);
        adopt_run_lease(
            tx,
            device_id,
            run_id,
            connection_id,
            effective_expiry,
            now_ms,
        )?;
    }
    if !records.is_empty() {
        tx.execute(
            "UPDATE hub_node_outbox
             SET superseded_at_ms = MAX(created_at_ms, ?2)
             WHERE device_id = ?1 AND acked_at_ms IS NULL
               AND superseded_at_ms IS NULL",
            params![device_id, now_ms],
        )?;
    }
    Ok(pending)
}

fn adopt_run_lease(
    tx: &rusqlite::Transaction<'_>,
    device_id: &str,
    run_id: &str,
    connection_id: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    let changed = tx.execute(
        "UPDATE hub_node_runs
         SET lease_owner = ?3, lease_expires_at_ms = ?4,
             updated_at_ms = MAX(updated_at_ms, ?5)
         WHERE device_id = ?1 AND run_id = ?2
           AND status IN ('leased', 'accepted', 'cancel_requested')",
        params![
            device_id,
            run_id,
            connection_id,
            lease_expires_at_ms,
            now_ms,
        ],
    )?;
    if changed != 1 {
        return Err(HubNodeRailError::LeaseConflict);
    }
    Ok(())
}

fn authorized_node(conn: &Connection, device_id: &str) -> Result<AuthorizedNode, HubNodeRailError> {
    conn.query_row(
        "SELECT platform, protocol_major, protocol_minor, grants_json
         FROM captain_devices
         WHERE device_id = ?1 AND role = 'node' AND status = 'active'",
        [device_id],
        |row| {
            let grants_json: String = row.get(3)?;
            let grants = serde_json::from_str(&grants_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(AuthorizedNode {
                platform: row.get(0)?,
                protocol_version: ProtocolVersion {
                    major: row.get(1)?,
                    minor: row.get(2)?,
                },
                grants,
            })
        },
    )
    .optional()?
    .ok_or(HubNodeRailError::NodeUnavailable)
}

fn connection_by_device(
    conn: &Connection,
    device_id: &str,
) -> Result<Option<HubNodeConnectionRecord>, HubNodeRailError> {
    conn.query_row(
        "SELECT device_id, connection_id, transport, protocol_major,
                protocol_minor, status, connected_at_ms, last_seen_ms,
                updated_at_ms, disconnected_at_ms, last_error_code
         FROM hub_node_connections WHERE device_id = ?1",
        [device_id],
        connection_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn connection_id_owner(
    conn: &Connection,
    connection_id: &str,
) -> Result<Option<String>, HubNodeRailError> {
    conn.query_row(
        "SELECT device_id FROM hub_node_connections WHERE connection_id = ?1",
        [connection_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn connection_from_row(row: &Row<'_>) -> rusqlite::Result<HubNodeConnectionRecord> {
    let transport: String = row.get(2)?;
    let status: String = row.get(5)?;
    Ok(HubNodeConnectionRecord {
        device_id: row.get(0)?,
        connection_id: row.get(1)?,
        transport: parse_transport(&transport).ok_or_else(|| invalid_column(2, "transport"))?,
        protocol_version: ProtocolVersion {
            major: row.get(3)?,
            minor: row.get(4)?,
        },
        status: HubNodeConnectionStatus::parse(&status)
            .ok_or_else(|| invalid_column(5, "connection status"))?,
        connected_at_ms: row.get(6)?,
        last_seen_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
        disconnected_at_ms: row.get(9)?,
        last_error_code: row.get(10)?,
    })
}

fn cursor_last_node_sequence(conn: &Connection, device_id: &str) -> Result<u64, HubNodeRailError> {
    let value: i64 = conn.query_row(
        "SELECT last_node_sequence FROM hub_node_cursors WHERE device_id = ?1",
        [device_id],
        |row| row.get(0),
    )?;
    u64::try_from(value).map_err(|_| HubNodeRailError::StorageInvariant)
}

fn transport_str(transport: NodeTransport) -> &'static str {
    match transport {
        NodeTransport::WebSocket => "web_socket",
        NodeTransport::HttpStream => "http_stream",
        NodeTransport::LongPoll => "long_poll",
    }
}

fn parse_transport(value: &str) -> Option<NodeTransport> {
    Some(match value {
        "web_socket" => NodeTransport::WebSocket,
        "http_stream" => NodeTransport::HttpStream,
        "long_poll" => NodeTransport::LongPoll,
        _ => return None,
    })
}

fn invalid_column(column: usize, name: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid {name}"),
        )),
    )
}
