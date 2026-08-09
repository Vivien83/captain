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

/// Stable operator-facing reasons for hyper-critical command classes.
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

/// Returns the matched critical class after quote-aware command segmentation,
/// shell word parsing, case folding, whitespace normalization, and common
/// wrapper handling.
///
/// This is an accident-prevention heuristic. Shell is dynamic, so this
/// classifier is not an operating-system isolation boundary.
pub fn is_critical(command: &str) -> Option<&'static str> {
    is_critical_inner(command, 0)
}

/// Classify a direct program invocation without flattening its argument
/// boundaries into shell text. Shell wrappers such as `sh -c <payload>` still
/// recurse into the payload, while ordinary program arguments remain data.
pub(crate) fn is_critical_program(executable: &str, args: &[String]) -> Option<&'static str> {
    let mut words = Vec::with_capacity(args.len().saturating_add(1));
    words.push(executable.to_string());
    words.extend(args.iter().cloned());
    classify_words(&words, 0)
}

/// Match a configurable blocklist entry after lexical normalization.
///
/// The check preserves the existing blocklist contract while removing trivial
/// bypasses based on case, repeated whitespace, or reordered combined short
/// flags such as `-fr` versus `-rf`.
pub(crate) fn matches_blocked_pattern(command: &str, pattern: &str) -> bool {
    let pattern_tokens = normalized_tokens(pattern);
    if pattern_tokens.is_empty() {
        return false;
    }

    if command_segments(command).iter().any(|segment| {
        let command_tokens = normalized_tokens(segment);
        command_tokens
            .windows(pattern_tokens.len())
            .any(|window| window == pattern_tokens.as_slice())
    }) {
        return true;
    }

    normalized_text(command).contains(&normalized_text(pattern))
}

fn is_critical_inner(command: &str, depth: usize) -> Option<&'static str> {
    if depth > 4 {
        return None;
    }

    for segment in command_segments(command) {
        let Some(words) = shell_words(&segment) else {
            continue;
        };
        if let Some(reason) = classify_words(&words, depth) {
            return Some(reason);
        }
    }

    for payload in shell_substitution_payloads(command) {
        if let Some(reason) = is_critical_inner(&payload, depth.saturating_add(1)) {
            return Some(reason);
        }
    }

    let normalized = normalized_text(command);
    if normalized.contains("drop database") {
        return Some("DROP DATABASE");
    }
    if normalized.contains("drop schema") {
        return Some("DROP SCHEMA");
    }
    if normalized.contains("truncate table") {
        return Some("TRUNCATE TABLE");
    }

    let compact: String = command
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if compact.contains(":(){:|:&};:") {
        return Some(":(){:|:&};:");
    }

    None
}

fn classify_words(words: &[String], depth: usize) -> Option<&'static str> {
    let (executable, args) = effective_command(words)?;
    let executable = executable_basename(executable);

    match executable {
        "rm" if destructive_rm(args) => Some("rm -rf /"),
        "dd" if args.iter().any(|arg| {
            let arg = arg.to_ascii_lowercase();
            arg.starts_with("if=") || arg.starts_with("of=/dev/")
        }) =>
        {
            if args
                .iter()
                .any(|arg| arg.to_ascii_lowercase().starts_with("of=/dev/"))
            {
                Some("dd of=/dev/")
            } else {
                Some("dd if=")
            }
        }
        name if name == "mkfs" || name.starts_with("mkfs.") => Some("mkfs"),
        "wipefs" => Some("wipefs"),
        "chmod" if destructive_chmod(args) => Some("chmod -R 777 /"),
        "git" if destructive_git_push(args) => Some("git push --force origin main"),
        "bash" | "sh" | "zsh" | "dash" => shell_payload(args)
            .and_then(|payload| is_critical_inner(payload, depth.saturating_add(1))),
        "eval" => {
            let payload = args.join(" ");
            is_critical_inner(&payload, depth.saturating_add(1))
        }
        _ => None,
    }
}

