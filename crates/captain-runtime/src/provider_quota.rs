//! Provider-owned subscription quota discovery.
//!
//! These values come from provider responses. They are never inferred from
//! Captain's local token ledger, which is a separate safety mechanism.

use crate::drivers::codex::{extract_chatgpt_account_id, CODEX_ORIGINATOR, CODEX_UA};
use crate::llm_driver::LlmError;
use captain_types::quota::{
    ProviderCreditsSnapshot, ProviderQuotaSnapshot, ProviderQuotaSource, ProviderQuotaWindow,
};
use chrono::{DateTime, Duration, Utc};
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Callback used by drivers to publish provider quota observations.
pub type ProviderQuotaObserver = Arc<dyn Fn(ProviderQuotaSnapshot) + Send + Sync>;

/// Parse all Codex limit families exposed in HTTP response headers.
pub fn parse_codex_rate_limit_headers(
    headers: &HeaderMap,
    source: ProviderQuotaSource,
) -> Vec<ProviderQuotaSnapshot> {
    let mut header_limit_ids = BTreeSet::new();
    for name in headers.keys() {
        let name = name.as_str().to_ascii_lowercase();
        let Some(prefix) = name.strip_suffix("-primary-used-percent") else {
            continue;
        };
        let Some(limit_id) = prefix.strip_prefix("x-") else {
            continue;
        };
        header_limit_ids.insert(limit_id.to_string());
    }

    let default_has_metadata = [
        "x-codex-primary-used-percent",
        "x-codex-secondary-used-percent",
        "x-codex-credits-has-credits",
        "x-codex-rate-limit-reached-type",
    ]
    .iter()
    .any(|name| headers.contains_key(*name));
    if default_has_metadata {
        header_limit_ids.insert("codex".to_string());
    }

    header_limit_ids
        .into_iter()
        .filter_map(|header_limit_id| {
            let prefix = format!("x-{header_limit_id}");
            let primary = parse_header_window(headers, &prefix, "primary");
            let secondary = parse_header_window(headers, &prefix, "secondary");
            let credits = parse_header_credits(headers);
            let limit_name = header_string(headers, &format!("{prefix}-limit-name"));
            let reached = header_string(headers, "x-codex-rate-limit-reached-type");
            if primary.is_none()
                && secondary.is_none()
                && credits.is_none()
                && limit_name.is_none()
                && reached.is_none()
            {
                return None;
            }
            Some(ProviderQuotaSnapshot {
                provider: "codex".to_string(),
                limit_id: normalize_limit_id(&header_limit_id),
                limit_name,
                primary,
                secondary,
                credits,
                plan_type: None,
                rate_limit_reached_type: reached,
                source,
                observed_at: Utc::now(),
            })
        })
        .collect()
}

