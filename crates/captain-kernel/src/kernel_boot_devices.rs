use crate::pairing::{PairedDevice, PairingManager, PersistOp};
use captain_memory::MemorySubstrate;
use captain_runtime::browser::BrowserManager;
use captain_runtime::media_understanding::MediaEngine;
use captain_runtime::tts::TtsEngine;
use captain_types::config::KernelConfig;
use std::sync::Arc;
use tracing::warn;

pub(super) struct BootDevices {
    pub(super) browser_ctx: BrowserManager,
    pub(super) media_engine: MediaEngine,
    pub(super) tts_engine: TtsEngine,
    pub(super) pairing: PairingManager,
}

pub(super) fn build_boot_devices(
    config: &KernelConfig,
    memory: &Arc<MemorySubstrate>,
) -> BootDevices {
    let browser_ctx = BrowserManager::new(config.browser.clone());
    let media_engine = MediaEngine::new(config.media.clone());
    let tts_engine = TtsEngine::new(config.tts.clone());
    let mut pairing = PairingManager::new(config.pairing.clone());

    if config.pairing.enabled {
        match memory.load_paired_devices() {
            Ok(rows) => pairing.load_devices(paired_devices_from_rows(rows)),
            Err(e) => {
                warn!("Failed to load paired devices from database: {e}");
            }
        }

        let persist_memory = Arc::clone(memory);
        pairing.set_persist(Box::new(move |device, op| match op {
            PersistOp::Save => {
                if let Err(e) = persist_memory.save_paired_device(
                    &device.device_id,
                    &device.display_name,
                    &device.platform,
                    &device.paired_at.to_rfc3339(),
                    &device.last_seen.to_rfc3339(),
                    device.push_token.as_deref(),
                ) {
                    tracing::warn!("Failed to persist paired device: {e}");
                }
            }
            PersistOp::Remove => {
                if let Err(e) = persist_memory.remove_paired_device(&device.device_id) {
                    tracing::warn!("Failed to remove paired device from DB: {e}");
                }
            }
        }));
    }

    BootDevices {
        browser_ctx,
        media_engine,
        tts_engine,
        pairing,
    }
}

fn paired_devices_from_rows(rows: Vec<serde_json::Value>) -> Vec<PairedDevice> {
    rows.into_iter()
        .filter_map(|row| {
            Some(PairedDevice {
                device_id: row["device_id"].as_str()?.to_string(),
                display_name: row["display_name"].as_str()?.to_string(),
                platform: row["platform"].as_str()?.to_string(),
                paired_at: chrono::DateTime::parse_from_rfc3339(row["paired_at"].as_str()?)
                    .ok()?
                    .with_timezone(&chrono::Utc),
                last_seen: chrono::DateTime::parse_from_rfc3339(row["last_seen"].as_str()?)
                    .ok()?
                    .with_timezone(&chrono::Utc),
                push_token: row["push_token"].as_str().map(String::from),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paired_devices_from_rows_keeps_valid_rows_and_skips_invalid_rows() {
        let rows = vec![
            json!({
                "device_id": "device-1",
                "display_name": "Phone",
                "platform": "ios",
                "paired_at": "2026-06-01T06:00:00Z",
                "last_seen": "2026-06-01T07:30:00Z",
                "push_token": "push-token"
            }),
            json!({
                "device_id": "broken",
                "display_name": "Broken",
                "platform": "ios",
                "paired_at": "not-a-date",
                "last_seen": "2026-06-01T07:30:00Z",
                "push_token": null
            }),
        ];

        let devices = paired_devices_from_rows(rows);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id, "device-1");
        assert_eq!(devices[0].display_name, "Phone");
        assert_eq!(devices[0].platform, "ios");
        assert_eq!(devices[0].push_token.as_deref(), Some("push-token"));
        assert_eq!(
            devices[0].paired_at.to_rfc3339(),
            "2026-06-01T06:00:00+00:00"
        );
        assert_eq!(
            devices[0].last_seen.to_rfc3339(),
            "2026-06-01T07:30:00+00:00"
        );
    }
}
