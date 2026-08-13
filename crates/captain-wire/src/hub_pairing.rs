//! Stable HTTP messages used to pair Clients and Nodes with a Captain Hub.

use crate::hub_protocol::{DeviceGrant, ProtocolVersion, HUB_NODE_PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

pub const PAIRING_CLAIM_PATH: &str = "/api/hub/pairing/claim";
pub const PAIRING_POLL_PATH: &str = "/api/hub/pairing/poll";
pub const DEVICE_TOKEN_PATH: &str = "/api/hub/devices/token";

const RAW_SECRET_HEX_LEN: usize = 64;
const DISPLAY_CODE_LEN: usize = 9;
const DISPLAY_CODE_SEPARATOR_INDEX: usize = 4;

/// One-time challenge returned after a device submits a validated claim.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingChallenge {
    pub request_id: String,
    pub display_code: String,
    pub polling_secret: String,
    pub expires_at_ms: i64,
    pub approval_path: String,
    pub protocol_version: ProtocolVersion,
}

impl PairingChallenge {
    pub fn validate(&self, now_ms: i64) -> Result<(), PairingContractError> {
        validate_request_id(&self.request_id)?;
        validate_display_code(&self.display_code)?;
        validate_raw_secret("polling_secret", &self.polling_secret)?;
        if self.expires_at_ms <= now_ms {
            return Err(PairingContractError::InvalidExpiry);
        }
        let expected_path = format!("/devices/pair?code={}", self.display_code);
        if self.approval_path != expected_path {
            return Err(PairingContractError::InvalidApprovalPath);
        }
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        Ok(())
    }
}

impl fmt::Debug for PairingChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingChallenge")
            .field("request_id", &self.request_id)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("approval_path", &"[REDACTED]")
            .field("protocol_version", &self.protocol_version)
            .finish_non_exhaustive()
    }
}

/// Poll request authenticated by the one-time secret returned in the challenge.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPollRequest {
    pub request_id: String,
    pub polling_secret: String,
}

impl PairingPollRequest {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        validate_request_id(&self.request_id)?;
        validate_raw_secret("polling_secret", &self.polling_secret)
    }
}

impl fmt::Debug for PairingPollRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingPollRequest")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPollResponse {
    pub status: PairingState,
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_grants: Option<DeviceGrant>,
    pub expires_at_ms: i64,
}

impl fmt::Debug for PairingPollResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingPollResponse")
            .field("status", &self.status)
            .field("device_id", &self.device_id)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}

impl PairingPollResponse {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        if self.expires_at_ms <= 0 {
            return Err(PairingContractError::InvalidExpiry);
        }
        match self.status {
            PairingState::Approved => {
                validate_device_id(
                    self.device_id
                        .as_deref()
                        .ok_or(PairingContractError::InvalidPairingState)?,
                )?;
                self.approved_grants
                    .as_ref()
                    .ok_or(PairingContractError::InvalidPairingState)?
                    .validate_shape()?;
            }
            PairingState::Pending | PairingState::Denied | PairingState::Expired => {
                if self.device_id.is_some() || self.approved_grants.is_some() {
                    return Err(PairingContractError::InvalidPairingState);
                }
            }
        }
        Ok(())
    }
}

/// Long-lived bootstrap credential exchange. The credential is never logged or
/// persisted in plaintext by the Hub.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCredentialExchange {
    pub device_id: String,
    pub credential: String,
}

impl DeviceCredentialExchange {
    pub fn validate(&self) -> Result<(), PairingContractError> {
        validate_device_id(&self.device_id)?;
        validate_raw_secret("credential", &self.credential)
    }
}

impl fmt::Debug for DeviceCredentialExchange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceCredentialExchange")
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}

/// Short-lived in-memory bearer issued after a successful credential exchange.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAccessToken {
    pub access_token: String,
    pub token_type: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub protocol_version: ProtocolVersion,
    #[serde(default)]
    pub approved_grants: DeviceGrant,
}

impl DeviceAccessToken {
    pub fn validate(&self, now_ms: i64) -> Result<(), PairingContractError> {
        validate_raw_secret("access_token", &self.access_token)?;
        if self.token_type != "Bearer" {
            return Err(PairingContractError::InvalidTokenType);
        }
        if self.issued_at_ms > now_ms
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms <= now_ms
        {
            return Err(PairingContractError::InvalidExpiry);
        }
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        self.approved_grants.validate_shape()?;
        Ok(())
    }
}

impl fmt::Debug for DeviceAccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceAccessToken")
            .field("token_type", &self.token_type)
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("protocol_version", &self.protocol_version)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PairingContractError {
    #[error("request_id must be a UUID")]
    InvalidRequestId,
    #[error("device_id is invalid")]
    InvalidDeviceId,
    #[error("display_code is invalid")]
    InvalidDisplayCode,
    #[error("{0} must be a 32-byte lowercase hexadecimal secret")]
    InvalidSecret(&'static str),
    #[error("pairing expiry is invalid")]
    InvalidExpiry,
    #[error("approval_path is invalid")]
    InvalidApprovalPath,
    #[error("token_type must be Bearer")]
    InvalidTokenType,
    #[error("pairing response state is inconsistent")]
    InvalidPairingState,
    #[error(transparent)]
    Protocol(#[from] crate::hub_protocol::ProtocolContractError),
}

fn validate_request_id(value: &str) -> Result<(), PairingContractError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| PairingContractError::InvalidRequestId)
}

fn validate_device_id(value: &str) -> Result<(), PairingContractError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'));
    if valid {
        Ok(())
    } else {
        Err(PairingContractError::InvalidDeviceId)
    }
}

fn validate_display_code(value: &str) -> Result<(), PairingContractError> {
    let valid = value.len() == DISPLAY_CODE_LEN
        && value.as_bytes().get(DISPLAY_CODE_SEPARATOR_INDEX) == Some(&b'-')
        && value.bytes().enumerate().all(|(index, byte)| {
            index == DISPLAY_CODE_SEPARATOR_INDEX
                || matches!(byte, b'2'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z')
        });
    if valid {
        Ok(())
    } else {
        Err(PairingContractError::InvalidDisplayCode)
    }
}

fn validate_raw_secret(field: &'static str, value: &str) -> Result<(), PairingContractError> {
    let valid = value.len() == RAW_SECRET_HEX_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(PairingContractError::InvalidSecret(field))
    }
}

#[cfg(test)]
#[path = "hub_pairing_tests.rs"]
mod tests;