/// Parse the official `codex.rate_limits` SSE payload.
pub fn parse_codex_rate_limit_event(value: &Value) -> Option<ProviderQuotaSnapshot> {
    if value.get("type").and_then(Value::as_str) != Some("codex.rate_limits") {
        return None;
    }
    let limits = value.get("rate_limits");
    let primary = limits
        .and_then(|data| data.get("primary"))
        .and_then(parse_event_window);
    let secondary = limits
        .and_then(|data| data.get("secondary"))
        .and_then(parse_event_window);
    let credits = value.get("credits").and_then(parse_credits);
    let raw_limit_id = value
        .get("metered_limit_name")
        .or_else(|| value.get("limit_name"))
        .and_then(Value::as_str)
        .unwrap_or("codex");
    Some(ProviderQuotaSnapshot {
        provider: "codex".to_string(),
        limit_id: normalize_limit_id(raw_limit_id),
        limit_name: value
            .get("limit_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        primary,
        secondary,
        credits,
        plan_type: value.get("plan_type").and_then(value_as_text),
        rate_limit_reached_type: value.get("rate_limit_reached_type").and_then(value_as_text),
        source: ProviderQuotaSource::StreamEvent,
        observed_at: Utc::now(),
    })
}

/// Parse the official Codex account usage response (`wham/usage`).
pub fn parse_codex_usage_payload(value: &Value) -> Vec<ProviderQuotaSnapshot> {
    let plan_type = value.get("plan_type").and_then(value_as_text);
    let credits = value.get("credits").and_then(parse_credits);
    let reached_type = value
        .get("rate_limit_reached_type")
        .and_then(parse_reached_type);
    let mut snapshots = Vec::new();

    if let Some(rate_limit) = value.get("rate_limit") {
        snapshots.push(snapshot_from_usage_status(
            "codex",
            Some("Codex".to_string()),
            rate_limit,
            credits.clone(),
            plan_type.clone(),
            reached_type.clone(),
        ));
    } else if credits.is_some() || plan_type.is_some() {
        snapshots.push(ProviderQuotaSnapshot {
            provider: "codex".to_string(),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: None,
            secondary: None,
            credits: credits.clone(),
            plan_type: plan_type.clone(),
            rate_limit_reached_type: reached_type.clone(),
            source: ProviderQuotaSource::AccountStatus,
            observed_at: Utc::now(),
        });
    }

    if let Some(additional) = value
        .get("additional_rate_limits")
        .and_then(Value::as_array)
    {
        for limit in additional {
            let Some(rate_limit) = limit.get("rate_limit").filter(|value| !value.is_null()) else {
                continue;
            };
            let limit_name = limit
                .get("limit_name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let limit_id = limit
                .get("metered_feature")
                .or_else(|| limit.get("metered_limit_name"))
                .or_else(|| limit.get("limit_name"))
                .and_then(Value::as_str)
                .unwrap_or("codex_additional");
            snapshots.push(snapshot_from_usage_status(
                limit_id,
                limit_name,
                rate_limit,
                None,
                plan_type.clone(),
                None,
            ));
        }
    }

    snapshots
}

/// Fetch subscription limits from Codex's account endpoint.
pub async fn fetch_codex_subscription_quotas(
    access_token: &str,
    base_url: &str,
) -> Result<Vec<ProviderQuotaSnapshot>, LlmError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| LlmError::Http(error.to_string()))?;
    let mut request = client
        .get(codex_usage_endpoint(base_url))
        .bearer_auth(access_token)
        .header("User-Agent", CODEX_UA)
        .header("originator", CODEX_ORIGINATOR)
        .header("Accept", "application/json");
    if let Some(account_id) = extract_chatgpt_account_id(access_token) {
        request = request.header("ChatGPT-Account-ID", account_id);
    }
    let response = request
        .send()
        .await
        .map_err(|error| LlmError::Http(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| LlmError::Http(error.to_string()))?;
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => LlmError::AuthenticationFailed(format!(
                "Codex usage status rejected the current session (HTTP {}).",
                status.as_u16()
            )),
            code => LlmError::Api {
                status: code,
                message: truncate(&body, 500),
            },
        });
    }
    let payload: Value =
        serde_json::from_str(&body).map_err(|error| LlmError::Parse(error.to_string()))?;
    Ok(parse_codex_usage_payload(&payload))
}

/// Official account endpoint corresponding to Captain's Codex Responses URL.
pub fn codex_usage_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if let Some(root) = base.strip_suffix("/backend-api/codex") {
        return format!("{root}/backend-api/wham/usage");
    }
    if base.ends_with("/api/codex") {
        return format!("{base}/usage");
    }
    format!("{base}/usage")
}

fn snapshot_from_usage_status(
    limit_id: &str,
    limit_name: Option<String>,
    status: &Value,
    credits: Option<ProviderCreditsSnapshot>,
    plan_type: Option<String>,
    inherited_reached_type: Option<String>,
) -> ProviderQuotaSnapshot {
    let limit_reached = status
        .get("limit_reached")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || status
            .get("allowed")
            .and_then(Value::as_bool)
            .is_some_and(|allowed| !allowed);
    ProviderQuotaSnapshot {
        provider: "codex".to_string(),
        limit_id: normalize_limit_id(limit_id),
        limit_name,
        primary: status.get("primary_window").and_then(parse_usage_window),
        secondary: status.get("secondary_window").and_then(parse_usage_window),
        credits,
        plan_type,
        rate_limit_reached_type: inherited_reached_type
            .or_else(|| limit_reached.then(|| "provider_reported".to_string())),
        source: ProviderQuotaSource::AccountStatus,
        observed_at: Utc::now(),
    }
}

