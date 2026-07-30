use super::{
    AuditAction, AuditEntry, AuditEpoch, AuditError, EpochState, CURRENT_HASH_VERSION,
    LEGACY_HASH_VERSION,
};
use sha2::{Digest, Sha256};

pub(super) fn build_entry(
    seq: u64,
    epoch: u64,
    timestamp: String,
    agent_id: String,
    action: AuditAction,
    detail: String,
    outcome: String,
    prev_hash: String,
) -> Result<AuditEntry, AuditError> {
    let mut entry = AuditEntry {
        seq,
        epoch,
        hash_version: CURRENT_HASH_VERSION,
        timestamp,
        agent_id,
        action,
        detail,
        outcome,
        prev_hash,
        hash: String::new(),
    };
    entry.hash = compute_entry_hash(&entry).map_err(AuditError::InvalidSchema)?;
    Ok(entry)
}

pub(super) fn compute_entry_hash(entry: &AuditEntry) -> Result<String, String> {
    match entry.hash_version {
        LEGACY_HASH_VERSION => Ok(compute_legacy_entry_hash(entry)),
        CURRENT_HASH_VERSION => Ok(compute_injective_entry_hash(entry)),
        version => Err(format!(
            "unsupported audit hash version {version} at seq {}",
            entry.seq
        )),
    }
}

fn compute_legacy_entry_hash(entry: &AuditEntry) -> String {
    let mut hasher = Sha256::new();
    hasher.update(entry.seq.to_string().as_bytes());
    hasher.update(entry.timestamp.as_bytes());
    hasher.update(entry.agent_id.as_bytes());
    hasher.update(entry.action.to_string().as_bytes());
    hasher.update(entry.detail.as_bytes());
    hasher.update(entry.outcome.as_bytes());
    hasher.update(entry.prev_hash.as_bytes());
    hex::encode(hasher.finalize())
}

fn compute_injective_entry_hash(entry: &AuditEntry) -> String {
    let mut hasher = Sha256::new();
    for field in [
        entry.hash_version.to_be_bytes().as_slice(),
        entry.epoch.to_be_bytes().as_slice(),
        entry.seq.to_be_bytes().as_slice(),
        entry.timestamp.as_bytes(),
        entry.agent_id.as_bytes(),
        entry.action.to_string().as_bytes(),
        entry.detail.as_bytes(),
        entry.outcome.as_bytes(),
        entry.prev_hash.as_bytes(),
    ] {
        hash_length_prefixed(&mut hasher, field);
    }
    hex::encode(hasher.finalize())
}

fn hash_length_prefixed(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field);
}

pub(super) fn verify_epoch(entries: &[AuditEntry], epoch: &AuditEpoch) -> Result<(), String> {
    if epoch.started_at.trim().is_empty() {
        return Err(format!("epoch {} has no start timestamp", epoch.id));
    }
    if epoch.state == EpochState::Active {
        for entry in entries {
            if entry.epoch == epoch.id && entry.seq < epoch.start_seq {
                return Err(format!(
                    "entry seq {} precedes active epoch {} start seq {}",
                    entry.seq, epoch.id, epoch.start_seq
                ));
            }
            if entry.seq >= epoch.start_seq && entry.epoch != epoch.id {
                return Err(format!(
                    "entry seq {} belongs to epoch {} after active epoch {} started",
                    entry.seq, entry.epoch, epoch.id
                ));
            }
        }
    }
    let mut expected_seq = epoch.start_seq;
    let mut expected_prev = epoch.predecessor_tip_hash.clone();
    let mut seen = 0usize;

    for entry in entries.iter().filter(|entry| entry.epoch == epoch.id) {
        if entry.seq != expected_seq {
            return Err(format!(
                "sequence break in epoch {}: expected {} but found {}",
                epoch.id, expected_seq, entry.seq
            ));
        }
        if entry.prev_hash != expected_prev {
            return Err(format!(
                "chain break in epoch {} at seq {}",
                epoch.id, entry.seq
            ));
        }
        let recomputed = compute_entry_hash(entry)?;
        if recomputed != entry.hash {
            return Err(format!(
                "hash mismatch in epoch {} at seq {}",
                epoch.id, entry.seq
            ));
        }
        if seen == 0 && epoch.id > 0 && entry.action != AuditAction::ChainRecovery {
            return Err(format!(
                "epoch {} does not start with ChainRecovery",
                epoch.id
            ));
        }
        expected_prev.clone_from(&entry.hash);
        expected_seq = expected_seq
            .checked_add(1)
            .ok_or_else(|| "audit sequence exhausted".to_string())?;
        seen += 1;
    }

    if epoch.id > 0 && seen == 0 {
        return Err(format!("recovery epoch {} is empty", epoch.id));
    }
    if epoch.state == EpochState::Active && epoch.terminal_hash.is_some() {
        return Err(format!(
            "active epoch {} unexpectedly has a terminal hash",
            epoch.id
        ));
    }
    Ok(())
}

pub(super) fn unique_active_epoch(epochs: &[AuditEpoch]) -> Result<u64, AuditError> {
    let active = epochs
        .iter()
        .filter(|epoch| epoch.state == EpochState::Active)
        .map(|epoch| epoch.id)
        .collect::<Vec<_>>();
    match active.as_slice() {
        [id] => Ok(*id),
        _ => Err(AuditError::InvalidSchema(format!(
            "expected exactly one active audit epoch, found {}",
            active.len()
        ))),
    }
}

pub(super) fn epoch_by_id(epochs: &[AuditEpoch], id: u64) -> Result<&AuditEpoch, AuditError> {
    epochs
        .iter()
        .find(|epoch| epoch.id == id)
        .ok_or_else(|| AuditError::InvalidSchema(format!("audit epoch {id} is missing")))
}

pub(super) fn epoch_tip(entries: &[AuditEntry], epoch: &AuditEpoch) -> String {
    entries
        .iter()
        .rev()
        .find(|entry| entry.epoch == epoch.id)
        .map(|entry| entry.hash.clone())
        .unwrap_or_else(|| epoch.predecessor_tip_hash.clone())
}

pub(super) fn next_sequence(entries: &[AuditEntry]) -> Result<u64, AuditError> {
    match entries.last() {
        Some(entry) => entry
            .seq
            .checked_add(1)
            .ok_or(AuditError::SequenceExhausted),
        None => Ok(0),
    }
}

pub(super) fn invalid_epoch_ids(epochs: &[AuditEpoch]) -> Vec<u64> {
    epochs
        .iter()
        .filter(|epoch| epoch.state == EpochState::Invalid)
        .map(|epoch| epoch.id)
        .collect()
}
