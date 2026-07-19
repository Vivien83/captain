use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn pending() -> NativeCapabilityInfo {
    NativeCapabilityInfo {
        name: "deploy-preview".into(),
        scope: "global".into(),
        status: "pending_approval".into(),
        pending_hash: Some("full-pending-hash".into()),
        human_action_required: true,
        revisions: vec![
            NativeRevision {
                source_hash: "full-pending-hash".into(),
                version: "2".into(),
                ..Default::default()
            },
            NativeRevision {
                source_hash: "old-active-hash".into(),
                version: "1".into(),
                approved_by: Some("operator".into()),
                ..Default::default()
            },
            NativeRevision {
                source_hash: "older-approved-hash".into(),
                version: "0.9".into(),
                approved_by: Some("operator".into()),
                ..Default::default()
            },
        ],
        active_hash: Some("old-active-hash".into()),
        ..Default::default()
    }
}

fn uncertain_run() -> NativeRunInfo {
    NativeRunInfo {
        run_id: "full-run-id".into(),
        capability_name: "deploy-preview".into(),
        source_hash: "full-source-hash".into(),
        status: "waiting_decision".into(),
        origin: "telegram".into(),
        nodes: vec![NativeRunNode {
            step_id: "deploy".into(),
            tool_name: "shell_exec".into(),
            status: "uncertain".into(),
            attempts: 2,
            tool_use_id: Some("capspec:full-run-id:deploy:2".into()),
        }],
    }
}

#[test]
fn pending_capabilities_are_sorted_before_ready_entries() {
    let mut state = NativeCapabilitiesState::new();
    let ready = NativeCapabilityInfo {
        name: "alpha".into(),
        ready: true,
        status: "operational".into(),
        ..Default::default()
    };
    state.replace(vec![ready, pending()], vec![]);
    assert_eq!(state.capabilities[0].name, "deploy-preview");
}

#[test]
fn approve_and_reject_keep_the_full_pending_hash() {
    let mut state = NativeCapabilitiesState::new();
    state.replace(vec![pending()], vec![]);
    assert_eq!(
        state.handle_key(key(KeyCode::Char('a'))),
        NativeCapabilitiesAction::Decide {
            name: "deploy-preview".into(),
            scope: "global".into(),
            expected_hash: "full-pending-hash".into(),
            approve: true,
        }
    );
    assert_eq!(
        state.handle_key(key(KeyCode::Char('x'))),
        NativeCapabilitiesAction::Decide {
            name: "deploy-preview".into(),
            scope: "global".into(),
            expected_hash: "full-pending-hash".into(),
            approve: false,
        }
    );
}

#[test]
fn rollback_targets_the_explicitly_selected_non_active_revision() {
    let mut state = NativeCapabilitiesState::new();
    state.replace(vec![pending()], vec![]);
    state.handle_key(key(KeyCode::Right));
    assert_eq!(
        state.handle_key(key(KeyCode::Char('b'))),
        NativeCapabilitiesAction::Continue
    );
    state.handle_key(key(KeyCode::Right));
    assert_eq!(
        state.handle_key(key(KeyCode::Char('b'))),
        NativeCapabilitiesAction::Rollback {
            name: "deploy-preview".into(),
            scope: "global".into(),
            target_hash: "older-approved-hash".into(),
        }
    );
}

#[test]
fn disable_requires_a_second_confirmation_key() {
    let mut state = NativeCapabilitiesState::new();
    state.replace(vec![pending()], vec![]);
    assert_eq!(
        state.handle_key(key(KeyCode::Char('d'))),
        NativeCapabilitiesAction::Continue
    );
    assert!(state.confirm_disable);
    assert_eq!(
        state.handle_key(key(KeyCode::Char('y'))),
        NativeCapabilitiesAction::Disable {
            name: "deploy-preview".into(),
            scope: "global".into(),
        }
    );
}

#[test]
fn uncertain_run_decision_requires_confirmation_and_keeps_exact_identity() {
    let mut state = NativeCapabilitiesState::new();
    state.replace(vec![pending()], vec![uncertain_run()]);
    assert_eq!(
        state.handle_key(key(KeyCode::Char('T'))),
        NativeCapabilitiesAction::Continue
    );
    assert_eq!(state.confirm_run, Some(NativeRunDecision::Retry));
    assert_eq!(
        state.handle_key(key(KeyCode::Char('y'))),
        NativeCapabilitiesAction::ResolveRun {
            run_id: "full-run-id".into(),
            node_id: "deploy".into(),
            tool_use_id: "capspec:full-run-id:deploy:2".into(),
            attempt: 2,
            decision: NativeRunDecision::Retry,
        }
    );
}

#[test]
fn cancelled_run_decision_never_emits_a_mutation() {
    let mut state = NativeCapabilitiesState::new();
    state.replace(vec![], vec![uncertain_run()]);
    state.handle_key(key(KeyCode::Char('F')));
    assert_eq!(
        state.handle_key(key(KeyCode::Esc)),
        NativeCapabilitiesAction::Continue
    );
    assert_eq!(state.confirm_run, None);
}

#[test]
fn native_capabilities_render_in_wide_and_compact_terminals() {
    for (width, height) in [(120, 32), (72, 36)] {
        let mut state = NativeCapabilitiesState::new();
        state.replace(vec![pending()], vec![uncertain_run()]);
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| draw(frame, frame.area(), &mut state))
            .expect("draw native capabilities");

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Native capabilities"));
        assert!(rendered.contains("deploy-preview"));
        assert!(rendered.contains("APPROVAL"));
        assert!(rendered.contains("UNCERTAIN RUN"));
        assert!(rendered.contains("shell_exec"));
    }
}