fn parse_header_window(
    headers: &HeaderMap,
    prefix: &str,
    window: &str,
) -> Option<ProviderQuotaWindow> {
    let used_percent = header_f64(headers, &format!("{prefix}-{window}-used-percent"))?;
    let window_seconds = header_i64(headers, &format!("{prefix}-{window}-window-minutes"))
        .and_then(|minutes| u64::try_from(minutes).ok())
        .map(|minutes| minutes.saturating_mul(60));
    let reset_after_seconds =
        header_i64(headers, &format!("{prefix}-{window}-reset-after-seconds"))
            .and_then(|seconds| u64::try_from(seconds).ok());
    let resets_at = header_i64(headers, &format!("{prefix}-{window}-reset-at"))
        .and_then(unix_timestamp)
        .or_else(|| {
            reset_after_seconds.map(|seconds| Utc::now() + Duration::seconds(seconds as i64))
        });
    Some(ProviderQuotaWindow {
        used_percent,
        window_seconds,
        reset_after_seconds: reset_after_seconds.or_else(|| resets_at.map(seconds_until)),
        resets_at,
    })
}

fn parse_usage_window(value: &Value) -> Option<ProviderQuotaWindow> {
    let used_percent = value.get("used_percent")?.as_f64()?;
    let window_seconds = value.get("limit_window_seconds").and_then(Value::as_u64);
    let reset_after_seconds = value.get("reset_after_seconds").and_then(Value::as_u64);
    let resets_at = value
        .get("reset_at")
        .and_then(Value::as_i64)
        .and_then(unix_timestamp)
        .or_else(|| {
            reset_after_seconds.map(|seconds| Utc::now() + Duration::seconds(seconds as i64))
        });
    Some(ProviderQuotaWindow {
        used_percent,
        window_seconds,
        reset_after_seconds,
        resets_at,
    })
}

fn parse_event_window(value: &Value) -> Option<ProviderQuotaWindow> {
    let used_percent = value.get("used_percent")?.as_f64()?;
    let window_seconds = value
        .get("window_minutes")
        .and_then(Value::as_i64)
        .and_then(|minutes| u64::try_from(minutes).ok())
        .map(|minutes| minutes.saturating_mul(60));
    let resets_at = value
        .get("reset_at")
        .and_then(Value::as_i64)
        .and_then(unix_timestamp);
    Some(ProviderQuotaWindow {
        used_percent,
        window_seconds,
        reset_after_seconds: resets_at.map(seconds_until),
        resets_at,
    })
}

fn parse_header_credits(headers: &HeaderMap) -> Option<ProviderCreditsSnapshot> {
    Some(ProviderCreditsSnapshot {
        has_credits: header_bool(headers, "x-codex-credits-has-credits")?,
        unlimited: header_bool(headers, "x-codex-credits-unlimited")?,
        balance: header_string(headers, "x-codex-credits-balance"),
    })
}

fn parse_credits(value: &Value) -> Option<ProviderCreditsSnapshot> {
    Some(ProviderCreditsSnapshot {
        has_credits: value.get("has_credits")?.as_bool()?,
        unlimited: value.get("unlimited")?.as_bool()?,
        balance: value.get("balance").and_then(value_as_text),
    })
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn header_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
    header_string(headers, name)?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    header_string(headers, name)?.parse::<i64>().ok()
}

