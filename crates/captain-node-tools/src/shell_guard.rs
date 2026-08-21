use captain_types::config::{CriticalMode, ExecPolicy, ExecSecurityMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellReviewRejection {
    Critical,
    Denied,
}

pub(crate) fn review(command: &str, policy: &ExecPolicy) -> Result<(), ShellReviewRejection> {
    if command.is_empty()
        || crate::output_security::ensure_no_secret_literal(command).is_err()
        || sources_secret_environment(command)
        || unbounded_command(command)
        || detached_command(command)
    {
        return Err(ShellReviewRejection::Denied);
    }

    if policy.critical_mode == CriticalMode::Paranoid || critical_command(command) {
        return Err(ShellReviewRejection::Critical);
    }

    match policy.effective_mode() {
        ExecSecurityMode::Deny => Err(ShellReviewRejection::Denied),
        ExecSecurityMode::Full => {
            if policy
                .blocked_commands
                .iter()
                .any(|blocked| normalized_contains(command, blocked))
            {
                Err(ShellReviewRejection::Denied)
            } else {
                Ok(())
            }
        }
        ExecSecurityMode::Allowlist => review_allowlist(command, policy),
    }
}

fn review_allowlist(command: &str, policy: &ExecPolicy) -> Result<(), ShellReviewRejection> {
    if contains_shell_metacharacters(command).is_some() {
        return Err(ShellReviewRejection::Denied);
    }
    let words = shlex::split(command).ok_or(ShellReviewRejection::Denied)?;
    let executable = words.first().ok_or(ShellReviewRejection::Denied)?;
    let base = executable
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(executable.as_str());
    if policy.safe_bins.iter().any(|allowed| allowed == base)
        || policy
            .allowed_commands
            .iter()
            .any(|allowed| allowed == base)
    {
        Ok(())
    } else {
        Err(ShellReviewRejection::Denied)
    }
}

fn normalized_contains(command: &str, blocked: &str) -> bool {
    let command = normalize(command);
    let blocked = normalize(blocked);
    !blocked.is_empty() && command.contains(&blocked)
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn sources_secret_environment(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if !lower.contains("secrets.env") {
        return false;
    }
    let normalized = format!(" {} ", lower.replace(['\n', '\r', '\t'], " "));
    normalized.contains(" source ")
        || normalized.contains(" . ~/.captain/secrets.env")
        || normalized.contains(" . $home/.captain/secrets.env")
        || normalized.contains(" . /root/.captain/secrets.env")
        || normalized.contains(" . /home/")
        || lower.contains("set -a")
        || lower.contains("set -o allexport")
}

fn unbounded_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("pmset -g thermlog")
        || lower.contains("log stream")
        || lower.contains("tail -f")
        || lower.contains("fs_usage")
        || lower.contains("tcpdump")
        || (lower.split_whitespace().next() == Some("top")
            && !lower.contains("-l ")
            && !lower.contains("-l1"))
}

fn detached_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| matches!(word, "nohup" | "disown"))
    {
        return true;
    }
    contains_unquoted_background_operator(command)
}

fn contains_unquoted_background_operator(command: &str) -> bool {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let characters: Vec<char> = command.chars().collect();
    for (index, character) in characters.iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && !single {
            escaped = true;
            continue;
        }
        if character == '\'' && !double {
            single = !single;
            continue;
        }
        if character == '"' && !single {
            double = !double;
            continue;
        }
        if character == '&' && !single && !double {
            let previous = index.checked_sub(1).and_then(|i| characters.get(i));
            let next = characters.get(index + 1);
            if previous != Some(&'&') && next != Some(&'&') {
                return true;
            }
        }
    }
    false
}

fn critical_command(command: &str) -> bool {
    let normalized = normalize(command);
    let compact: String = command
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if compact.contains(":(){:|:&};:")
        || normalized.contains("drop database")
        || normalized.contains("drop schema")
        || normalized.contains("truncate table")
        || normalized.contains("dd if=")
        || normalized.contains("dd of=/dev/")
        || normalized.contains("wipefs")
        || normalized.contains("mkfs")
        || normalized.contains("chmod -r 777 /")
        || normalized.contains("git push --force origin main")
        || normalized.contains("git push --force origin master")
        || normalized.contains("git push -f origin main")
        || normalized.contains("git push -f origin master")
    {
        return true;
    }
    let Some(words) = shlex::split(command) else {
        return true;
    };
    let rm = words.iter().position(|word| word == "rm");
    let Some(rm) = rm else {
        return false;
    };
    let arguments = &words[rm + 1..];
    let recursive = arguments.iter().any(|arg| {
        arg == "--recursive"
            || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('r'))
    });
    let force = arguments.iter().any(|arg| {
        arg == "--force" || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('f'))
    });
    let root_or_home = arguments.iter().any(|arg| {
        let target = arg
            .trim_matches(['\'', '"'])
            .trim_end_matches('*')
            .trim_end_matches('/');
        target.is_empty() || matches!(target, "~" | "$HOME" | "${HOME}")
    });
    recursive && force && root_or_home
}

fn contains_shell_metacharacters(command: &str) -> Option<&'static str> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && !single {
            escaped = true;
            continue;
        }
        if character == '\'' && !double {
            single = !single;
            continue;
        }
        if character == '"' && !single {
            double = !double;
            continue;
        }
        if !single && !double {
            match character {
                '|' => return Some("pipe operator"),
                ';' => return Some("semicolon operator"),
                '&' => return Some("ampersand operator"),
                '>' | '<' => return Some("redirection operator"),
                '`' => return Some("command substitution"),
                '\n' | '\r' => return Some("embedded newline"),
                '\0' => return Some("null byte"),
                _ => {}
            }
        }
    }
    if single || double || escaped {
        Some("unbalanced quoting")
    } else if command.contains("$(") || command.contains("${") {
        Some("shell expansion")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{ExecSecurityMode, ExecutionProfile};

    fn policy() -> ExecPolicy {
        ExecPolicy {
            profile: ExecutionProfile::RemoteOperator,
            mode: ExecSecurityMode::Full,
            safe_bins: vec!["git".into(), "pwd".into()],
            allowed_commands: vec!["git".into(), "pwd".into()],
            critical_mode: CriticalMode::Open,
            ..ExecPolicy::default()
        }
    }

    #[test]
    fn remote_policy_denies_critical_and_unlisted_commands() {
        assert_eq!(
            review("rm -rf /", &policy()),
            Err(ShellReviewRejection::Critical)
        );
        assert_eq!(
            review("curl https://example.com", &policy()),
            Err(ShellReviewRejection::Denied)
        );
        assert_eq!(review("git status --short", &policy()), Ok(()));
    }
}
