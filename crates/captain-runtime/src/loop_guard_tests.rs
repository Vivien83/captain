use super::*;

#[test]
fn allow_below_threshold() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"query": "test"});
    let v = guard.check("web_search", &params);
    assert_eq!(v, LoopGuardVerdict::Allow);
    let v = guard.check("web_search", &params);
    assert_eq!(v, LoopGuardVerdict::Allow);
}

#[test]
fn warn_at_threshold() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"path": "/etc/passwd"});
    guard.check("file_read", &params);
    guard.check("file_read", &params);
    let v = guard.check("file_read", &params);
    assert!(matches!(v, LoopGuardVerdict::Warn(_)));
}

#[test]
fn block_at_threshold() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"command": "ls"});
    for _ in 0..4 {
        guard.check("shell_exec", &params);
    }
    let v = guard.check("shell_exec", &params);
    assert!(matches!(v, LoopGuardVerdict::Block(_)));
}

#[test]
fn different_params_no_collision() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    for i in 0..10 {
        let params = serde_json::json!({"query": format!("query_{}", i)});
        let v = guard.check("web_search", &params);
        assert_eq!(v, LoopGuardVerdict::Allow);
    }
}

#[test]
fn global_circuit_breaker() {
    let config = LoopGuardConfig {
        warn_threshold: 100,
        block_threshold: 100,
        global_circuit_breaker: 5,
        ..Default::default()
    };
    let mut guard = LoopGuard::new(config);
    for i in 0..5 {
        let params = serde_json::json!({"n": i});
        let v = guard.check("tool", &params);
        assert_eq!(v, LoopGuardVerdict::Allow);
    }
    let v = guard.check("tool", &serde_json::json!({"n": 5}));
    assert!(matches!(v, LoopGuardVerdict::CircuitBreak(_)));
}

#[test]
fn default_config() {
    let config = LoopGuardConfig::default();
    assert_eq!(config.warn_threshold, 3);
    assert_eq!(config.block_threshold, 5);
    assert_eq!(config.global_circuit_breaker, 30);
}

#[test]
fn test_outcome_aware_warning() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"query": "weather"});
    let result = "sunny 72F";

    let w = guard.record_outcome("web_search", &params, result);
    assert!(w.is_none());

    let w = guard.record_outcome("web_search", &params, result);
    assert!(w.is_some());
    assert!(w.unwrap().contains("identical results"));
}

#[test]
fn test_outcome_aware_blocks_next_call() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"query": "weather"});
    let result = "sunny 72F";

    guard.record_outcome("web_search", &params, result);
    guard.record_outcome("web_search", &params, result);
    let w = guard.record_outcome("web_search", &params, result);
    assert!(w.is_some());

    let v = guard.check("web_search", &params);
    assert!(matches!(v, LoopGuardVerdict::Block(_)));
    if let LoopGuardVerdict::Block(msg) = v {
        assert!(msg.contains("identical results"));
    }
}

#[test]
fn test_ping_pong_ab_detection() {
    let mut guard = LoopGuard::new(LoopGuardConfig {
        warn_threshold: 100,
        block_threshold: 100,
        ping_pong_min_repeats: 3,
        ..Default::default()
    });
    let params_a = serde_json::json!({"file": "a.txt"});
    let params_b = serde_json::json!({"file": "b.txt"});

    guard.check("file_read", &params_a);
    guard.check("file_write", &params_b);
    guard.check("file_read", &params_a);
    guard.check("file_write", &params_b);
    guard.check("file_read", &params_a);
    let v = guard.check("file_write", &params_b);

    assert!(
        matches!(v, LoopGuardVerdict::Block(ref msg) if msg.contains("Ping-pong"))
            || matches!(v, LoopGuardVerdict::Warn(ref msg) if msg.contains("Ping-pong")),
        "Expected ping-pong detection, got: {:?}",
        v
    );
}

#[test]
fn test_ping_pong_abc_detection() {
    let mut guard = LoopGuard::new(LoopGuardConfig {
        warn_threshold: 100,
        block_threshold: 100,
        ping_pong_min_repeats: 3,
        ..Default::default()
    });
    let params_a = serde_json::json!({"a": 1});
    let params_b = serde_json::json!({"b": 2});
    let params_c = serde_json::json!({"c": 3});

    for _ in 0..3 {
        guard.check("tool_a", &params_a);
        guard.check("tool_b", &params_b);
        guard.check("tool_c", &params_c);
    }

    let stats = guard.stats();
    assert!(stats.ping_pong_detected);
}

