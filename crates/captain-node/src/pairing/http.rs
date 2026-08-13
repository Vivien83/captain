//! Bounded, redacted HTTP bootstrap calls for Node pairing.

use super::NodePairingError;
use crate::network::NodeHttpClient;
use captain_wire::{
    DeviceAccessToken, DeviceCredentialExchange, DevicePairingClaim, PairingChallenge,
    PairingPollRequest, PairingPollResponse, DEVICE_TOKEN_PATH, PAIRING_CLAIM_PATH,
    PAIRING_POLL_PATH,
};
use futures::StreamExt;
use reqwest::{header::RETRY_AFTER, Response, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zeroize::Zeroizing;

const MAX_HUB_RESPONSE_BYTES: usize = 128 * 1024;

impl NodeHttpClient {
    pub(super) async fn submit_pairing_claim(
        &self,
        claim: &DevicePairingClaim,
    ) -> Result<PairingChallenge, NodePairingError> {
        self.post_json(
            self.endpoint(PAIRING_CLAIM_PATH)?,
            claim,
            StatusCode::CREATED,
        )
        .await
    }

    pub(super) async fn poll_pairing(
        &self,
        request: &PairingPollRequest,
    ) -> Result<PairingPollResponse, NodePairingError> {
        self.post_json(self.endpoint(PAIRING_POLL_PATH)?, request, StatusCode::OK)
            .await
    }

    pub(super) async fn exchange_credential(
        &self,
        request: &DeviceCredentialExchange,
    ) -> Result<DeviceAccessToken, NodePairingError> {
        self.post_json(self.endpoint(DEVICE_TOKEN_PATH)?, request, StatusCode::OK)
            .await
    }

    fn endpoint(&self, path: &str) -> Result<Url, NodePairingError> {
        let mut origin = self.endpoints.connect.clone();
        origin.set_path("/");
        origin.set_query(None);
        origin.set_fragment(None);
        origin
            .join(path.trim_start_matches('/'))
            .map_err(|_| NodePairingError::InvalidHubResponse)
    }

    async fn post_json<T: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        endpoint: Url,
        request: &T,
        expected_status: StatusCode,
    ) -> Result<R, NodePairingError> {
        let response = tokio::time::timeout(
            self.request_timeout,
            self.client.post(endpoint).json(request).send(),
        )
        .await
        .map_err(|_| NodePairingError::RequestTimedOut)?
        .map_err(|error| {
            if error.is_timeout() {
                NodePairingError::RequestTimedOut
            } else {
                NodePairingError::NetworkUnavailable
            }
        })?;
        let status = response.status();
        let retry_after_secs = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let body = tokio::time::timeout(self.request_timeout, bounded_response_body(response))
            .await
            .map_err(|_| NodePairingError::RequestTimedOut)??;
        if status != expected_status {
            return Err(classify_hub_error(status, retry_after_secs, &body));
        }
        serde_json::from_slice(&body).map_err(|_| NodePairingError::InvalidHubResponse)
    }
}

async fn bounded_response_body(response: Response) -> Result<Zeroizing<Vec<u8>>, NodePairingError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HUB_RESPONSE_BYTES as u64)
    {
        return Err(NodePairingError::HubResponseTooLarge);
    }
    let mut body = Zeroizing::new(Vec::new());
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| NodePairingError::NetworkUnavailable)?;
        if body.len().saturating_add(chunk.len()) > MAX_HUB_RESPONSE_BYTES {
            return Err(NodePairingError::HubResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct HubErrorEnvelope {
    error: HubErrorBody,
}

#[derive(Deserialize)]
struct HubErrorBody {
    code: String,
}

fn classify_hub_error(
    status: StatusCode,
    retry_after_secs: Option<u64>,
    body: &[u8],
) -> NodePairingError {
    let code = serde_json::from_slice::<HubErrorEnvelope>(body)
        .ok()
        .map(|error| sanitize_error_code(&error.error.code))
        .unwrap_or_else(|| "unknown".to_string());
    match code.as_str() {
        "pairing_enrollment_closed" => NodePairingError::EnrollmentClosed,
        "pairing_disabled" => NodePairingError::PairingDisabled,
        "credential_already_claimed" | "pairing_state_conflict" => {
            NodePairingError::CredentialConflict
        }
        "pairing_rate_limited" | "too_many_pairing_requests" => NodePairingError::RateLimited {
            retry_after_secs: retry_after_secs.unwrap_or(1),
        },
        "pairing_expired" => NodePairingError::PairingExpired,
        "invalid_polling_credential" => NodePairingError::InvalidPollingCredential,
        "invalid_device_credential" => NodePairingError::InvalidDeviceCredential,
        "pairing_storage_unavailable" => NodePairingError::HubUnavailable,
        _ if status == StatusCode::TOO_MANY_REQUESTS => NodePairingError::RateLimited {
            retry_after_secs: retry_after_secs.unwrap_or(1),
        },
        _ if status == StatusCode::SERVICE_UNAVAILABLE => NodePairingError::HubUnavailable,
        _ => NodePairingError::HubRejected {
            status: status.as_u16(),
            code,
        },
    }
}

fn sanitize_error_code(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
pub(super) const TEST_MAX_HUB_RESPONSE_BYTES: usize = MAX_HUB_RESPONSE_BYTES;

#[cfg(test)]
mod tests {
    use super::*;

    fn error_body(code: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"error": {"code": code}})).unwrap()
    }

    #[test]
    fn hub_errors_are_categorical_and_remote_codes_are_sanitized() {
        assert_eq!(
            classify_hub_error(
                StatusCode::FORBIDDEN,
                None,
                &error_body("pairing_enrollment_closed"),
            ),
            NodePairingError::EnrollmentClosed
        );
        assert_eq!(
            classify_hub_error(
                StatusCode::TOO_MANY_REQUESTS,
                Some(17),
                &error_body("too_many_pairing_requests"),
            ),
            NodePairingError::RateLimited {
                retry_after_secs: 17,
            }
        );
        assert_eq!(
            classify_hub_error(
                StatusCode::BAD_GATEWAY,
                None,
                &error_body("unsafe\nremote detail"),
            ),
            NodePairingError::HubRejected {
                status: 502,
                code: "unknown".to_string(),
            }
        );
    }
}
