//! Stable execution-location contract shared by Hub work surfaces.
//!
//! A target contains logical identifiers only. Local filesystem paths stay on
//! the Node and must never enter this contract or Hub persistence.

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_WORKSPACE_ID_LEN: usize = 128;

/// Where tools for one session or project should execute.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ExecutionTarget {
    /// Let Captain choose from the work context and currently available
    /// capabilities. Auto never means silent fallback from an explicit pin.
    #[default]
    Auto,
    /// Execute on the authoritative Hub.
    Hub,
    /// Execute on one paired Node inside one advertised logical workspace.
    Node {
        device_id: String,
        workspace_id: String,
    },
}

impl ExecutionTarget {
    pub fn validate(&self) -> Result<(), ExecutionTargetContractError> {
        match self {
            Self::Auto | Self::Hub => Ok(()),
            Self::Node {
                device_id,
                workspace_id,
            } => {
                validate_node_device_id(device_id)?;
                validate_workspace_id(workspace_id)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExecutionTargetContractError {
    #[error("invalid Node device identifier")]
    InvalidNodeDeviceId,
    #[error("invalid logical workspace identifier")]
    InvalidWorkspaceId,
}

fn validate_node_device_id(value: &str) -> Result<(), ExecutionTargetContractError> {
    let Some(uuid) = value.strip_prefix("node-") else {
        return Err(ExecutionTargetContractError::InvalidNodeDeviceId);
    };
    let parsed = uuid::Uuid::parse_str(uuid)
        .map_err(|_| ExecutionTargetContractError::InvalidNodeDeviceId)?;
    if format!("node-{parsed}") != value {
        return Err(ExecutionTargetContractError::InvalidNodeDeviceId);
    }
    Ok(())
}

fn validate_workspace_id(value: &str) -> Result<(), ExecutionTargetContractError> {
    if value.is_empty()
        || value.len() > MAX_WORKSPACE_ID_LEN
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ExecutionTargetContractError::InvalidWorkspaceId);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_and_hub_are_valid_without_remote_metadata() {
        assert_eq!(ExecutionTarget::default(), ExecutionTarget::Auto);
        ExecutionTarget::Auto.validate().unwrap();
        ExecutionTarget::Hub.validate().unwrap();
    }

    #[test]
    fn node_requires_canonical_device_and_logical_workspace_identifiers() {
        let device_id = format!("node-{}", uuid::Uuid::new_v4());
        ExecutionTarget::Node {
            device_id: device_id.clone(),
            workspace_id: "workspace-main_1".to_string(),
        }
        .validate()
        .unwrap();

        for target in [
            ExecutionTarget::Node {
                device_id: device_id.to_uppercase(),
                workspace_id: "workspace-main".to_string(),
            },
            ExecutionTarget::Node {
                device_id: "client-00000000-0000-0000-0000-000000000000".to_string(),
                workspace_id: "workspace-main".to_string(),
            },
            ExecutionTarget::Node {
                device_id,
                workspace_id: "/Users/private/workspace".to_string(),
            },
        ] {
            assert!(target.validate().is_err());
        }
    }

    #[test]
    fn serde_contract_is_tagged_and_never_contains_a_path_field() {
        let target = ExecutionTarget::Node {
            device_id: "node-00000000-0000-0000-0000-000000000001".to_string(),
            workspace_id: "main".to_string(),
        };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["kind"], "node");
        assert_eq!(json["workspace_id"], "main");
        assert!(json.get("path").is_none());
        assert_eq!(
            serde_json::from_value::<ExecutionTarget>(json).unwrap(),
            target
        );
        assert!(
            serde_json::from_value::<ExecutionTarget>(serde_json::json!({
                "kind": "node",
                "device_id": "node-00000000-0000-0000-0000-000000000001",
                "workspace_id": "main",
                "path": "/private/workspace"
            }))
            .is_err()
        );
    }
}