fn effective_command(words: &[String]) -> Option<(&str, &[String])> {
    let mut index = 0;

    loop {
        while index < words.len() && is_assignment(&words[index]) {
            index += 1;
        }
        let executable = executable_basename(words.get(index)?);
        match executable {
            "sudo" => {
                index += 1;
                index = skip_wrapper_options(
                    words,
                    index,
                    &[
                        "-u",
                        "--user",
                        "-g",
                        "--group",
                        "-h",
                        "--host",
                        "-p",
                        "--prompt",
                        "-c",
                        "--close-from",
                    ],
                );
            }
            "env" => {
                index += 1;
                index = skip_wrapper_options(
                    words,
                    index,
                    &["-u", "--unset", "-c", "--chdir", "-s", "--split-string"],
                );
            }
            "command" | "exec" | "nohup" | "setsid" | "time" => {
                index += 1;
                while index < words.len() && words[index].starts_with('-') {
                    index += 1;
                }
            }
            "nice" => {
                index += 1;
                index = skip_wrapper_options(words, index, &["-n", "--adjustment"]);
            }
            _ => break,
        }
    }

    let executable = words.get(index)?;
    Some((executable, &words[index.saturating_add(1)..]))
}

fn skip_wrapper_options(words: &[String], mut index: usize, options_with_value: &[&str]) -> usize {
    while index < words.len() {
        let option = words[index].as_str();
        if option == "--" {
            return index.saturating_add(1);
        }
        if !option.starts_with('-') {
            break;
        }
        let consumes_next = options_with_value.contains(&option);
        index = index.saturating_add(if consumes_next { 2 } else { 1 });
    }
    index.min(words.len())
}

fn executable_basename(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value).trim()
}

fn is_assignment(value: &str) -> bool {
    let Some((name, _)) = value.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().enumerate().all(|(index, character)| {
            character == '_'
                || character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit())
        })
}

fn destructive_rm(args: &[String]) -> bool {
    let mut recursive = false;
    let mut force = false;
    let mut targets = Vec::new();
    let mut options_finished = false;

    for argument in args {
        let lowered = argument.to_ascii_lowercase();
        if !options_finished && lowered == "--" {
            options_finished = true;
            continue;
        }
        if !options_finished && lowered.starts_with("--") {
            recursive |= lowered == "--recursive";
            force |= lowered == "--force";
            continue;
        }
        if !options_finished && lowered.starts_with('-') && !lowered.starts_with("--") {
            recursive |= lowered.chars().skip(1).any(|flag| matches!(flag, 'r'));
            force |= lowered.chars().skip(1).any(|flag| flag == 'f');
            continue;
        }
        targets.push(lowered);
    }

    recursive
        && force
        && targets
            .iter()
            .any(|target| root_like_target(target) || home_like_target(target))
}

fn destructive_chmod(args: &[String]) -> bool {
    let recursive = args.iter().any(|arg| {
        let lowered = arg.to_ascii_lowercase();
        lowered == "--recursive"
            || lowered.starts_with('-') && lowered.chars().skip(1).any(|flag| flag == 'r')
    });
    let world_writable = args.iter().any(|arg| arg == "777");
    recursive && world_writable && args.iter().any(|target| root_like_target(target))
}

fn destructive_git_push(args: &[String]) -> bool {
    let Some(push_index) = args.iter().position(|arg| arg.eq_ignore_ascii_case("push")) else {
        return false;
    };
    let push_args = &args[push_index.saturating_add(1)..];
    let forced = push_args.iter().any(|arg| {
        arg == "-f"
            || arg.eq_ignore_ascii_case("--force")
            || arg.to_ascii_lowercase().starts_with("--force=")
            || arg.to_ascii_lowercase().starts_with("--force-with-lease")
            || arg.starts_with('+')
    });
    let protected_branch = push_args.iter().any(|arg| {
        let lowered = arg.trim_start_matches('+').to_ascii_lowercase();
        let destination = lowered
            .rsplit_once(':')
            .map(|(_, destination)| destination)
            .unwrap_or(lowered.as_str())
            .trim_start_matches("refs/heads/");
        destination == "main" || destination == "master"
    });
    forced && protected_branch
}

fn shell_payload(args: &[String]) -> Option<&str> {
    args.iter().enumerate().find_map(|(index, arg)| {
        (arg == "-c" || arg.ends_with('c') && arg.starts_with('-'))
            .then(|| args.get(index.saturating_add(1)).map(String::as_str))
            .flatten()
    })
}

