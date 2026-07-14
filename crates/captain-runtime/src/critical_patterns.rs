//! Hyper-critical command patterns — system-level destructive operations.
//!
//! Curated subset of `default_blocked_commands` (see `captain-types::config`)
//! that represents truly catastrophic actions: data destruction at the disk
//! level, full database drops, fork bombs, and irreversible git pushes to
//! protected branches.
//!
//! These patterns are matched **before** shell execution and trigger the
//! one-shot approval modal in `Open` mode (see `CriticalMode`).
//!
//! Stays narrowly scoped on purpose: the broader `blocked_commands` list
//! handles all the other dangerous operations — this module is the
//! "stop the world" set.

/// Hyper-critical patterns. If a `shell_exec` command contains any of
/// these as a substring, it is considered catastrophic.
pub const CRITICAL_PATTERNS: &[&str] = &[
    // Data destruction at root
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf $HOME",
    "rm -rf --no-preserve-root",
    // Disk-level wipes
    "dd if=",
    "dd of=/dev/",
    "mkfs",
    "wipefs",
    // Database catastrophes
    "DROP DATABASE",
    "DROP SCHEMA",
    "TRUNCATE TABLE",
    // Fork bomb (and its no-space variant)
    ":(){ :|:&};:",
    ":(){:|:&};:",
    // Permission catastrophes
    "chmod -R 777 /",
    // Force-push to protected branches
    "git push --force origin main",
    "git push --force origin master",
    "git push -f origin main",
    "git push -f origin master",
];

/// Returns the matched pattern if `command` contains any critical substring,
/// `None` otherwise. Case-sensitive on purpose — the patterns are exact
/// because attackers rarely uppercase a `rm` and we want zero false positives.
pub fn is_critical(command: &str) -> Option<&'static str> {
    CRITICAL_PATTERNS
        .iter()
        .copied()
        .find(|p| command.contains(p))
}

/// Decision for a critical command, based on the active mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriticalDecision {
    /// Not a critical command — proceed without prompting.
    Proceed,
    /// Critical — ask the user (modal one-shot via `kh.request_approval`).
    AskUser(&'static str),
    /// Critical and the active mode forbids it without exception — block.
    Block(&'static str),
}

pub use captain_types::config::CriticalMode;

/// Decide what to do with a `shell_exec` command under the given mode.
pub fn decide(command: &str, mode: CriticalMode) -> CriticalDecision {
    match (is_critical(command), mode) {
        (None, CriticalMode::Paranoid) => CriticalDecision::AskUser("paranoid_shell"),
        (None, _) => CriticalDecision::Proceed,
        (Some(p), CriticalMode::Open) => CriticalDecision::AskUser(p),
        (Some(p), CriticalMode::Safe) => CriticalDecision::Block(p),
        (Some(p), CriticalMode::Paranoid) => CriticalDecision::Block(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rm_rf_root() {
        assert_eq!(is_critical("rm -rf /"), Some("rm -rf /"));
        assert_eq!(is_critical("sudo rm -rf / --quiet"), Some("rm -rf /"));
    }

    #[test]
    fn matches_dd_to_device() {
        assert!(is_critical("dd of=/dev/sda bs=1M").is_some());
    }

    #[test]
    fn matches_drop_database() {
        assert_eq!(
            is_critical("psql -c 'DROP DATABASE prod;'"),
            Some("DROP DATABASE")
        );
    }

    #[test]
    fn matches_fork_bomb_both_variants() {
        assert!(is_critical("bash -c ':(){ :|:&};:'").is_some());
        assert!(is_critical("bash -c ':(){:|:&};:'").is_some());
    }

    #[test]
    fn matches_force_push_main() {
        assert!(is_critical("git push --force origin main").is_some());
        assert!(is_critical("git push -f origin master").is_some());
    }

    #[test]
    fn safe_command_is_not_critical() {
        assert_eq!(is_critical("ls -la"), None);
        assert_eq!(is_critical("rm file.txt"), None); // rm without -rf /
        assert_eq!(is_critical("git push origin feature"), None);
        assert_eq!(is_critical("dd help"), None); // dd without if= or of=/dev/
    }

    #[test]
    fn open_mode_asks_user_on_critical() {
        match decide("rm -rf /", CriticalMode::Open) {
            CriticalDecision::AskUser(p) => assert_eq!(p, "rm -rf /"),
            other => panic!("expected AskUser, got: {other:?}"),
        }
    }

    #[test]
    fn open_mode_proceeds_on_safe_command() {
        assert_eq!(decide("ls", CriticalMode::Open), CriticalDecision::Proceed);
    }

    #[test]
    fn safe_mode_blocks_critical_outright() {
        match decide("DROP DATABASE prod", CriticalMode::Safe) {
            CriticalDecision::Block(p) => assert_eq!(p, "DROP DATABASE"),
            other => panic!("expected Block, got: {other:?}"),
        }
    }

    #[test]
    fn safe_mode_proceeds_on_safe_command() {
        assert_eq!(decide("ls", CriticalMode::Safe), CriticalDecision::Proceed);
    }

    #[test]
    fn paranoid_blocks_critical_and_asks_for_rest() {
        assert!(matches!(
            decide("rm -rf /", CriticalMode::Paranoid),
            CriticalDecision::Block(_)
        ));
        match decide("ls", CriticalMode::Paranoid) {
            CriticalDecision::AskUser(reason) => assert_eq!(reason, "paranoid_shell"),
            other => panic!("expected AskUser(paranoid_shell), got: {other:?}"),
        }
    }

    #[test]
    fn critical_mode_default_is_open() {
        assert_eq!(CriticalMode::default(), CriticalMode::Open);
    }

    #[test]
    fn critical_mode_serializes_lowercase() {
        let s = serde_json::to_string(&CriticalMode::Safe).unwrap();
        assert_eq!(s, "\"safe\"");
        let m: CriticalMode = serde_json::from_str("\"open\"").unwrap();
        assert_eq!(m, CriticalMode::Open);
    }
}
