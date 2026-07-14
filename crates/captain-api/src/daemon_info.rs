use captain_types::version::captain_version;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Daemon info written to `~/.captain/daemon.json` so the CLI can find us.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub listen_addr: String,
    pub started_at: String,
    pub version: String,
    pub platform: String,
}

pub(crate) fn write_daemon_info_file(
    info_path: &Path,
    addr: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    if info_path.exists() {
        if let Ok(existing) = std::fs::read_to_string(info_path) {
            if let Ok(info) = serde_json::from_str::<DaemonInfo>(&existing) {
                if is_process_alive(info.pid) && is_daemon_responding(&info.listen_addr) {
                    return Err(format!(
                        "Another daemon (PID {}) is already running at {}",
                        info.pid, info.listen_addr
                    )
                    .into());
                }
            }
        }
        info!("Removing stale daemon info file");
        let _ = std::fs::remove_file(info_path);
    }

    let daemon_info = DaemonInfo {
        pid: std::process::id(),
        listen_addr: addr.to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
        version: captain_version(),
        platform: std::env::consts::OS.to_string(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&daemon_info) {
        let _ = std::fs::write(info_path, json);
        restrict_permissions(info_path);
    }
    Ok(())
}

pub(crate) fn remove_daemon_info_file(info_path: &Path) {
    let _ = std::fs::remove_file(info_path);
}

/// Read daemon info from the standard location.
pub fn read_daemon_info(home_dir: &Path) -> Option<DaemonInfo> {
    let info_path = home_dir.join("daemon.json");
    let contents = std::fs::read_to_string(info_path).ok()?;
    serde_json::from_str(&contents).ok()
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| {
                o.status.success() && {
                    let out = String::from_utf8_lossy(&o.stdout);
                    !out.contains("INFO:") && out.contains(&pid.to_string())
                }
            })
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

fn is_daemon_responding(addr: &str) -> bool {
    let addr_only = addr
        .strip_prefix("http://")
        .or_else(|| addr.strip_prefix("https://"))
        .unwrap_or(addr);
    if let Ok(sock_addr) = addr_only.parse::<SocketAddr>() {
        std::net::TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_millis(500))
            .is_ok()
    } else {
        std::net::TcpStream::connect(addr_only)
            .map(|_| true)
            .unwrap_or(false)
    }
}
