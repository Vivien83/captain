//! Review/action command helpers for channel commands.

use super::command_format::{
    format_id_prefix_usage, format_project_answer_usage, format_skill_approval_usage,
};
use std::future::Future;

pub(crate) async fn run_id_prefix_command<F, Fut>(
    args: &[String],
    command: &str,
    action: F,
) -> String
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = String>,
{
    let Some(id_prefix) = args.first().cloned() else {
        return format_id_prefix_usage(command);
    };
    action(id_prefix).await
}

pub(crate) async fn run_project_answer_command<F, Fut>(args: &[String], action: F) -> String
where
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = String>,
{
    if args.len() < 2 {
        return format_project_answer_usage();
    }
    let id_prefix = args[0].clone();
    let answer = args[1..].join(" ");
    action(id_prefix, answer).await
}

pub(crate) async fn run_skill_approval_command<F, Fut>(args: &[String], action: F) -> String
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = String>,
{
    let Some(id_prefix) = args.first().cloned() else {
        return format_skill_approval_usage();
    };
    if !skill_approval_external_validation(args) {
        return format_skill_approval_usage();
    }
    action(id_prefix).await
}

fn skill_approval_external_validation(args: &[String]) -> bool {
    let normalized = args
        .iter()
        .skip(1)
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    if normalized
        .iter()
        .any(|value| value == "schema_diff_tests_human")
    {
        return true;
    }
    ["schema", "diff", "tests", "human"]
        .iter()
        .all(|needle| normalized.iter().any(|value| value == needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[tokio::test]
    async fn id_prefix_command_returns_usage_without_id() {
        let text =
            run_id_prefix_command(&[], "approve", |_id| async { "should not run".to_string() })
                .await;

        assert_eq!(text, "Usage: /approve <id-prefix>");
    }

    #[tokio::test]
    async fn id_prefix_command_passes_owned_id_to_action() {
        let text = run_id_prefix_command(&args(&["abc123"]), "approve", |id| async move {
            format!("approved:{id}")
        })
        .await;

        assert_eq!(text, "approved:abc123");
    }

    #[tokio::test]
    async fn project_answer_command_requires_id_and_answer() {
        let text = run_project_answer_command(&args(&["ask-1"]), |_id, _answer| async {
            "should not run".to_string()
        })
        .await;

        assert_eq!(text, "Usage: /project_answer <id-prefix> <réponse>");
    }

    #[tokio::test]
    async fn project_answer_command_joins_remaining_words() {
        let text = run_project_answer_command(
            &args(&["ask-1", "oui", "avec", "details"]),
            |id, answer| async move { format!("{id}:{answer}") },
        )
        .await;

        assert_eq!(text, "ask-1:oui avec details");
    }

    #[tokio::test]
    async fn skill_approval_command_requires_external_validation_words() {
        let text = run_skill_approval_command(&args(&["prop-1"]), |_id| async {
            "should not run".to_string()
        })
        .await;

        assert_eq!(
            text,
            "Usage: /skill_approve <id-prefix> schema diff tests human"
        );
    }

    #[tokio::test]
    async fn skill_approval_command_passes_id_after_external_validation() {
        let text = run_skill_approval_command(
            &args(&["prop-1", "schema", "diff", "tests", "human"]),
            |id| async move { format!("approved:{id}") },
        )
        .await;

        assert_eq!(text, "approved:prop-1");
    }
}
