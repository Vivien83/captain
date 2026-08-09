//! Operator TUI state for durable, selectively projected tool runs.

use captain_runtime::{
    tool_run_operator::{OperatorToolRun, OperatorToolRunTail},
    tool_runs::ToolRunStatus,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;

mod render;

pub use render::{chat_runs_message, draw};

#[cfg(test)]
mod tests;

const REFRESH_TICKS: u16 = 100;
const TAIL_SCROLL_STEP: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunFilter {
    All,
    Running,
    Failed,
    Interrupted,
    Cancelled,
}

impl RunFilter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Running,
        Self::Failed,
        Self::Interrupted,
        Self::Cancelled,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Running => "Running",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
            Self::Cancelled => "Cancelled",
        }
    }

    fn matches(self, status: ToolRunStatus) -> bool {
        match self {
            Self::All => true,
            Self::Running => status == ToolRunStatus::Running,
            Self::Failed => status == ToolRunStatus::Failed,
            Self::Interrupted => status == ToolRunStatus::Interrupted,
            Self::Cancelled => status == ToolRunStatus::Cancelled,
        }
    }

    fn shifted(self, delta: isize) -> Self {
        let current = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        let next = if delta < 0 {
            current.checked_sub(1).unwrap_or(Self::ALL.len() - 1)
        } else {
            (current + 1) % Self::ALL.len()
        };
        Self::ALL[next]
    }
}

