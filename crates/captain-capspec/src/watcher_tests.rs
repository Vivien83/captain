use super::*;
use notify_debouncer_mini::notify::PollWatcher;
use std::fs;
use tempfile::TempDir;

fn source(version: &str) -> String {
    format!(
        r#"format = 1
name = "live-reader"
description = "Read a file after hot reload version {version}."
version = "{version}"

[permissions]
tools = ["file_read"]
read_paths = ["/tmp/**"]

[[steps]]
id = "read"
tool = "file_read"
with = {{ path = "/tmp/input.txt" }}
"#
    )
}

#[test]
fn owned_watcher_reloads_valid_edit_without_restart() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("capabilities");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("live-reader.captain");
    fs::write(&path, source("1.0.0")).unwrap();
    let registry =
        Arc::new(CapabilityRegistry::open(&root, &temp.path().join("capabilities.db")).unwrap());
    let watcher = CapabilityWatcher::<PollWatcher>::new_with_backend(
        Arc::clone(&registry),
        60,
        NotifyConfig::default()
            .with_poll_interval(Duration::from_millis(30))
            .with_compare_contents(true),
    )
    .unwrap();

    std::thread::sleep(Duration::from_millis(100));
    fs::write(&path, source("2.0.0")).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let active = registry
            .active_by_tool("cap_live_reader", None)
            .unwrap()
            .unwrap();
        if active.version == "2.0.0" {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "hot reload timed out");
        std::thread::sleep(Duration::from_millis(30));
    }

    let status = watcher.status().unwrap();
    assert_eq!(status.watched_roots.len(), 1);
    assert!(status.successful_reloads >= 1);
    assert_eq!(status.failed_reloads, 0);
}