#[test]
fn test_no_false_ping_pong() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());

    for i in 0..10 {
        let params = serde_json::json!({"n": i});
        guard.check("tool", &params);
    }

    let stats = guard.stats();
    assert!(!stats.ping_pong_detected);
}

#[test]
fn test_poll_tool_relaxed_thresholds() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"command": "docker ps --status running"});

    for _ in 0..8 {
        let v = guard.check("shell_exec", &params);
        assert_eq!(
            v,
            LoopGuardVerdict::Allow,
            "Poll tool should have relaxed thresholds"
        );
    }

    let v = guard.check("shell_exec", &params);
    assert!(
        matches!(v, LoopGuardVerdict::Warn(_)),
        "Expected warn at poll threshold, got: {:?}",
        v
    );
}

#[test]
fn test_is_poll_call_detection() {
    assert!(LoopGuard::is_poll_call(
        "shell_exec",
        &serde_json::json!({"command": "docker ps --status"})
    ));
    assert!(LoopGuard::is_poll_call(
        "shell_exec",
        &serde_json::json!({"command": "tail -f /var/log/app.log"})
    ));
    assert!(!LoopGuard::is_poll_call(
        "shell_exec",
        &serde_json::json!({"command": "echo hi"})
    ));
    assert!(!LoopGuard::is_poll_call(
        "shell_exec",
        &serde_json::json!({"command": "this is a very long command that definitely exceeds fifty characters in length"})
    ));
    assert!(!LoopGuard::is_poll_call(
        "file_read",
        &serde_json::json!({"path": "/etc/hosts"})
    ));
    assert!(LoopGuard::is_poll_call(
        "some_tool",
        &serde_json::json!({"check": "status"})
    ));
    assert!(LoopGuard::is_poll_call(
        "api_call",
        &serde_json::json!({"action": "poll_results"})
    ));
    assert!(LoopGuard::is_poll_call(
        "queue",
        &serde_json::json!({"mode": "wait_for_completion"})
    ));
}

#[test]
fn test_poll_backoff_schedule() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params = serde_json::json!({"command": "kubectl get pods --status"});

    assert_eq!(guard.get_poll_backoff("shell_exec", &params), None);
    assert_eq!(guard.get_poll_backoff("shell_exec", &params), Some(5000));
    assert_eq!(guard.get_poll_backoff("shell_exec", &params), Some(10000));
    assert_eq!(guard.get_poll_backoff("shell_exec", &params), Some(30000));
    assert_eq!(guard.get_poll_backoff("shell_exec", &params), Some(60000));
    assert_eq!(guard.get_poll_backoff("shell_exec", &params), Some(60000));

    let non_poll = serde_json::json!({"path": "/etc/hosts"});
    assert_eq!(guard.get_poll_backoff("file_read", &non_poll), None);
}

#[test]
fn test_warning_bucket_limits() {
    let mut guard = LoopGuard::new(LoopGuardConfig {
        warn_threshold: 2,
        block_threshold: 100,
        max_warnings_per_call: 2,
        ..Default::default()
    });
    let params = serde_json::json!({"x": 1});

    assert_eq!(guard.check("tool", &params), LoopGuardVerdict::Allow);
    assert!(matches!(
        guard.check("tool", &params),
        LoopGuardVerdict::Warn(_)
    ));
    assert!(matches!(
        guard.check("tool", &params),
        LoopGuardVerdict::Warn(_)
    ));
    assert!(matches!(
        guard.check("tool", &params),
        LoopGuardVerdict::Block(_)
    ));
}

#[test]
fn test_warning_upgrade_to_block() {
    let mut guard = LoopGuard::new(LoopGuardConfig {
        warn_threshold: 1,
        block_threshold: 100,
        max_warnings_per_call: 1,
        ..Default::default()
    });
    let params = serde_json::json!({"y": 2});

    let v = guard.check("tool", &params);
    assert!(matches!(v, LoopGuardVerdict::Warn(_)));

    let v = guard.check("tool", &params);
    assert!(
        matches!(v, LoopGuardVerdict::Block(ref msg) if msg.contains("warnings exhausted")),
        "Expected block with 'warnings exhausted', got: {:?}",
        v
    );
}

