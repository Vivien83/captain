//! Pure native-service definitions for the standalone Captain Node binary.

use std::path::{Path, PathBuf};
use thiserror::Error;

pub const NODE_LAUNCHD_LABEL: &str = "fr.captainagent.node";
pub const NODE_SYSTEMD_SERVICE: &str = "captain-node.service";
pub const NODE_WINDOWS_SERVICE: &str = "CaptainNode";
pub const NODE_WINDOWS_DISPLAY_NAME: &str = "Captain Node";

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NodeServiceDefinitionError {
    #[error("The service binary or Captain home path is invalid")]
    InvalidPath,
}

pub fn node_service_log_path(home: &Path) -> PathBuf {
    home.join("node").join("logs").join("captain-node.log")
}

pub fn launchd_plist_content(
    binary: &Path,
    home: &Path,
) -> Result<String, NodeServiceDefinitionError> {
    let binary = unix_service_path(binary)?;
    let home = unix_service_path(home)?;
    let log = unix_service_path(&node_service_log_path(Path::new(&home)))?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{NODE_LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>--home</string>
    <string>{}</string>
    <string>service-runtime</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>CAPTAIN_HOME</key>
    <string>{}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>5</integer>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        xml_escape(&binary),
        xml_escape(&home),
        xml_escape(&home),
        xml_escape(&home),
        xml_escape(&log),
        xml_escape(&log),
    ))
}

pub fn systemd_user_unit_content(
    binary: &Path,
    home: &Path,
) -> Result<String, NodeServiceDefinitionError> {
    let binary = unix_service_path(binary)?;
    let home = unix_service_path(home)?;
    Ok(format!(
        r#"[Unit]
Description=Captain Node outbound execution service
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=300
StartLimitBurst=10

[Service]
Type=simple
Environment="CAPTAIN_HOME={}"
ExecStart={} --home {} service-runtime
Restart=always
RestartSec=5
TimeoutStopSec=15
KillSignal=SIGTERM
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
WorkingDirectory={}

[Install]
WantedBy=default.target
"#,
        systemd_escape(&home),
        systemd_quote(&binary),
        systemd_quote(&home),
        systemd_quote(&home),
    ))
}

pub fn windows_service_bin_path(
    binary: &Path,
    home: &Path,
) -> Result<String, NodeServiceDefinitionError> {
    let binary = windows_service_path(binary)?;
    let home = windows_service_path(home)?;
    Ok(format!(
        "{} --home {} service-runtime",
        windows_quote(&binary),
        windows_quote(&home)
    ))
}

fn unix_service_path(path: &Path) -> Result<String, NodeServiceDefinitionError> {
    if !path.is_absolute() {
        return Err(NodeServiceDefinitionError::InvalidPath);
    }
    validated_path_text(path)
}

fn windows_service_path(path: &Path) -> Result<String, NodeServiceDefinitionError> {
    let value = validated_path_text(path)?;
    let bytes = value.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc_absolute = value.starts_with(r"\\") || value.starts_with("//");
    if !drive_absolute && !unc_absolute {
        return Err(NodeServiceDefinitionError::InvalidPath);
    }
    if value.contains('"') {
        return Err(NodeServiceDefinitionError::InvalidPath);
    }
    Ok(value)
}

fn validated_path_text(path: &Path) -> Result<String, NodeServiceDefinitionError> {
    let value = path
        .to_str()
        .ok_or(NodeServiceDefinitionError::InvalidPath)?;
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(NodeServiceDefinitionError::InvalidPath);
    }
    Ok(value.to_owned())
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_escape(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$")
}

fn systemd_quote(raw: &str) -> String {
    format!("\"{}\"", systemd_escape(raw))
}

fn windows_quote(raw: &str) -> String {
    let trailing_backslashes = raw.bytes().rev().take_while(|byte| *byte == b'\\').count();
    let mut quoted = String::with_capacity(raw.len() + trailing_backslashes + 2);
    quoted.push('"');
    quoted.push_str(raw);
    for _ in 0..trailing_backslashes {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_service_is_node_only_and_restarting() {
        let plist = launchd_plist_content(
            Path::new("/Applications/Captain & Node/captain-node"),
            Path::new("/Users/test/.captain"),
        )
        .unwrap();
        assert!(plist.contains(NODE_LAUNCHD_LABEL));
        assert!(plist.contains("Captain &amp; Node"));
        assert!(plist.contains("<string>service-runtime</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(plist.contains("captain-node.log"));
        assert!(!plist.contains("captain start"));
    }

    #[test]
    fn systemd_service_restarts_and_stops_cooperatively() {
        let unit = systemd_user_unit_content(
            Path::new("/home/test/Captain $Node/captain-node"),
            Path::new("/home/test/.captain"),
        )
        .unwrap();
        assert!(unit.contains(
            "ExecStart=\"/home/test/Captain $$Node/captain-node\" --home \"/home/test/.captain\" service-runtime"
        ));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("KillSignal=SIGTERM"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("UMask=0077"));
    }

    #[test]
    fn windows_service_command_quotes_binary_and_home_exactly() {
        let command = windows_service_bin_path(
            Path::new(r"C:\Program Files\Captain\captain-node.exe"),
            Path::new(r"C:\Users\Test User\.captain"),
        )
        .unwrap();
        assert_eq!(
            command,
            r#""C:\Program Files\Captain\captain-node.exe" --home "C:\Users\Test User\.captain" service-runtime"#
        );
    }

    #[test]
    fn service_definitions_reject_relative_and_control_character_paths() {
        assert!(launchd_plist_content(Path::new("captain-node"), Path::new("/tmp/home")).is_err());
        assert!(systemd_user_unit_content(
            Path::new("/tmp/captain-node\nother"),
            Path::new("/tmp/home")
        )
        .is_err());
        assert!(windows_service_bin_path(
            Path::new(r"Captain\captain-node.exe"),
            Path::new(r"C:\Users\test\.captain")
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn service_definitions_reject_non_utf8_paths_instead_of_rewriting_them() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let binary = Path::new(OsStr::from_bytes(b"/tmp/captain-node-\xff"));
        assert!(launchd_plist_content(binary, Path::new("/tmp/home")).is_err());
    }
}
