//! Native CapSpec operator screen: exact-hash decisions and durable revisions.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

mod render;
pub use render::draw;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeScope {
    Effective,
    Global,
    Project,
}

impl NativeScope {
    pub fn query(self) -> &'static str {
        match self {
            Self::Effective => "effective",
            Self::Global => "global",
            Self::Project => "project",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Effective => "effective",
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeRevision {
    pub source_hash: String,
    pub version: String,
    pub approved_by: Option<String>,
    pub rejected_by: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeCapabilityInfo {
    pub name: String,
    pub tool_name: String,
    pub description: String,
    pub version: String,
    pub status: String,
    pub scope: String,
    pub ready: bool,
    pub human_action_required: bool,
    pub active_hash: Option<String>,
    pub pending_hash: Option<String>,
    pub selected_hash: Option<String>,
    pub permission_fingerprint: String,
    pub tools: Vec<String>,
    pub revisions: Vec<NativeRevision>,
    pub source: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeRunInfo {
    pub run_id: String,
    pub capability_name: String,
    pub source_hash: String,
    pub status: String,
    pub origin: String,
    pub nodes: Vec<NativeRunNode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeRunNode {
    pub step_id: String,
    pub tool_name: String,
    pub status: String,
    pub attempts: u32,
    pub tool_use_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRunDecision {
    ConfirmSucceeded,
    Retry,
    MarkFailed,
}

pub struct NativeCapabilitiesState {
    pub scope: NativeScope,
    pub capabilities: Vec<NativeCapabilityInfo>,
    pub runs: Vec<NativeRunInfo>,
    pub list_state: ListState,
    pub revision_index: usize,
    pub run_index: usize,
    pub loading: bool,
    pub source_visible: bool,
    pub confirm_disable: bool,
    pub confirm_run: Option<NativeRunDecision>,
    pub status_msg: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NativeCapabilitiesAction {
    Continue,
    Refresh,
    Inspect {
        name: String,
        scope: String,
        include_source: bool,
    },
    Decide {
        name: String,
        scope: String,
        expected_hash: String,
        approve: bool,
    },
    Rollback {
        name: String,
        scope: String,
        target_hash: String,
    },
    Disable {
        name: String,
        scope: String,
    },
    ResolveRun {
        run_id: String,
        node_id: String,
        tool_use_id: String,
        attempt: u32,
        decision: NativeRunDecision,
    },
}

impl NativeCapabilitiesState {
    pub fn new() -> Self {
        Self {
            scope: NativeScope::Effective,
            capabilities: Vec::new(),
            runs: Vec::new(),
            list_state: ListState::default(),
            revision_index: 0,
            run_index: 0,
            loading: false,
            source_visible: false,
            confirm_disable: false,
            confirm_run: None,
            status_msg: String::new(),
        }
    }

    pub fn replace(
        &mut self,
        mut capabilities: Vec<NativeCapabilityInfo>,
        runs: Vec<NativeRunInfo>,
    ) {
        let selected_name = self.selected().map(|item| item.name.clone());
        capabilities.sort_by(|left, right| {
            capability_priority(left)
                .cmp(&capability_priority(right))
                .then_with(|| left.name.cmp(&right.name))
        });
        self.capabilities = capabilities;
        self.runs = runs;
        let waiting = self.uncertain_run_count();
        self.run_index = self.run_index.min(waiting.saturating_sub(1));
        let selected = selected_name
            .and_then(|name| self.capabilities.iter().position(|item| item.name == name))
            .or((!self.capabilities.is_empty()).then_some(0));
        self.list_state.select(selected);
        self.revision_index = 0;
        self.loading = false;
    }

    pub fn replace_inspected(&mut self, inspected: NativeCapabilityInfo) {
        if let Some(index) = self
            .capabilities
            .iter()
            .position(|item| item.name == inspected.name && item.scope == inspected.scope)
        {
            self.capabilities[index] = inspected;
            self.list_state.select(Some(index));
        }
        self.loading = false;
    }

    pub fn selected(&self) -> Option<&NativeCapabilityInfo> {
        self.list_state
            .selected()
            .and_then(|index| self.capabilities.get(index))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NativeCapabilitiesAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return NativeCapabilitiesAction::Continue;
        }
        if self.confirm_disable {
            return match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_disable = false;
                    self.selected_action(|item| NativeCapabilitiesAction::Disable {
                        name: item.name.clone(),
                        scope: item.scope.clone(),
                    })
                }
                _ => {
                    self.confirm_disable = false;
                    NativeCapabilitiesAction::Continue
                }
            };
        }
        if let Some(decision) = self.confirm_run.take() {
            return match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.run_decision_action(decision),
                _ => NativeCapabilitiesAction::Continue,
            };
        }
        match key.code {
            KeyCode::Char('1') => return self.switch_scope(NativeScope::Effective),
            KeyCode::Char('2') => return self.switch_scope(NativeScope::Global),
            KeyCode::Char('3') => return self.switch_scope(NativeScope::Project),
            KeyCode::Char('r') => return NativeCapabilitiesAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(true),
            KeyCode::Left | KeyCode::Char('h') => self.move_revision(false),
            KeyCode::Right | KeyCode::Char('l') => self.move_revision(true),
            KeyCode::Char('[') => self.move_run(false),
            KeyCode::Char(']') => self.move_run(true),
            KeyCode::Enter => {
                return self.selected_action(|item| NativeCapabilitiesAction::Inspect {
                    name: item.name.clone(),
                    scope: item.scope.clone(),
                    include_source: false,
                });
            }
            KeyCode::Char('v') => {
                let include_source = !self.source_visible;
                self.source_visible = include_source;
                if include_source
                    && self
                        .selected()
                        .and_then(|item| item.source.as_ref())
                        .is_none()
                {
                    return self.selected_action(|item| NativeCapabilitiesAction::Inspect {
                        name: item.name.clone(),
                        scope: item.scope.clone(),
                        include_source: true,
                    });
                }
            }
            KeyCode::Char('a') => return self.decision_action(true),
            KeyCode::Char('x') => return self.decision_action(false),
            KeyCode::Char('b') => return self.rollback_action(),
            KeyCode::Char('d') | KeyCode::Delete => {
                if self.selected().is_some() {
                    self.confirm_disable = true;
                }
            }
            KeyCode::Char('T') => self.arm_run_decision(NativeRunDecision::Retry),
            KeyCode::Char('C') => self.arm_run_decision(NativeRunDecision::ConfirmSucceeded),
            KeyCode::Char('F') => self.arm_run_decision(NativeRunDecision::MarkFailed),
            _ => {}
        }
        NativeCapabilitiesAction::Continue
    }

    fn switch_scope(&mut self, scope: NativeScope) -> NativeCapabilitiesAction {
        self.scope = scope;
        self.loading = true;
        self.status_msg.clear();
        NativeCapabilitiesAction::Refresh
    }

    fn move_selection(&mut self, forward: bool) {
        if self.capabilities.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        let next = if forward {
            (current + 1) % self.capabilities.len()
        } else if current == 0 {
            self.capabilities.len() - 1
        } else {
            current - 1
        };
        self.list_state.select(Some(next));
        self.revision_index = 0;
        self.source_visible = false;
        self.confirm_disable = false;
        self.confirm_run = None;
    }

    fn move_run(&mut self, forward: bool) {
        let total = self.uncertain_run_count();
        if total == 0 {
            return;
        }
        self.run_index = if forward {
            (self.run_index + 1) % total
        } else if self.run_index == 0 {
            total - 1
        } else {
            self.run_index - 1
        };
        self.confirm_run = None;
    }

    fn arm_run_decision(&mut self, decision: NativeRunDecision) {
        if self.selected_uncertain().is_some() {
            self.confirm_run = Some(decision);
        }
    }

    pub fn selected_uncertain(&self) -> Option<(&NativeRunInfo, &NativeRunNode)> {
        self.runs
            .iter()
            .filter_map(|run| {
                (run.status == "waiting_decision")
                    .then(|| {
                        run.nodes
                            .iter()
                            .find(|node| node.status == "uncertain")
                            .map(|node| (run, node))
                    })
                    .flatten()
            })
            .nth(self.run_index)
    }

    pub fn uncertain_run_count(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| {
                run.status == "waiting_decision"
                    && run.nodes.iter().any(|node| node.status == "uncertain")
            })
            .count()
    }

    fn run_decision_action(&self, decision: NativeRunDecision) -> NativeCapabilitiesAction {
        self.selected_uncertain()
            .and_then(|(run, node)| {
                node.tool_use_id
                    .as_ref()
                    .map(|tool_use_id| NativeCapabilitiesAction::ResolveRun {
                        run_id: run.run_id.clone(),
                        node_id: node.step_id.clone(),
                        tool_use_id: tool_use_id.clone(),
                        attempt: node.attempts,
                        decision,
                    })
            })
            .unwrap_or(NativeCapabilitiesAction::Continue)
    }

    fn move_revision(&mut self, forward: bool) {
        let total = self
            .selected()
            .map(|item| item.revisions.len())
            .unwrap_or(0);
        if total == 0 {
            return;
        }
        self.revision_index = if forward {
            (self.revision_index + 1) % total
        } else if self.revision_index == 0 {
            total - 1
        } else {
            self.revision_index - 1
        };
    }

    fn decision_action(&self, approve: bool) -> NativeCapabilitiesAction {
        self.selected_action(|item| match item.pending_hash.as_ref() {
            Some(hash) if item.human_action_required => NativeCapabilitiesAction::Decide {
                name: item.name.clone(),
                scope: item.scope.clone(),
                expected_hash: hash.clone(),
                approve,
            },
            _ => NativeCapabilitiesAction::Continue,
        })
    }

    fn rollback_action(&self) -> NativeCapabilitiesAction {
        self.selected_action(|item| {
            item.revisions
                .get(self.revision_index)
                .filter(|revision| {
                    revision.approved_by.is_some()
                        && Some(&revision.source_hash) != item.active_hash.as_ref()
                })
                .map(|revision| NativeCapabilitiesAction::Rollback {
                    name: item.name.clone(),
                    scope: item.scope.clone(),
                    target_hash: revision.source_hash.clone(),
                })
                .unwrap_or(NativeCapabilitiesAction::Continue)
        })
    }

    fn selected_action(
        &self,
        action: impl FnOnce(&NativeCapabilityInfo) -> NativeCapabilitiesAction,
    ) -> NativeCapabilitiesAction {
        self.selected()
            .map(action)
            .unwrap_or(NativeCapabilitiesAction::Continue)
    }
}

fn capability_priority(item: &NativeCapabilityInfo) -> u8 {
    if item.human_action_required {
        0
    } else if item.ready {
        1
    } else {
        2
    }
}

#[cfg(test)]
#[path = "native_capabilities/tests.rs"]
mod tests;
