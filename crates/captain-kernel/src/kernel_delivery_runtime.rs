use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;

use super::CaptainKernel;

/// Deliver a cron response to a channel, retrying if the channel is not yet
/// registered or the delivery target reports a transient failure.
async fn deliver_with_retry(
    kernel: &CaptainKernel,
    channel: &str,
    recipient: &str,
    response: &str,
) -> Result<(), crate::delivery_reliability::DeliveryFailure> {
    let target = crate::delivery_reliability::channel_target(channel, recipient);
    for attempt in 1..=crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS {
        match kernel
            .send_channel_message(channel, recipient, response, None)
            .await
        {
            Ok(_) => {
                tracing::info!(channel = %channel, recipient = %recipient, attempt, "Cron: delivered to channel");
                return Ok(());
            }
            Err(e) => {
                let err_str = e.to_string();
                let is_retryable =
                    crate::delivery_reliability::is_retryable_delivery_error(&err_str);
                if !is_retryable
                    || attempt == crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS
                {
                    return Err(crate::delivery_reliability::DeliveryFailure::new(
                        target, err_str, attempt,
                    ));
                }
                let delay_ms = crate::delivery_reliability::jittered_backoff_delay_ms(attempt);
                tracing::warn!(
                    channel = %channel,
                    attempt,
                    next_delay_ms = delay_ms,
                    "Cron channel delivery failed transiently, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }
    Err(crate::delivery_reliability::DeliveryFailure::new(
        target,
        format!("channel '{channel}' not available after retries"),
        crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS,
    ))
}

pub(crate) async fn cron_deliver_response(
    kernel: &CaptainKernel,
    agent_id: AgentId,
    response: &str,
    delivery: &captain_types::scheduler::CronDelivery,
) -> Result<(), crate::delivery_reliability::DeliveryFailure> {
    use captain_types::scheduler::CronDelivery;

    if response.is_empty() {
        return Ok(());
    }

    match delivery {
        CronDelivery::None => Ok(()),
        CronDelivery::Channel { channel, to } => {
            tracing::debug!(channel = %channel, to = %to, "Cron: delivering to channel");
            let kv_val = serde_json::json!({"channel": channel, "recipient": to});
            let _ = kernel
                .memory
                .structured_set(agent_id, "delivery.last_channel", kv_val);
            deliver_with_retry(kernel, channel, to, response)
                .await
                .inspect_err(|e| {
                    tracing::warn!(channel = %channel, to = %to, error = %e, "Cron channel delivery failed");
                })
        }
        CronDelivery::LastChannel => {
            match kernel
                .memory
                .structured_get(agent_id, "delivery.last_channel")
            {
                Ok(Some(val)) => {
                    let channel = val["channel"].as_str().unwrap_or("");
                    let recipient = val["recipient"].as_str().unwrap_or("");
                    if !channel.is_empty() && !recipient.is_empty() {
                        deliver_with_retry(kernel, channel, recipient, response)
                            .await
                            .inspect_err(|e| {
                                tracing::warn!(channel = %channel, recipient = %recipient, error = %e, "Cron last-channel delivery failed");
                            })
                    } else {
                        Ok(())
                    }
                }
                _ => {
                    tracing::debug!("Cron: no last channel found for agent {}", agent_id);
                    Ok(())
                }
            }
        }
        CronDelivery::Webhook { url } => {
            captain_runtime::web_fetch::check_ssrf(url).map_err(|e| {
                tracing::warn!(url = %url, error = %e, "Cron webhook blocked by SSRF guard");
                crate::delivery_reliability::DeliveryFailure::new(
                    crate::delivery_reliability::webhook_target(url),
                    format!("webhook blocked by SSRF guard: {e}"),
                    1,
                )
            })?;
            tracing::debug!(url = %url, "Cron: delivering via webhook");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| {
                    crate::delivery_reliability::DeliveryFailure::new(
                        crate::delivery_reliability::webhook_target(url),
                        format!("webhook client init failed: {e}"),
                        1,
                    )
                })?;
            let payload = serde_json::json!({
                "agent_id": agent_id.to_string(),
                "response": response,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            deliver_webhook_with_retry(&client, url, &payload).await
        }
    }
}

pub(crate) async fn retry_due_cron_deliveries(kernel: &CaptainKernel) {
    let due = kernel.cron_scheduler.due_redeliveries();
    if due.is_empty() {
        return;
    }

    for queued in due {
        let payload = match kernel.cron_scheduler.read_redelivery_payload(&queued) {
            Ok(payload) => payload,
            Err(e) => {
                let failure =
                    crate::delivery_reliability::DeliveryFailure::new(queued.target.clone(), e, 1);
                kernel.cron_scheduler.record_redelivery_failure(
                    queued.job_id,
                    &queued.id,
                    &failure,
                    "",
                );
                continue;
            }
        };

        match cron_deliver_response(kernel, queued.agent_id, &payload, &queued.delivery).await {
            Ok(()) => {
                tracing::info!(
                    job_id = %queued.job_id,
                    redelivery_id = %queued.id,
                    target = %queued.target,
                    "Cron redelivery succeeded"
                );
                kernel
                    .cron_scheduler
                    .record_redelivery_success(queued.job_id, &queued.id);
            }
            Err(e) => {
                tracing::warn!(
                    job_id = %queued.job_id,
                    redelivery_id = %queued.id,
                    target = %queued.target,
                    error = %e,
                    "Cron redelivery failed"
                );
                kernel.cron_scheduler.record_redelivery_failure(
                    queued.job_id,
                    &queued.id,
                    &e,
                    &payload,
                );
            }
        }
    }

    if let Err(e) = kernel.cron_scheduler.persist() {
        tracing::warn!("Cron redelivery persist failed: {e}");
    }
}

async fn deliver_webhook_with_retry(
    client: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
) -> Result<(), crate::delivery_reliability::DeliveryFailure> {
    let target = crate::delivery_reliability::webhook_target(url);
    for attempt in 1..=crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS {
        match client.post(url).json(payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(status = %resp.status(), attempt, "Cron webhook delivered");
                return Ok(());
            }
            Ok(resp) => {
                let status = resp.status();
                let err = format!("webhook returned HTTP {status}");
                let retryable = status.as_u16() == 429 || status.is_server_error();
                if !retryable
                    || attempt == crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS
                {
                    return Err(crate::delivery_reliability::DeliveryFailure::new(
                        target, err, attempt,
                    ));
                }
            }
            Err(e) => {
                let err = format!("webhook delivery failed: {e}");
                let retryable = crate::delivery_reliability::is_retryable_delivery_error(&err);
                if !retryable
                    || attempt == crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS
                {
                    return Err(crate::delivery_reliability::DeliveryFailure::new(
                        target, err, attempt,
                    ));
                }
            }
        }
        let delay_ms = crate::delivery_reliability::jittered_backoff_delay_ms(attempt);
        tracing::warn!(
            url = %url,
            attempt,
            next_delay_ms = delay_ms,
            "Cron webhook delivery retrying"
        );
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    Err(crate::delivery_reliability::DeliveryFailure::new(
        target,
        "webhook delivery failed after retries",
        crate::delivery_reliability::DEFAULT_MAX_DELIVERY_ATTEMPTS,
    ))
}