fn header_bool(headers: &HeaderMap, name: &str) -> Option<bool> {
    match header_string(headers, name)?.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn value_as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_reached_type(value: &Value) -> Option<String> {
    value_as_text(value).or_else(|| value.get("type").and_then(value_as_text))
}

fn normalize_limit_id(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn unix_timestamp(value: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(value, 0)
}

fn seconds_until(timestamp: DateTime<Utc>) -> u64 {
    (timestamp - Utc::now()).num_seconds().max(0) as u64
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};
    use serde_json::json;

    fn header(headers: &mut HeaderMap, name: &'static str, value: &'static str) {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }

    #[test]
    fn parses_dynamic_codex_header_families() {
        let mut headers = HeaderMap::new();
        header(&mut headers, "x-codex-primary-used-percent", "12.5");
        header(&mut headers, "x-codex-primary-window-minutes", "300");
        header(&mut headers, "x-codex-primary-reset-at", "2000000000");
        header(&mut headers, "x-codex-bengalfox-primary-used-percent", "91");
        header(
            &mut headers,
            "x-codex-bengalfox-primary-window-minutes",
            "10080",
        );
        header(&mut headers, "x-codex-bengalfox-limit-name", "gpt-5-codex");
        header(&mut headers, "x-codex-credits-has-credits", "true");
        header(&mut headers, "x-codex-credits-unlimited", "false");
        header(&mut headers, "x-codex-credits-balance", "17.50");

        let snapshots =
            parse_codex_rate_limit_headers(&headers, ProviderQuotaSource::ResponseHeaders);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].limit_id, "codex");
        assert_eq!(
            snapshots[0].primary.as_ref().unwrap().window_seconds,
            Some(18_000)
        );
        assert_eq!(snapshots[1].limit_id, "codex_bengalfox");
        assert_eq!(
            snapshots[1].alert_level(),
            captain_types::quota::QuotaAlertLevel::Critical
        );
        assert_eq!(snapshots[1].limit_name.as_deref(), Some("gpt-5-codex"));
    }

    #[test]
    fn parses_account_status_primary_weekly_and_additional_limits() {
        let payload = json!({
            "plan_type": "plus",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 25.0,
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 900,
                    "reset_at": 2000000000
                },
                "secondary_window": {
                    "used_percent": 75.0,
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": 300000,
                    "reset_at": 2000500000
                }
            },
            "credits": {"has_credits": true, "unlimited": false, "balance": "4.2"},
            "rate_limit_reached_type": {"type": "workspace_member_usage_limit_reached"},
            "additional_rate_limits": [{
                "limit_name": "Sonic",
                "metered_feature": "codex_sonic",
                "rate_limit": {
                    "allowed": false,
                    "limit_reached": true,
                    "primary_window": {
                        "used_percent": 100.0,
                        "limit_window_seconds": 3600,
                        "reset_after_seconds": 60,
                        "reset_at": 2000000000
                    }
                }
            }]
        });

        let snapshots = parse_codex_usage_payload(&payload);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(
            snapshots[0].secondary.as_ref().unwrap().window_seconds,
            Some(604_800)
        );
        assert_eq!(snapshots[0].plan_type.as_deref(), Some("plus"));
        assert_eq!(
            snapshots[0].rate_limit_reached_type.as_deref(),
            Some("workspace_member_usage_limit_reached")
        );
        assert_eq!(snapshots[1].limit_id, "codex_sonic");
        assert_eq!(
            snapshots[1].alert_level(),
            captain_types::quota::QuotaAlertLevel::Exhausted
        );
    }

    #[test]
    fn parses_official_sse_event_shape() {
        let payload = json!({
            "type": "codex.rate_limits",
            "plan_type": "pro",
            "rate_limits": {
                "primary": {"used_percent": 33.0, "window_minutes": 300, "reset_at": 2000000000},
                "secondary": {"used_percent": 44.0, "window_minutes": 10080, "reset_at": 2000500000}
            },
            "credits": {"has_credits": true, "unlimited": true, "balance": null}
        });
        let snapshot = parse_codex_rate_limit_event(&payload).unwrap();
        assert_eq!(snapshot.limit_id, "codex");
        assert_eq!(snapshot.primary.unwrap().window_seconds, Some(18_000));
        assert_eq!(snapshot.secondary.unwrap().window_seconds, Some(604_800));
    }

    #[test]
    fn derives_official_chatgpt_usage_endpoint() {
        assert_eq!(
            codex_usage_endpoint("https://chatgpt.com/backend-api/codex"),
            "https://chatgpt.com/backend-api/wham/usage"
        );
    }
}
