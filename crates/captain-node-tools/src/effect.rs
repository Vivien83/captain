use crate::node_tool_runtime::LocalNodeToolEffect;

pub(crate) fn classify(tool_name: &str, input: &serde_json::Value) -> LocalNodeToolEffect {
    match tool_name {
        "file_read" | "file_list" | "glob" | "grep" | "file_inspect_batch" => {
            LocalNodeToolEffect::ReadOnly
        }
        "file_write" | "edit_file" | "multi_edit" | "apply_patch" => {
            LocalNodeToolEffect::LocalMutation
        }
        "shell_exec" => input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .map(classify_shell)
            .unwrap_or(LocalNodeToolEffect::ExternalEffect),
        _ => LocalNodeToolEffect::ExternalEffect,
    }
}

fn classify_shell(command: &str) -> LocalNodeToolEffect {
    let normalized = format!(
        " {} ",
        command
            .to_ascii_lowercase()
            .replace('\r', " ")
            .replace('\n', " ; ")
    );
    if has_external_effect(&normalized) {
        LocalNodeToolEffect::ExternalEffect
    } else if has_mutation(&normalized) || has_verification(&normalized) {
        LocalNodeToolEffect::LocalMutation
    } else if is_observation(&normalized) {
        LocalNodeToolEffect::ReadOnly
    } else {
        LocalNodeToolEffect::ExternalEffect
    }
}

fn has_external_effect(command: &str) -> bool {
    [
        " git push",
        " docker push",
        " kubectl apply",
        " kubectl create",
        " kubectl delete",
        " gh release create",
        " gh pr create",
        " gh pr merge",
        " gh pr close",
        " gh issue create",
        " curl -x post",
        " curl -x put",
        " curl -x patch",
        " curl -x delete",
        " curl --request post",
        " curl --request put",
        " curl --request patch",
        " curl --request delete",
        " curl -d ",
        " curl --data",
        " curl -t ",
        " curl --upload-file",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn has_mutation(command: &str) -> bool {
    [
        " rm ",
        " mv ",
        " cp ",
        " mkdir ",
        " touch ",
        " chmod ",
        " chown ",
        " tee ",
        " sed -i",
        " git add",
        " git commit",
        " git checkout",
        " git switch",
        " git reset",
        " git restore",
        " git clean",
        " cargo install",
        " npm install",
        " npm update",
        " pip install",
        " apt install",
        " apt-get install",
        " brew install",
        " docker restart",
        " docker start",
        " docker stop",
        " docker rm",
        " docker compose up",
        " docker compose down",
        " systemctl restart",
        " systemctl start",
        " systemctl stop",
        " systemctl enable",
        " systemctl disable",
        " captain update",
    ]
    .iter()
    .any(|needle| command.contains(needle))
        || (command.contains(" cargo fmt") && !command.contains("--check"))
        || command.contains(" > ")
        || command.contains(" >> ")
}

fn has_verification(command: &str) -> bool {
    [
        " cargo test",
        " cargo check",
        " cargo clippy",
        " cargo build",
        " cargo fmt --check",
        " npm test",
        " npm run test",
        " npm run lint",
        " pytest",
        " go test",
        " git status",
        " git diff",
        " git show",
        " git log",
        " git rev-parse",
        " captain doctor",
        " captain status",
        " systemctl status",
        " systemctl is-active",
        " journalctl",
        " docker ps",
        " docker logs",
        " docker inspect",
        " health_check",
        " integrity_check",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn is_observation(command: &str) -> bool {
    let mut saw_command = false;
    let only_observations = command
        .split([';', '&', '|'])
        .filter_map(|segment| segment.split_whitespace().next())
        .filter(|word| !matches!(*word, "set" | "then" | "do" | "done"))
        .all(|word| {
            saw_command = true;
            matches!(
                word,
                "cat"
                    | "date"
                    | "df"
                    | "du"
                    | "echo"
                    | "env"
                    | "false"
                    | "find"
                    | "free"
                    | "grep"
                    | "head"
                    | "id"
                    | "jq"
                    | "ls"
                    | "memory_pressure"
                    | "pgrep"
                    | "printenv"
                    | "printf"
                    | "ps"
                    | "pwd"
                    | "rg"
                    | "ss"
                    | "stat"
                    | "sysctl"
                    | "tail"
                    | "test"
                    | "true"
                    | "uname"
                    | "uptime"
                    | "vm_stat"
                    | "wc"
                    | "which"
                    | "whoami"
            )
        });
    saw_command && only_observations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributed_shell_effects_remain_conservative() {
        assert_eq!(classify_shell("pwd"), LocalNodeToolEffect::ReadOnly);
        assert_eq!(
            classify_shell("git status --short"),
            LocalNodeToolEffect::LocalMutation
        );
        assert_eq!(
            classify_shell("git push origin main"),
            LocalNodeToolEffect::ExternalEffect
        );
        assert_eq!(
            classify_shell("unknown-command"),
            LocalNodeToolEffect::ExternalEffect
        );
    }
}