fn root_like_target(target: &str) -> bool {
    let target = target.trim_matches(['"', '\'']);
    if !target.starts_with('/') {
        return false;
    }
    let target = target.trim_end_matches('*');
    target
        .split('/')
        .filter(|component| !component.is_empty())
        .all(|component| matches!(component, "." | ".."))
}

fn home_like_target(target: &str) -> bool {
    let normalized = target
        .trim_matches(['"', '\''])
        .trim_end_matches('*')
        .trim_end_matches('/');
    matches!(normalized, "~" | "$home" | "${home}")
}

fn normalized_tokens(value: &str) -> Vec<String> {
    command_segments(value)
        .into_iter()
        .flat_map(|segment| shell_words(&segment).unwrap_or_default())
        .map(|token| normalize_token(&token))
        .filter(|token| !token.is_empty())
        .collect()
}

fn normalize_token(token: &str) -> String {
    let lowered = token.to_ascii_lowercase();
    if lowered.starts_with('-')
        && !lowered.starts_with("--")
        && lowered.len() > 2
        && lowered
            .chars()
            .skip(1)
            .all(|character| character.is_ascii_alphabetic())
    {
        let mut flags: Vec<char> = lowered.chars().skip(1).collect();
        flags.sort_unstable();
        return format!("-{}", flags.into_iter().collect::<String>());
    }
    lowered
}

fn normalized_text(value: &str) -> String {
    value
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_words(segment: &str) -> Option<Vec<String>> {
    let words = shlex::split(segment)?;
    Some(
        words
            .into_iter()
            .map(|word| word.to_ascii_lowercase())
            .collect(),
    )
}

fn command_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    while let Some(character) = chars.next() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && !in_single_quote {
            if chars.peek() == Some(&'\n') {
                chars.next();
                current.push(' ');
                continue;
            }
            current.push(character);
            escaped = true;
            continue;
        }
        if character == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            current.push(character);
            continue;
        }
        if character == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            current.push(character);
            continue;
        }
        if !in_single_quote
            && !in_double_quote
            && matches!(character, ';' | '|' | '&' | '\n' | '\r' | '`' | '(' | ')')
        {
            if !current.trim().is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(character);
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
}

fn shell_substitution_payloads(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut payloads = Vec::new();
    let mut index = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while index < chars.len() {
        let character = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if character == '\\' && !in_single_quote {
            escaped = true;
            index += 1;
            continue;
        }
        if character == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            index += 1;
            continue;
        }
        if character == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            index += 1;
            continue;
        }
        if in_single_quote {
            index += 1;
            continue;
        }
        if character == '`' {
            if let Some((payload, next_index)) = backtick_payload(&chars, index + 1) {
                payloads.push(payload);
                index = next_index;
                continue;
            }
        }
        if character == '$' && chars.get(index + 1) == Some(&'(') {
            if let Some((payload, next_index)) = parenthesized_payload(&chars, index + 2) {
                payloads.push(payload);
                index = next_index;
                continue;
            }
        }
        index += 1;
    }

    payloads
}

fn backtick_payload(chars: &[char], mut index: usize) -> Option<(String, usize)> {
    let mut payload = String::new();
    let mut escaped = false;
    while index < chars.len() {
        let character = chars[index];
        if escaped {
            payload.push(character);
            escaped = false;
            index += 1;
            continue;
        }
        if character == '\\' {
            payload.push(character);
            escaped = true;
            index += 1;
            continue;
        }
        if character == '`' {
            return Some((payload, index + 1));
        }
        payload.push(character);
        index += 1;
    }
    None
}

fn parenthesized_payload(chars: &[char], mut index: usize) -> Option<(String, usize)> {
    let mut payload = String::new();
    let mut depth = 1usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while index < chars.len() {
        let character = chars[index];
        if escaped {
            payload.push(character);
            escaped = false;
            index += 1;
            continue;
        }
        if character == '\\' && !in_single_quote {
            payload.push(character);
            escaped = true;
            index += 1;
            continue;
        }
        if character == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            payload.push(character);
            index += 1;
            continue;
        }
        if character == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            payload.push(character);
            index += 1;
            continue;
        }
        if !in_single_quote && character == '$' && chars.get(index + 1) == Some(&'(') {
            depth = depth.saturating_add(1);
            payload.push(character);
            payload.push('(');
            index += 2;
            continue;
        }
        if !in_single_quote && !in_double_quote && character == '(' {
            depth = depth.saturating_add(1);
            payload.push(character);
            index += 1;
            continue;
        }
        if !in_single_quote && !in_double_quote && character == ')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((payload, index + 1));
            }
            payload.push(character);
            index += 1;
            continue;
        }
        payload.push(character);
        index += 1;
    }

    None
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
    decide_match(is_critical(command), mode)
}

