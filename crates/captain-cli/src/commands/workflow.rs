use std::path::PathBuf;

use crate::{daemon_client, daemon_json, require_daemon};

pub(crate) fn cmd_workflow_list() {
    let base = require_daemon("workflow list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/workflows")).send());

    match body.as_array() {
        Some(workflows) if workflows.is_empty() => println!("No workflows registered."),
        Some(workflows) => {
            println!("{:<38} {:<20} {:<6} CREATED", "ID", "NAME", "STEPS");
            println!("{}", "-".repeat(80));
            for w in workflows {
                println!(
                    "{:<38} {:<20} {:<6} {}",
                    w["id"].as_str().unwrap_or("?"),
                    w["name"].as_str().unwrap_or("?"),
                    w["steps"].as_u64().unwrap_or(0),
                    w["created_at"].as_str().unwrap_or("?"),
                );
            }
        }
        None => println!("No workflows registered."),
    }
}

pub(crate) fn cmd_workflow_create(file: PathBuf) {
    let base = require_daemon("workflow create");
    let json_body = read_workflow_json(&file);
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows"))
            .json(&json_body)
            .send(),
    );

    if let Some(id) = body["workflow_id"].as_str() {
        println!("Workflow created successfully!");
        println!("  ID: {id}");
    } else {
        eprintln!(
            "Failed to create workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_workflow_run(workflow_id: &str, input: &str) {
    let base = require_daemon("workflow run");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows/{workflow_id}/run"))
            .json(&serde_json::json!({"input": input}))
            .send(),
    );

    if let Some(output) = body["output"].as_str() {
        println!("Workflow completed!");
        println!("  Run ID: {}", body["run_id"].as_str().unwrap_or("?"));
        println!("  Output:\n{output}");
    } else {
        eprintln!(
            "Workflow failed: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_workflow_get(workflow_id: &str) {
    let base = require_daemon("workflow get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/workflows/{workflow_id}"))
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Workflow not found: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }

    println!("Workflow: {}", body["name"].as_str().unwrap_or("?"));
    println!("  ID:          {}", body["id"].as_str().unwrap_or("?"));
    println!(
        "  Description: {}",
        body["description"].as_str().unwrap_or("")
    );
    println!(
        "  Created:     {}",
        body["created_at"].as_str().unwrap_or("?")
    );

    if let Some(steps) = body["steps"].as_array() {
        println!("  Steps ({}):", steps.len());
        for (i, s) in steps.iter().enumerate() {
            let name = s["name"].as_str().unwrap_or("step");
            let agent = s["agent"]
                .get("name")
                .or_else(|| s["agent"].get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("    #{}: {} -> {}", i + 1, name, agent);
        }
    }
}

pub(crate) fn cmd_workflow_update(workflow_id: &str, file: PathBuf) {
    let base = require_daemon("workflow update");
    let json_body = read_workflow_json(&file);
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/workflows/{workflow_id}"))
            .json(&json_body)
            .send(),
    );

    if body["status"].as_str() == Some("updated") {
        println!("Workflow updated successfully!");
        println!("  ID: {}", body["workflow_id"].as_str().unwrap_or("?"));
    } else {
        eprintln!(
            "Failed to update workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_workflow_delete(workflow_id: &str) {
    let base = require_daemon("workflow delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/workflows/{workflow_id}"))
            .send(),
    );

    if body["status"].as_str() == Some("removed") {
        println!("Workflow deleted successfully!");
        println!("  ID: {}", body["workflow_id"].as_str().unwrap_or("?"));
    } else {
        eprintln!(
            "Failed to delete workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn read_workflow_json(file: &PathBuf) -> serde_json::Value {
    if !file.exists() {
        eprintln!("Workflow file not found: {}", file.display());
        std::process::exit(1);
    }
    let contents = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Error reading workflow file: {e}");
        std::process::exit(1);
    });
    serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("Invalid JSON: {e}");
        std::process::exit(1);
    })
}
