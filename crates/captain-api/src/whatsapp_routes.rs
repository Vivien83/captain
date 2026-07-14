//! Frozen WhatsApp route handlers.

use axum::response::IntoResponse;

/// POST /api/channels/whatsapp/qr/start - refuse frozen WhatsApp Web QR setup.
pub async fn whatsapp_qr_start() -> impl IntoResponse {
    crate::channel_routes::frozen_channel_response("whatsapp", "error")
}

/// GET /api/channels/whatsapp/qr/status - refuse frozen WhatsApp Web QR polling.
pub async fn whatsapp_qr_status() -> impl IntoResponse {
    crate::channel_routes::frozen_channel_response("whatsapp", "error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn qr_start_is_frozen() {
        let response = whatsapp_qr_start().await.into_response();
        let status = response.status();
        let body = response_json(response).await;

        assert_eq!(status, StatusCode::GONE);
        assert_eq!(body["channel"], "whatsapp");
        assert_eq!(body["state"], "frozen");
        assert_eq!(
            body["active_channels"],
            serde_json::json!(["telegram", "discord", "signal", "email"])
        );
    }

    #[tokio::test]
    async fn qr_status_is_frozen() {
        let response = whatsapp_qr_status().await.into_response();
        let status = response.status();
        let body = response_json(response).await;

        assert_eq!(status, StatusCode::GONE);
        assert_eq!(body["channel"], "whatsapp");
        assert_eq!(body["state"], "frozen");
    }
}
