//! Local filesystem paths for inbound channel media.

use std::path::PathBuf;

/// Default inbound directory for files received via channels.
///
/// Mirrors the `captain_home_for_runtime` helper in captain-runtime to stay
/// consistent with the rest of the codebase: `$CAPTAIN_HOME` overrides,
/// otherwise `~/.captain/`.
pub(super) fn captain_inbound_dir(channel: &str) -> PathBuf {
    inbound_dir_for_env(
        channel,
        std::env::var("CAPTAIN_HOME").ok(),
        std::env::var("HOME").ok(),
    )
}

fn inbound_dir_for_env(
    channel: &str,
    captain_home: Option<String>,
    home: Option<String>,
) -> PathBuf {
    let base = if let Some(path) = captain_home {
        PathBuf::from(path)
    } else if let Some(home) = home {
        PathBuf::from(home).join(".captain")
    } else {
        PathBuf::from(".captain")
    };
    base.join("inbound").join(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_dir_prefers_captain_home() {
        assert_eq!(
            inbound_dir_for_env(
                "telegram",
                Some("/tmp/captain-home".to_string()),
                Some("/Users/example".to_string())
            ),
            PathBuf::from("/tmp/captain-home/inbound/telegram")
        );
    }

    #[test]
    fn inbound_dir_falls_back_to_home_captain_dir() {
        assert_eq!(
            inbound_dir_for_env("discord", None, Some("/Users/example".to_string())),
            PathBuf::from("/Users/example/.captain/inbound/discord")
        );
    }

    #[test]
    fn inbound_dir_uses_relative_captain_when_home_is_unknown() {
        assert_eq!(
            inbound_dir_for_env("signal", None, None),
            PathBuf::from(".captain/inbound/signal")
        );
    }
}