#[test]
fn test_stats_snapshot() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());
    let params_a = serde_json::json!({"a": 1});
    let params_b = serde_json::json!({"b": 2});

    guard.check("tool_a", &params_a);
    guard.check("tool_a", &params_a);
    guard.check("tool_a", &params_a);
    guard.check("tool_b", &params_b);

    let stats = guard.stats();
    assert_eq!(stats.total_calls, 4);
    assert_eq!(stats.unique_calls, 2);
    assert_eq!(stats.most_repeated_tool, Some("tool_a".to_string()));
    assert_eq!(stats.most_repeated_count, 3);
    assert!(!stats.ping_pong_detected);
}

#[test]
fn test_history_ring_buffer_limit() {
    let config = LoopGuardConfig {
        warn_threshold: 100,
        block_threshold: 100,
        global_circuit_breaker: 200,
        ..Default::default()
    };
    let mut guard = LoopGuard::new(config);

    for i in 0..50 {
        let params = serde_json::json!({"n": i});
        guard.check("tool", &params);
    }

    assert_eq!(guard.recent_calls.len(), HISTORY_SIZE);

    let stats = guard.stats();
    assert_eq!(stats.total_calls, 50);
    assert_eq!(stats.unique_calls, 50);
}

/// Reproduces the live incident this fix addresses: a tool fails repeatedly
/// while the caller varies an unrelated argument (e.g. a name) on every
/// call, hoping one attempt slips through. Per-hash detection alone never
/// fires here since every (tool_name, params) hash is unique.
#[test]
fn consecutive_errors_with_varying_params_eventually_block() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());

    let block_threshold = LoopGuardConfig::default().consecutive_error_block_threshold;
    for i in 0..block_threshold {
        let params = serde_json::json!({"name": format!("demo-compteur-{i}")});
        let verdict = guard.check("agent_spawn", &params);
        assert_eq!(
            verdict,
            LoopGuardVerdict::Allow,
            "attempt {i} should still be allowed"
        );
        let warning = guard.record_tool_error("agent_spawn", true);
        if i + 1 >= LoopGuardConfig::default().consecutive_error_warn_threshold {
            assert!(
                warning.is_some(),
                "attempt {i} should carry a streak warning"
            );
        }
    }

    // The next distinct attempt is blocked pre-execution, before it ever
    // reaches the tool — regardless of its (also unique) params.
    let params = serde_json::json!({"name": "demo-compteur-final"});
    let verdict = guard.check("agent_spawn", &params);
    assert!(
        matches!(verdict, LoopGuardVerdict::Block(_)),
        "got: {verdict:?}"
    );
}

#[test]
fn a_success_resets_the_consecutive_error_streak() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());

    for i in 0..4 {
        let params = serde_json::json!({"name": format!("demo-{i}")});
        guard.check("agent_spawn", &params);
        guard.record_tool_error("agent_spawn", true);
    }

    // A success clears the streak entirely.
    guard.check("agent_spawn", &serde_json::json!({"name": "demo-ok"}));
    guard.record_tool_error("agent_spawn", false);

    // So the next failing attempt starts counting from zero again instead
    // of being blocked immediately.
    let verdict = guard.check(
        "agent_spawn",
        &serde_json::json!({"name": "demo-after-reset"}),
    );
    assert_eq!(verdict, LoopGuardVerdict::Allow);
}

#[test]
fn consecutive_errors_of_different_tools_are_tracked_independently() {
    let mut guard = LoopGuard::new(LoopGuardConfig::default());

    for i in 0..5 {
        guard.check("agent_spawn", &serde_json::json!({"name": format!("a{i}")}));
        guard.record_tool_error("agent_spawn", true);
    }
    // agent_spawn is now blocked, but an unrelated tool must be unaffected.
    let verdict = guard.check("process_start", &serde_json::json!({"command": "sleep"}));
    assert_eq!(verdict, LoopGuardVerdict::Allow);
}
