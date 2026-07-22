use std::path::PathBuf;

use super::status_compat::ensure_status_observability;
use super::status_health::{print_disk_summary, print_runtime_health_summary};
use super::status_in_process::in_process_runtime_health;
use super::status_shutdown::print_shutdown_drain_summary;
use super::status_verbose::print_verbose_runtime;
use super::status_workload::{kernel_status_workload, print_status_workload};
use crate::{boot_kernel, daemon_client, daemon_json, find_daemon, ui, ServiceManagerArg};

pub(crate) fn cmd_status(config: Option<PathBuf>, json: bool, verbose: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let mut body = daemon_json(client.get(format!("{base}/api/status")).send());
        ensure_status_observability(&mut body);

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        print_daemon_status(&base, &body, verbose);
    } else {
        print_in_process_status(config, json, verbose);
    }
}

fn print_daemon_status(base: &str, body: &serde_json::Value, verbose: bool) {
    ui::section("Captain Daemon Status");
    ui::blank();
    ui::kv_ok("Status", body["status"].as_str().unwrap_or("?"));
    ui::kv(
        "Agents",
        &body["agent_count"].as_u64().unwrap_or(0).to_string(),
    );
    ui::kv(
        "Active runs",
        &body["active_run_count"].as_u64().unwrap_or(0).to_string(),
    );
    ui::kv("Provider", body["default_provider"].as_str().unwrap_or("?"));
    ui::kv("Model", body["default_model"].as_str().unwrap_or("?"));
    print_llm_status(
        body["llm_driver_ready"].as_bool().unwrap_or(true),
        body["llm_driver_error"].as_str(),
        true,
    );
    ui::kv("API", base);
    if let Some(listen) = body["api_listen"].as_str().filter(|s| !s.trim().is_empty()) {
        ui::kv("Listen", listen);
    }
    if let Some(public_url) = body["deployment"]["public_url"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
    {
        ui::kv("Public URL", public_url);
    }
    ui::kv("Web terminal", &format!("{base}/terminal"));
    ui::kv("Data dir", body["data_dir"].as_str().unwrap_or("?"));
    ui::kv(
        "Uptime",
        &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
    );

    print_daemon_runtime(body);
    print_status_workload(body, verbose);
    print_budget_summary(body, verbose);
    print_daemon_paths(body, verbose);
    if verbose {
        print_verbose_runtime(body);
    }
    print_active_agents(body);
}

fn print_daemon_runtime(body: &serde_json::Value) {
    ui::blank();
    ui::section("Runtime");
    ui::kv("Auth", body["auth_mode"].as_str().unwrap_or("?"));
    print_runtime_health_summary(body);
    print_shutdown_drain_summary(body);
    print_disk_summary(body);
    print_runtime_update_summary(body);
    let channels = &body["channels"];
    let configured = channels["configured_count"]
        .as_u64()
        .unwrap_or_else(|| body["channel_configured_count"].as_u64().unwrap_or(0));
    let total = channels["total"]
        .as_u64()
        .unwrap_or_else(|| body["channel_total"].as_u64().unwrap_or(0));
    let ready = channels["ready_count"].as_u64().unwrap_or(configured);
    let channel_names = status_name_list(&channels["configured"])
        .or_else(|| status_name_list(&body["configured_channels"]))
        .unwrap_or_default();
    if channel_names.is_empty() {
        ui::kv(
            "Channels",
            &format!("{configured}/{total} configured, {ready} ready"),
        );
    } else {
        ui::kv(
            "Channels",
            &format!("{configured}/{total} configured, {ready} ready ({channel_names})"),
        );
    }
    if let Some(locked) = status_name_list(&channels["locked"]).filter(|names| !names.is_empty()) {
        ui::kv_warn("Channel readiness", &format!("locked: {locked}"));
        ui::hint("Run `captain channel list` for missing fields and setup actions.");
    }
    print_streaming_summary(body);
    print_agent_api_summary(body);
    print_consciousness_summary(body);
    let tts_provider = body["tts"]["provider"].as_str().unwrap_or("off");
    let tts_enabled = body["tts"]["enabled"].as_bool().unwrap_or(false);
    ui::kv(
        "TTS",
        if tts_enabled {
            tts_provider
        } else {
            "disabled"
        },
    );
    print_native_capability_summary(body);
    ui::kv("Media", &media_summary(body));
}

fn print_streaming_summary(body: &serde_json::Value) {
    if let Some(summary) = streaming_status_summary(&body["streaming"]) {
        ui::kv("Streaming", &summary);
    }
}

fn print_runtime_update_summary(body: &serde_json::Value) {
    let update = &body["runtime_update"];
    if update.is_null() {
        return;
    }
    if let Some(error) = update["error"].as_str() {
        ui::kv_warn("Captain update", "state unavailable");
        ui::hint(error);
        return;
    }
    let next = update["next_check_at"].as_str().unwrap_or("unknown");
    if update["update_in_progress"].as_bool().unwrap_or(false) {
        ui::kv_warn("Captain update", "installation in progress");
    } else if let Some(version) = update["pending_version"].as_str() {
        ui::kv_warn(
            "Captain update",
            &format!("{version} available, next check {next}"),
        );
    } else if update["last_success_at"].is_string() {
        ui::kv_ok("Captain update", &format!("current, next check {next}"));
    } else {
        ui::kv("Captain update", &format!("first check scheduled {next}"));
    }
    if let Some(error) = update["last_error"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    {
        ui::hint(error);
    }
    let dead = update["dead_notifications"].as_u64().unwrap_or(0);
    if dead > 0 {
        ui::hint(&format!(
            "{dead} update notification(s) exhausted delivery retries; Captain will reopen delivery on the next 12-hour release check."
        ));
    }
}

fn streaming_status_summary(streaming: &serde_json::Value) -> Option<String> {
    if streaming.is_null() {
        return None;
    }
    let active = streaming["active"].as_u64().unwrap_or(0);
    let completed = streaming["completed"].as_u64().unwrap_or(0);
    let mut summary = format!("active {active}, completed {completed}");
    let last = &streaming["last"];
    if !last.is_null() {
        if let Some(surface) = last["surface"].as_str().filter(|value| !value.is_empty()) {
            summary.push_str(&format!(", last {surface}"));
        }
        if let Some(first_token) = latency_ms_label(last["first_token_ms"].as_u64()) {
            summary.push_str(&format!(", first-token {first_token}"));
        }
        if let Some(first_signal) = latency_ms_label(last["first_signal_ms"].as_u64()) {
            let kind = last["first_signal_kind"].as_str().unwrap_or("signal");
            summary.push_str(&format!(", first-signal {kind} {first_signal}"));
        }
        if let Some(total) = latency_ms_label(last["total_ms"].as_u64()) {
            summary.push_str(&format!(", total {total}"));
        }
    } else if active > 0 {
        summary.push_str(", waiting for first completed stream");
    }
    Some(summary)
}

fn latency_ms_label(value: Option<u64>) -> Option<String> {
    value.map(|ms| format!("{ms}ms"))
}

fn print_consciousness_summary(body: &serde_json::Value) {
    let consciousness = &body["consciousness"];
    let Some(state) = consciousness["state"].as_str() else {
        return;
    };
    let confidence = consciousness["confidence"].as_f64().unwrap_or(0.0);
    let queued = consciousness["queued_thoughts"].as_u64().unwrap_or(0);
    let active_work = consciousness["active_work"].as_u64().unwrap_or(0);
    let project_attention = consciousness["projects"]["attention"].as_u64().unwrap_or(0);
    let mut summary = format!("{state} (conf {confidence:.2}, queue {queued}, work {active_work}");
    if project_attention > 0 {
        summary.push_str(&format!(", projects {project_attention}"));
    }
    summary.push(')');
    if state == "steady" {
        ui::kv_ok("Consciousness", &summary);
    } else {
        ui::kv_warn("Consciousness", &summary);
    }
    if let Some(action) = first_string(&consciousness["operator_actions"]) {
        ui::hint(&action);
    }
}

fn first_string(value: &serde_json::Value) -> Option<String> {
    value
        .as_array()?
        .iter()
        .find_map(|item| item.as_str().map(String::from))
}

fn print_agent_api_summary(body: &serde_json::Value) {
    let queue = &body["agent_api"]["egress_queue"];
    let Some(summary) = agent_api_queue_issue_summary(queue) else {
        return;
    };
    ui::kv_warn("Agent API", &summary);
    if let Some(issue) = queue["issue"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    {
        ui::hint(issue);
    } else {
        ui::hint("Inspect `/api/agents/{id}/api/egress` before retrying callbacks.");
    }
}

fn print_budget_summary(body: &serde_json::Value, verbose: bool) {
    let budget = &body["budget"];
    if budget.is_null() {
        return;
    }
    let global = &budget["global"];
    ui::blank();
    ui::section("Budget");
    ui::kv(
        "Global",
        &format!(
            "hour {} | day {} | month {}",
            budget_window_summary(
                global["hourly_spend"].as_f64().unwrap_or(0.0),
                global["hourly_limit"].as_f64().unwrap_or(0.0),
                global["hourly_pct"].as_f64().unwrap_or(0.0),
            ),
            budget_window_summary(
                global["daily_spend"].as_f64().unwrap_or(0.0),
                global["daily_limit"].as_f64().unwrap_or(0.0),
                global["daily_pct"].as_f64().unwrap_or(0.0),
            ),
            budget_window_summary(
                global["monthly_spend"].as_f64().unwrap_or(0.0),
                global["monthly_limit"].as_f64().unwrap_or(0.0),
                global["monthly_pct"].as_f64().unwrap_or(0.0),
            ),
        ),
    );
    ui::kv(
        "Agent tokens",
        &budget_token_summary(
            budget["total_tokens_used"].as_u64().unwrap_or(0),
            budget["limited_agents"].as_u64().unwrap_or(0),
        ),
    );
    print_provider_subscription_summary(&budget["provider_subscriptions"], verbose);
    if let Some(action) = first_string(&budget["operator_actions"]) {
        ui::hint(&action);
    }
    if verbose {
        if let Some(agents) = budget["agents"]
            .as_array()
            .filter(|items| !items.is_empty())
        {
            ui::blank();
            ui::section("Agent Budgets");
            for agent in agents {
                println!("    {}", agent_budget_line(agent));
            }
        }
    }
}

fn print_provider_subscription_summary(provider: &serde_json::Value, verbose: bool) {
    let state = provider["state"].as_str().unwrap_or("unavailable");
    let Some(items) = provider["items"].as_array() else {
        ui::kv_warn(
            "Provider subscription",
            "not observed (official provider signals only)",
        );
        return;
    };
    if items.is_empty() {
        ui::kv_warn(
            "Provider subscription",
            "not observed (official provider signals only)",
        );
        return;
    }

    let mut ordered = items.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|item| std::cmp::Reverse(provider_alert_rank(item)));
    let count = if verbose { ordered.len() } else { 1 };
    for (index, item) in ordered.into_iter().take(count).enumerate() {
        let label = if index == 0 {
            "Provider subscription"
        } else {
            "Provider limit"
        };
        let summary = provider_quota_item_summary(item);
        if state == "ok" && !item["stale"].as_bool().unwrap_or(false) {
            ui::kv_ok(label, &summary);
        } else {
            ui::kv_warn(label, &summary);
        }
    }
}

fn provider_alert_rank(item: &serde_json::Value) -> u8 {
    match item["alert_level"].as_str().unwrap_or("normal") {
        "exhausted" => 3,
        "critical" => 2,
        "warning" => 1,
        _ if item["stale"].as_bool().unwrap_or(false) => 1,
        _ => 0,
    }
}

fn provider_quota_item_summary(item: &serde_json::Value) -> String {
    let name = item["limit_name"]
        .as_str()
        .or_else(|| item["limit_id"].as_str())
        .unwrap_or("provider");
    let plan = item["plan_type"]
        .as_str()
        .map(|value| format!(" [{value}]"))
        .unwrap_or_default();
    let windows = ["primary", "secondary"]
        .iter()
        .filter_map(|key| provider_window_summary(&item[*key]))
        .collect::<Vec<_>>();
    let mut summary = format!("{name}{plan}");
    if !windows.is_empty() {
        summary.push_str(" -- ");
        summary.push_str(&windows.join(" | "));
    }
    let alert = item["alert_level"].as_str().unwrap_or("normal");
    if alert != "normal" {
        summary.push_str(&format!(" [{alert}]"));
    }
    if item["stale"].as_bool().unwrap_or(false) {
        summary.push_str(" [stale]");
    }
    summary
}

fn provider_window_summary(window: &serde_json::Value) -> Option<String> {
    let used = window["used_percent"].as_f64()?;
    let duration = window["window_seconds"]
        .as_u64()
        .map(provider_window_duration)
        .unwrap_or_else(|| "window".to_string());
    let reset = window["resets_at"]
        .as_str()
        .map(|value| format!(", reset {value}"))
        .unwrap_or_default();
    Some(format!("{duration} {used:.1}%{reset}"))
}

fn provider_window_duration(seconds: u64) -> String {
    if seconds % 604_800 == 0 {
        format!("{}w", seconds / 604_800)
    } else if seconds % 86_400 == 0 {
        format!("{}d", seconds / 86_400)
    } else if seconds % 3_600 == 0 {
        format!("{}h", seconds / 3_600)
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn budget_window_summary(spend: f64, limit: f64, pct: f64) -> String {
    if limit > 0.0 {
        format!("${spend:.4}/${limit:.2} ({:.1}%)", pct * 100.0)
    } else {
        format!("${spend:.4}/unlimited")
    }
}

fn budget_token_summary(total_tokens_used: u64, limited_agents: u64) -> String {
    if limited_agents > 0 {
        format!("{total_tokens_used} used across {limited_agents} limited agent(s)")
    } else {
        format!("{total_tokens_used} used; token limits unlimited")
    }
}

fn agent_budget_line(agent: &serde_json::Value) -> String {
    let name = agent["name"].as_str().unwrap_or("?");
    let id = agent["agent_id"].as_str().unwrap_or("?");
    let used = agent["tokens"]["used"].as_u64().unwrap_or(0);
    let limit = agent["tokens"]["limit"].as_u64().unwrap_or(0);
    let pct = agent["tokens"]["pct"].as_f64().unwrap_or(0.0);
    let calls = agent["tool_calls"]["used"].as_u64().unwrap_or(0);
    let calls_limit = agent["tool_calls"]["limit_per_minute"]
        .as_u64()
        .unwrap_or(0);
    if limit > 0 {
        format!(
            "{name} ({id}) -- tokens {used}/{limit} ({:.1}%), tools {calls}/{calls_limit}/min",
            pct * 100.0
        )
    } else {
        format!("{name} ({id}) -- tokens {used}/unlimited, tools {calls}/{calls_limit}/min")
    }
}

fn agent_api_queue_issue_summary(queue: &serde_json::Value) -> Option<String> {
    if queue.is_null() {
        return None;
    }
    if queue["readable"].as_bool() == Some(false) {
        return Some("egress queue unavailable".to_string());
    }
    let pending = queue["pending"].as_u64().unwrap_or(0);
    let due = queue["due"].as_u64().unwrap_or(0);
    let dead = queue["dead_letters"].as_u64().unwrap_or(0);
    let agents = queue["agents_with_queue"].as_u64().unwrap_or(0);
    if pending == 0 && due == 0 && dead == 0 {
        return None;
    }
    Some(format!(
        "{pending} pending callback(s), {due} due, {dead} dead letter(s), {agents} agent(s)"
    ))
}

fn status_name_list(value: &serde_json::Value) -> Option<String> {
    let names = value
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(names)
}

fn print_native_capability_summary(body: &serde_json::Value) {
    let native_voice = &body["native_voice"];
    let native_stt_ready = native_voice["stt_ready"].as_bool().unwrap_or(false);
    let native_tts_ready = native_voice["tts_ready"].as_bool().unwrap_or(false);
    if native_stt_ready && native_tts_ready {
        let engine = native_voice["tts_engine"].as_str().unwrap_or("native");
        ui::kv_ok("Native voice", &format!("ready (whisper-small + {engine})"));
    } else {
        ui::kv_warn("Native voice", "pending");
        ui::hint("Run `captain voice install` to enable local STT/TTS without API keys.");
    }
    let native_embeddings = &body["native_embeddings"];
    if native_embeddings["ready"].as_bool().unwrap_or(false) {
        ui::kv_ok("Embeddings", "local ONNX ready");
    } else {
        ui::kv_warn("Embeddings", "local runtime pending");
        ui::hint("Run `captain embeddings install` to enable semantic memory locally.");
    }
}

fn media_summary(body: &serde_json::Value) -> String {
    let mut media = Vec::new();
    if body["media"]["image_description"]
        .as_bool()
        .unwrap_or(false)
    {
        media.push("images");
    }
    if body["media"]["audio_transcription"]
        .as_bool()
        .unwrap_or(false)
    {
        media.push("audio");
    }
    if body["media"]["video_description"]
        .as_bool()
        .unwrap_or(false)
    {
        media.push("video");
    }
    if media.is_empty() {
        "disabled".to_string()
    } else {
        media.join(", ")
    }
}

fn print_daemon_paths(body: &serde_json::Value, verbose: bool) {
    ui::blank();
    ui::section("Paths");
    ui::kv("Home", body["home_dir"].as_str().unwrap_or("?"));
    ui::kv("Config", body["config_path"].as_str().unwrap_or("?"));
    ui::kv("Logs", body["log_file"].as_str().unwrap_or("?"));

    if verbose {
        ui::blank();
        ui::section("Operational Paths");
        ui::kv("Workspaces", body["workspaces_dir"].as_str().unwrap_or("?"));
        ui::kv("Workflows", body["workflows_dir"].as_str().unwrap_or("?"));
        ui::kv("Sessions", body["sessions_dir"].as_str().unwrap_or("?"));
        ui::blank();
        crate::commands::service::print_service_snapshot(
            &crate::commands::service::service_snapshot(ServiceManagerArg::Auto),
        );
    }
}

fn print_active_agents(body: &serde_json::Value) {
    if let Some(agents) = body["agents"].as_array() {
        if !agents.is_empty() {
            ui::blank();
            ui::section("Active Agents");
            for a in agents {
                println!(
                    "    {} ({}) -- {} [{}:{}]",
                    a["name"].as_str().unwrap_or("?"),
                    a["id"].as_str().unwrap_or("?"),
                    a["state"].as_str().unwrap_or("?"),
                    a["model_provider"].as_str().unwrap_or("?"),
                    a["model_name"].as_str().unwrap_or("?"),
                );
            }
        }
    }
}

fn print_in_process_status(config: Option<PathBuf>, json: bool, verbose: bool) {
    let kernel = boot_kernel(config);
    let agent_count = kernel.registry.count();
    let (llm_driver_ready, llm_driver_error) = kernel.default_llm_driver_status();
    let workload = kernel_status_workload(&kernel);
    let disk = captain_api::status_disk::build_disk_status(&kernel.config.home_dir);
    let provider_subscriptions =
        captain_api::provider_quota_status::build_provider_subscription_status(
            kernel.memory.provider_quotas(),
        );
    let budget = serde_json::json!({
        "provider_subscriptions": provider_subscriptions,
        "operator_actions": []
    });
    let runtime_update = match kernel.runtime_update_snapshot() {
        Ok(snapshot) => serde_json::to_value(snapshot).unwrap_or_else(
            |error| serde_json::json!({"status": "unavailable", "error": error.to_string()}),
        ),
        Err(error) => {
            serde_json::json!({"status": "unavailable", "error": error.to_string()})
        }
    };
    let runtime_health = in_process_runtime_health(llm_driver_ready, &workload, &disk, &budget);
    let status_body = serde_json::json!({
        "status": "in-process",
        "agent_count": agent_count,
        "active_run_count": 0,
        "data_dir": kernel.config.data_dir.display().to_string(),
        "default_provider": kernel.config.default_model.provider.clone(),
        "default_model": kernel.config.default_model.model.clone(),
        "llm_driver_ready": llm_driver_ready,
        "llm_driver_error": llm_driver_error.clone(),
        "daemon": false,
        "shutdown": {"status": "idle", "active_work_count": 0, "active_run_count": 0, "active_process_count": 0, "operator_actions": []},
        "native_embeddings": captain_runtime::native_embeddings::status(),
        "workload": workload,
        "disk": disk,
        "budget": budget,
        "runtime_update": runtime_update,
        "runtime_health": runtime_health,
    });

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status_body).unwrap_or_default()
        );
        return;
    }

    ui::section("Captain Status (in-process)");
    ui::blank();
    ui::kv("Agents", &agent_count.to_string());
    ui::kv("Active runs", "0");
    ui::kv("Provider", &kernel.config.default_model.provider);
    ui::kv("Model", &kernel.config.default_model.model);
    print_llm_status(llm_driver_ready, llm_driver_error.as_deref(), false);
    print_runtime_health_summary(&status_body);
    print_disk_summary(&status_body);
    print_runtime_update_summary(&status_body);
    print_provider_subscription_summary(&status_body["budget"]["provider_subscriptions"], verbose);
    print_in_process_embeddings();
    ui::kv("Data dir", &kernel.config.data_dir.display().to_string());
    ui::kv("Home", &kernel.config.home_dir.display().to_string());
    ui::kv(
        "Config",
        &kernel
            .config
            .home_dir
            .join("config.toml")
            .display()
            .to_string(),
    );
    ui::kv(
        "Logs",
        &kernel
            .config
            .home_dir
            .join("captain.log")
            .display()
            .to_string(),
    );
    ui::kv_warn("Daemon", "NOT RUNNING");
    print_status_workload(&status_body, verbose);
    if verbose {
        print_in_process_operational_paths(&kernel);
    }
    ui::blank();
    ui::hint("Run `captain start` to launch the daemon");

    if agent_count > 0 {
        ui::blank();
        ui::section("Persisted Agents");
        for entry in kernel.registry.list() {
            println!("    {} ({}) -- {:?}", entry.name, entry.id, entry.state);
        }
    }
}

fn print_llm_status(ready: bool, error: Option<&str>, daemon: bool) {
    if ready {
        ui::kv_ok("LLM", "ready");
    } else {
        ui::kv_warn("LLM", "NOT READY");
        if let Some(error) = error.filter(|value| !value.trim().is_empty()) {
            ui::kv("LLM error", error);
        }
        if daemon {
            ui::hint("Corrige le provider LLM puis redemarre Captain.");
        } else {
            ui::hint("Corrige le provider LLM puis relance Captain.");
        }
    }
}

fn print_in_process_embeddings() {
    let native_embeddings = captain_runtime::native_embeddings::status();
    if native_embeddings.ready {
        ui::kv_ok("Embeddings", "local ONNX ready");
    } else {
        ui::kv_warn("Embeddings", "local runtime pending");
        ui::hint("Run `captain embeddings install` to enable semantic memory locally.");
    }
}

fn print_in_process_operational_paths(kernel: &captain_kernel::CaptainKernel) {
    ui::blank();
    ui::section("Operational Paths");
    ui::kv("Home", &kernel.config.home_dir.display().to_string());
    ui::kv(
        "Config",
        &kernel
            .config
            .home_dir
            .join("config.toml")
            .display()
            .to_string(),
    );
    ui::kv(
        "Logs",
        &kernel
            .config
            .home_dir
            .join("captain.log")
            .display()
            .to_string(),
    );
    ui::kv(
        "Workspaces",
        &kernel
            .config
            .workspaces_dir
            .clone()
            .unwrap_or_else(|| kernel.config.home_dir.join("workspaces"))
            .display()
            .to_string(),
    );
    ui::kv(
        "Workflows",
        &kernel
            .config
            .workflows_dir
            .clone()
            .unwrap_or_else(|| kernel.config.home_dir.join("workflows"))
            .display()
            .to_string(),
    );
    ui::blank();
    crate::commands::service::print_service_snapshot(&crate::commands::service::service_snapshot(
        ServiceManagerArg::Auto,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_api_queue_issue_summary_is_hidden_when_clean() {
        let queue = serde_json::json!({
            "readable": true,
            "pending": 0,
            "due": 0,
            "dead_letters": 0,
            "agents_with_queue": 0
        });
        assert!(agent_api_queue_issue_summary(&queue).is_none());
    }

    #[test]
    fn agent_api_queue_issue_summary_reports_operator_counts() {
        let queue = serde_json::json!({
            "readable": true,
            "pending": 2,
            "due": 1,
            "dead_letters": 1,
            "agents_with_queue": 2
        });
        assert_eq!(
            agent_api_queue_issue_summary(&queue).unwrap(),
            "2 pending callback(s), 1 due, 1 dead letter(s), 2 agent(s)"
        );
    }

    #[test]
    fn budget_window_summary_formats_limited_and_unlimited_spend() {
        assert_eq!(
            budget_window_summary(0.25, 1.0, 0.25),
            "$0.2500/$1.00 (25.0%)"
        );
        assert_eq!(budget_window_summary(0.0, 0.0, 0.0), "$0.0000/unlimited");
    }

    #[test]
    fn budget_token_summary_reports_limited_agents() {
        assert_eq!(
            budget_token_summary(1200, 2),
            "1200 used across 2 limited agent(s)"
        );
        assert_eq!(budget_token_summary(0, 0), "0 used; token limits unlimited");
    }

    #[test]
    fn agent_budget_line_shows_usage_limit_and_tool_limit() {
        let agent = serde_json::json!({
            "agent_id": "agent-1",
            "name": "worker",
            "tokens": {"used": 90, "limit": 100, "pct": 0.9},
            "tool_calls": {"used": 4, "limit_per_minute": 10}
        });

        assert_eq!(
            agent_budget_line(&agent),
            "worker (agent-1) -- tokens 90/100 (90.0%), tools 4/10/min"
        );
    }

    #[test]
    fn provider_quota_summary_uses_server_window_durations() {
        let item = serde_json::json!({
            "limit_name": "Codex",
            "plan_type": "pro",
            "alert_level": "warning",
            "primary": {
                "used_percent": 72.5,
                "window_seconds": 18000,
                "resets_at": "2026-07-18T18:00:00Z"
            },
            "secondary": {
                "used_percent": 41.0,
                "window_seconds": 604800
            }
        });

        let summary = provider_quota_item_summary(&item);

        assert!(summary.contains("Codex [pro]"));
        assert!(summary.contains("5h 72.5%"));
        assert!(summary.contains("1w 41.0%"));
        assert!(summary.contains("2026-07-18T18:00:00Z"));
        assert!(summary.contains("[warning]"));
    }

    #[test]
    fn streaming_status_summary_reports_first_token_and_total() {
        let streaming = serde_json::json!({
            "active": 0,
            "completed": 3,
            "last": {
                "surface": "web",
                "first_signal_ms": 120,
                "first_signal_kind": "tool_progress",
                "first_token_ms": 742,
                "total_ms": 3210
            }
        });

        assert_eq!(
            streaming_status_summary(&streaming).unwrap(),
            "active 0, completed 3, last web, first-token 742ms, first-signal tool_progress 120ms, total 3210ms"
        );
    }

    #[test]
    fn streaming_status_summary_handles_no_completed_stream() {
        let streaming = serde_json::json!({
            "active": 1,
            "completed": 0,
            "last": null
        });

        assert_eq!(
            streaming_status_summary(&streaming).unwrap(),
            "active 1, completed 0, waiting for first completed stream"
        );
    }
}