/// Decide what to do with a direct program invocation while preserving exact
/// executable and argument boundaries.
pub(crate) fn decide_program(
    executable: &str,
    args: &[String],
    mode: CriticalMode,
) -> CriticalDecision {
    decide_match(is_critical_program(executable, args), mode)
}

fn decide_match(matched_pattern: Option<&'static str>, mode: CriticalMode) -> CriticalDecision {
    match (matched_pattern, mode) {
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
    fn normalizes_rm_flags_whitespace_and_long_options() {
        for command in [
            "rm -fr /",
            "rm -rf  /",
            "rm --recursive --force /",
            "env LANG=C sudo -u root rm -fR //",
            "env -u LANG sudo TEST_MODE=1 rm -fR //",
            "bash -c 'rm -fr /'",
            "printf ok && rm --force --recursive \"${HOME}\"",
            "echo \"$(rm --force --recursive /)\"",
            "echo \"`rm -fr /`\"",
        ] {
            assert_eq!(
                is_critical(command),
                Some("rm -rf /"),
                "missed normalized variant: {command}"
            );
        }
    }

    #[test]
    fn does_not_treat_a_literal_rm_example_as_a_command() {
        assert_eq!(is_critical("printf '%s' 'rm -rf /'"), None);
        assert_eq!(is_critical("git commit -m 'document rm -rf /'"), None);
        assert_eq!(is_critical("printf '%s' '$(rm -rf /)'"), None);
        assert_eq!(is_critical("printf '%s' \"rm -rf /\""), None);
    }

    #[test]
    fn matches_dd_to_device() {
        assert!(is_critical("dd of=/dev/sda bs=1M").is_some());
    }

    #[test]
    fn matches_drop_database() {
        assert_eq!(
            is_critical("psql -c 'drop   database prod;'"),
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
        assert!(is_critical("git push origin +HEAD:refs/heads/main").is_some());
        assert!(is_critical("git push --force-with-lease origin HEAD:master").is_some());
        assert!(is_critical("git push origin feature").is_none());
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
    fn direct_shell_program_preserves_and_reviews_its_payload() {
        let args = vec!["-c".to_string(), "rm -fr /".to_string()];
        assert_eq!(is_critical_program("/bin/sh", &args), Some("rm -rf /"));
        assert_eq!(
            decide_program("/bin/sh", &args, CriticalMode::Safe),
            CriticalDecision::Block("rm -rf /")
        );
    }

    #[test]
    fn ordinary_program_arguments_are_not_reparsed_as_shell_commands() {
        let args = vec!["%s".to_string(), "rm -rf /".to_string()];
        assert_eq!(is_critical_program("printf", &args), None);
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
    fn critical_mode_default_is_safe() {
        assert_eq!(CriticalMode::default(), CriticalMode::Safe);
    }

    #[test]
    fn critical_mode_serializes_lowercase() {
        let s = serde_json::to_string(&CriticalMode::Safe).unwrap();
        assert_eq!(s, "\"safe\"");
        let m: CriticalMode = serde_json::from_str("\"open\"").unwrap();
        assert_eq!(m, CriticalMode::Open);
    }

    #[test]
    fn configurable_blocklist_matching_normalizes_trivial_bypasses() {
        assert!(matches_blocked_pattern("rm -fr  /", "rm -rf /"));
        assert!(matches_blocked_pattern(
            "git push --FORCE origin MAIN",
            "git push --force origin main"
        ));
        assert!(matches_blocked_pattern(
            "psql -c 'drop   database prod'",
            "DROP DATABASE"
        ));
        assert!(!matches_blocked_pattern("printf safe", "rm -rf /"));
    }
}
