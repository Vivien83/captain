//! Automation command parsing for channel commands.

use super::command_format::{format_schedule_usage, format_trigger_usage, format_workflow_usage};
use std::future::Future;

pub(crate) async fn run_workflow_command<F, Fut>(args: &[String], action: F) -> String
where
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = String>,
{
    if args.len() < 2 || args[0] != "run" {
        return format_workflow_usage();
    }
    let workflow_name = args[1].clone();
    let input = if args.len() > 2 {
        args[2..].join(" ")
    } else {
        String::new()
    };
    action(workflow_name, input).await
}

pub(crate) async fn run_trigger_command<FCreate, FutCreate, FDelete, FutDelete>(
    args: &[String],
    create: FCreate,
    delete: FDelete,
) -> String
where
    FCreate: FnOnce(String, String, String) -> FutCreate,
    FutCreate: Future<Output = String>,
    FDelete: FnOnce(String) -> FutDelete,
    FutDelete: Future<Output = String>,
{
    if args.len() >= 4 && args[0] == "add" {
        let agent_name = args[1].clone();
        let pattern = args[2].clone();
        let prompt = args[3..].join(" ");
        return create(agent_name, pattern, prompt).await;
    }
    if args.len() >= 2 && args[0] == "del" {
        return delete(args[1].clone()).await;
    }
    format_trigger_usage()
}

pub(crate) async fn run_schedule_command<F, Fut>(args: &[String], manage: F) -> String
where
    F: FnOnce(String, Vec<String>) -> Fut,
    Fut: Future<Output = String>,
{
    let Some(action) = args.first() else {
        return format_schedule_usage();
    };
    if !matches!(action.as_str(), "add" | "del" | "run") {
        return format_schedule_usage();
    }
    manage(action.clone(), args[1..].to_vec()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[tokio::test]
    async fn workflow_requires_run_and_name() {
        let text = run_workflow_command(&args(&["list"]), |_name, _input| async {
            "should not run".to_string()
        })
        .await;

        assert_eq!(text, "Usage: /workflow run <name> [input]");
    }

    #[tokio::test]
    async fn workflow_passes_name_and_joined_input() {
        let text = run_workflow_command(
            &args(&["run", "daily", "hello", "world"]),
            |name, input| async move { format!("{name}:{input}") },
        )
        .await;

        assert_eq!(text, "daily:hello world");
    }

    #[tokio::test]
    async fn trigger_add_and_delete_route_to_separate_actions() {
        let add = run_trigger_command(
            &args(&["add", "captain", "pattern", "prompt", "tail"]),
            |agent, pattern, prompt| async move { format!("add:{agent}:{pattern}:{prompt}") },
            |_id| async { "delete should not run".to_string() },
        )
        .await;
        assert_eq!(add, "add:captain:pattern:prompt tail");

        let delete = run_trigger_command(
            &args(&["del", "abc123"]),
            |_agent, _pattern, _prompt| async { "add should not run".to_string() },
            |id| async move { format!("del:{id}") },
        )
        .await;
        assert_eq!(delete, "del:abc123");
    }

    #[tokio::test]
    async fn trigger_reports_usage_for_incomplete_actions() {
        let text = run_trigger_command(
            &args(&["add", "captain"]),
            |_agent, _pattern, _prompt| async { "should not add".to_string() },
            |_id| async { "should not delete".to_string() },
        )
        .await;

        assert!(text.contains("/trigger add <agent> <pattern> <prompt>"));
    }

    #[tokio::test]
    async fn schedule_accepts_only_known_actions() {
        let invalid = run_schedule_command(&args(&["pause", "abc"]), |_action, _rest| async {
            "should not run".to_string()
        })
        .await;
        assert!(invalid.contains("/schedule run <id-prefix>"));

        let valid = run_schedule_command(&args(&["run", "abc"]), |action, rest| async move {
            format!("{action}:{}", rest.join(","))
        })
        .await;
        assert_eq!(valid, "run:abc");
    }
}