pub struct LiveRunsState {
    pub items: Vec<OperatorToolRun>,
    pub list_state: ListState,
    pub filter: RunFilter,
    pub tail: Option<OperatorToolRunTail>,
    pub loading_runs: bool,
    pub error: String,
    loading_tail_for: Option<String>,
    confirm_cancel_for: Option<String>,
    cancelling_for: Option<String>,
    tail_offset_from_end: usize,
    tick: usize,
    refresh_ticks: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveRunsAction {
    Continue,
    Close,
    Refresh,
    LoadTail(String),
    Cancel(String),
}

impl LiveRunsState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            list_state: ListState::default(),
            filter: RunFilter::All,
            tail: None,
            loading_runs: false,
            error: String::new(),
            loading_tail_for: None,
            confirm_cancel_for: None,
            cancelling_for: None,
            tail_offset_from_end: 0,
            tick: 0,
            refresh_ticks: 0,
        }
    }

    pub fn begin_runs_load(&mut self) {
        self.loading_runs = true;
        self.refresh_ticks = 0;
    }

    pub fn apply_runs(&mut self, result: Result<Vec<OperatorToolRun>, String>) -> Option<String> {
        self.loading_runs = false;
        match result {
            Ok(items) => {
                let selected_id = self.selected_run().map(|run| run.run_id.clone());
                self.items = items;
                self.error.clear();
                self.select_run_id(selected_id.as_deref());
                let selected_id = self.selected_run().map(|run| run.run_id.clone());
                if self.tail.as_ref().map(|tail| tail.run_id.as_str()) != selected_id.as_deref() {
                    self.tail = None;
                    self.tail_offset_from_end = 0;
                }
                if self.loading_tail_for.as_deref() != selected_id.as_deref() {
                    self.loading_tail_for = None;
                }
                if !self.cancel_target_is_valid(self.confirm_cancel_for.as_deref()) {
                    self.confirm_cancel_for = None;
                }
                selected_id
            }
            Err(error) => {
                self.error = error;
                None
            }
        }
    }

    pub fn begin_tail_load(&mut self, run_id: &str) -> bool {
        if !self.selected_run().is_some_and(|run| run.run_id == run_id)
            || self.loading_tail_for.as_deref() == Some(run_id)
        {
            return false;
        }
        self.loading_tail_for = Some(run_id.to_string());
        true
    }

    pub fn apply_tail(&mut self, run_id: &str, result: Result<OperatorToolRunTail, String>) {
        if !self.selected_run().is_some_and(|run| run.run_id == run_id)
            || self.loading_tail_for.as_deref() != Some(run_id)
        {
            return;
        }
        self.loading_tail_for = None;
        match result {
            Ok(tail) if tail.run_id == run_id => {
                self.tail = Some(tail);
                self.tail_offset_from_end = 0;
                self.error.clear();
            }
            Ok(_) => {
                self.tail = None;
                self.tail_offset_from_end = 0;
                self.error = "Tool run tail identity mismatch".to_string();
            }
            Err(error) => {
                self.tail = None;
                self.tail_offset_from_end = 0;
                self.error = error;
            }
        }
    }

    pub fn apply_cancel(
        &mut self,
        run_id: &str,
        result: Result<OperatorToolRun, String>,
    ) -> Option<String> {
        if self.cancelling_for.as_deref() != Some(run_id) {
            return None;
        }
        self.cancelling_for = None;
        self.confirm_cancel_for = None;
        match result {
            Ok(updated) if updated.run_id == run_id => {
                if let Some(item) = self.items.iter_mut().find(|item| item.run_id == run_id) {
                    *item = updated;
                }
                if let Some(tail) = self.tail.as_mut().filter(|tail| tail.run_id == run_id) {
                    tail.status = ToolRunStatus::Cancelled;
                }
                self.error.clear();
                Some(run_id.to_string())
            }
            Ok(_) => {
                self.error = "Tool run cancellation identity mismatch".to_string();
                None
            }
            Err(error) => {
                self.error = error;
                None
            }
        }
    }

    pub fn tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        if self.loading_runs {
            return false;
        }
        self.refresh_ticks = self.refresh_ticks.saturating_add(1);
        if self.refresh_ticks < REFRESH_TICKS {
            return false;
        }
        self.refresh_ticks = 0;
        true
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> LiveRunsAction {
        if self.confirm_cancel_for.is_some() {
            return match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_cancel(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.confirm_cancel_for = None;
                    LiveRunsAction::Continue
                }
                _ => LiveRunsAction::Continue,
            };
        }
        match key.code {
            KeyCode::Esc => LiveRunsAction::Close,
            KeyCode::Char('r') => LiveRunsAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Left | KeyCode::Char('h') => self.shift_filter(-1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('f') => self.shift_filter(1),
            KeyCode::Enter | KeyCode::Char('t') => self
                .selected_run()
                .map(|run| LiveRunsAction::LoadTail(run.run_id.clone()))
                .unwrap_or(LiveRunsAction::Continue),
            KeyCode::PageUp => {
                self.tail_offset_from_end = self
                    .tail_offset_from_end
                    .saturating_add(TAIL_SCROLL_STEP)
                    .min(self.tail_line_count());
                LiveRunsAction::Continue
            }
            KeyCode::PageDown => {
                self.tail_offset_from_end =
                    self.tail_offset_from_end.saturating_sub(TAIL_SCROLL_STEP);
                LiveRunsAction::Continue
            }
            KeyCode::Home => {
                self.tail_offset_from_end = self.tail_line_count().saturating_sub(1);
                LiveRunsAction::Continue
            }
            KeyCode::End => {
                self.tail_offset_from_end = 0;
                LiveRunsAction::Continue
            }
            KeyCode::Char('x') => {
                self.arm_cancel();
                LiveRunsAction::Continue
            }
            _ => LiveRunsAction::Continue,
        }
    }

    pub fn selected_run(&self) -> Option<&OperatorToolRun> {
        let visible_index = self.list_state.selected()?;
        let item_index = self.visible_indices().get(visible_index).copied()?;
        self.items.get(item_index)
    }

    fn visible_indices(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| self.filter.matches(item.status).then_some(index))
            .collect()
    }

    fn select_run_id(&mut self, run_id: Option<&str>) {
        let visible = self.visible_indices();
        let selected = run_id
            .and_then(|run_id| {
                visible
                    .iter()
                    .position(|index| self.items[*index].run_id == run_id)
            })
            .or((!visible.is_empty()).then_some(0));
        self.list_state.select(selected);
    }

    fn move_selection(&mut self, delta: isize) -> LiveRunsAction {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return LiveRunsAction::Continue;
        }
        let current = self
            .list_state
            .selected()
            .unwrap_or(0)
            .min(visible.len() - 1);
        let next = if delta < 0 {
            current.checked_sub(1).unwrap_or(visible.len() - 1)
        } else {
            (current + 1) % visible.len()
        };
        self.list_state.select(Some(next));
        self.selection_changed();
        LiveRunsAction::LoadTail(self.items[visible[next]].run_id.clone())
    }

    fn shift_filter(&mut self, delta: isize) -> LiveRunsAction {
        let selected_id = self.selected_run().map(|run| run.run_id.clone());
        self.filter = self.filter.shifted(delta);
        self.select_run_id(selected_id.as_deref());
        self.selection_changed();
        self.selected_run()
            .map(|run| LiveRunsAction::LoadTail(run.run_id.clone()))
            .unwrap_or(LiveRunsAction::Continue)
    }

    fn selection_changed(&mut self) {
        self.tail = None;
        self.loading_tail_for = None;
        self.confirm_cancel_for = None;
        self.tail_offset_from_end = 0;
        self.error.clear();
    }

    fn arm_cancel(&mut self) {
        let Some((run_id, valid)) = self.selected_run().map(|run| {
            (
                run.run_id.clone(),
                run.status == ToolRunStatus::Running && run.cancellable,
            )
        }) else {
            return;
        };
        if valid && self.cancelling_for.is_none() {
            self.confirm_cancel_for = Some(run_id);
            self.error.clear();
        } else {
            self.error = "Only an active run with a live cancel handle can be stopped".to_string();
        }
    }

    fn confirm_cancel(&mut self) -> LiveRunsAction {
        let Some(run_id) = self.confirm_cancel_for.take() else {
            return LiveRunsAction::Continue;
        };
        if !self.cancel_target_is_valid(Some(&run_id)) || self.cancelling_for.is_some() {
            self.error = "The selected run is no longer cancellable".to_string();
            return LiveRunsAction::Continue;
        }
        self.cancelling_for = Some(run_id.clone());
        LiveRunsAction::Cancel(run_id)
    }

    fn cancel_target_is_valid(&self, run_id: Option<&str>) -> bool {
        let Some(run_id) = run_id else {
            return false;
        };
        self.items.iter().any(|run| {
            run.run_id == run_id && run.status == ToolRunStatus::Running && run.cancellable
        })
    }

    fn tail_line_count(&self) -> usize {
        self.tail
            .as_ref()
            .map(|tail| tail.content.lines().count())
            .unwrap_or(0)
    }
}
