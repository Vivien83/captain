//! Projects screen: Project OS lifecycle, goals, tasks, and dedicated chat.

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

pub const PROJECT_PHASES: &[&str] = &[
    "observe", "think", "plan", "build", "execute", "verify", "learn",
];

#[derive(Clone, Default)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub goal: String,
    pub status: String,
    pub lifecycle_phase: String,
    pub goal_count: usize,
    pub active_goal_count: usize,
    pub source_type: String,
    pub workspace_path: String,
    pub repository: String,
    pub runtime_status: String,
    pub runtime_progress: u64,
    pub runtime_worker_count: usize,
}

#[derive(Clone, Default)]
pub struct ProjectTask {
    pub id: String,
    pub title: String,
    pub status: String,
    pub description: String,
}

#[derive(Clone, Default)]
pub struct ProjectGoal {
    pub id: String,
    pub name: String,
    pub status: String,
    pub check_command: String,
    pub description: String,
}

#[derive(Clone, Default)]
pub struct ProjectRuntimeWorker {
    pub role: String,
    pub phase: String,
    pub status: String,
    pub agent_id: String,
    pub summary: String,
}

#[derive(Clone, Default)]
pub struct ProjectRuntimeEvent {
    pub title: String,
    pub phase: String,
    pub status: String,
    pub detail: String,
    pub actor: String,
}

#[derive(Clone, Default)]
pub struct ProjectDetail {
    pub project: ProjectInfo,
    pub tasks: Vec<ProjectTask>,
    pub goals: Vec<ProjectGoal>,
    pub checkpoint: Option<String>,
    pub runtime_workers: Vec<ProjectRuntimeWorker>,
    pub runtime_events: Vec<ProjectRuntimeEvent>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectSubScreen {
    List,
    Detail,
    Create,
    GoalCreate,
    DeleteProjectConfirm,
    DeleteGoalConfirm,
}

pub struct ProjectState {
    pub sub: ProjectSubScreen,
    pub projects: Vec<ProjectInfo>,
    pub list_state: ListState,
    pub detail: Option<ProjectDetail>,
    pub goal_state: ListState,
    pub loading: bool,
    pub status_msg: String,
    pub tick: usize,
    pub create_step: usize,
    pub create_source_type: String,
    pub create_name: String,
    pub create_slug: String,
    pub create_goal: String,
    pub create_local_path: String,
    pub create_github_full_name: String,
    pub create_branch: String,
    pub goal_step: usize,
    pub goal_name: String,
    pub goal_check_command: String,
    pub goal_interval_secs: String,
}

pub enum ProjectAction {
    Continue,
    Refresh,
    Resume(String),
    OpenChat(String),
    CreateProject {
        name: String,
        slug: String,
        goal: String,
        source_type: String,
        local_path: String,
        github_full_name: String,
        branch: String,
    },
    DeleteProject(String),
    StartRuntime(String),
    PauseRuntime(String),
    ResumeRuntime(String),
    TakeoverRuntime(String),
    SetLifecycle {
        id_or_slug: String,
        phase: String,
    },
    CreateGoal {
        project_id: String,
        name: String,
        check_command: String,
        interval_secs: u64,
    },
    PauseGoal {
        project_id: String,
        goal_id: String,
    },
    ResumeGoal {
        project_id: String,
        goal_id: String,
    },
    DeleteGoal {
        project_id: String,
        goal_id: String,
    },
}

impl ProjectState {
    pub fn new() -> Self {
        Self {
            sub: ProjectSubScreen::List,
            projects: Vec::new(),
            list_state: ListState::default(),
            detail: None,
            goal_state: ListState::default(),
            loading: false,
            status_msg: String::new(),
            tick: 0,
            create_step: 0,
            create_source_type: "local".to_string(),
            create_name: String::new(),
            create_slug: String::new(),
            create_goal: String::new(),
            create_local_path: String::new(),
            create_github_full_name: String::new(),
            create_branch: String::new(),
            goal_step: 0,
            goal_name: String::new(),
            goal_check_command: String::new(),
            goal_interval_secs: "300".to_string(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ProjectAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return ProjectAction::Continue;
        }
        match self.sub {
            ProjectSubScreen::List => self.handle_list(key),
            ProjectSubScreen::Detail => self.handle_detail(key),
            ProjectSubScreen::Create => self.handle_create(key),
            ProjectSubScreen::GoalCreate => self.handle_goal_create(key),
            ProjectSubScreen::DeleteProjectConfirm => self.handle_delete_project_confirm(key),
            ProjectSubScreen::DeleteGoalConfirm => self.handle_delete_goal_confirm(key),
        }
    }

    fn selected_project(&self) -> Option<&ProjectInfo> {
        self.list_state
            .selected()
            .and_then(|idx| self.projects.get(idx))
    }

    fn detail_project_id(&self) -> Option<String> {
        self.detail
            .as_ref()
            .map(|detail| detail.project.slug.clone())
            .filter(|slug| !slug.is_empty())
            .or_else(|| self.detail.as_ref().map(|detail| detail.project.id.clone()))
    }

    pub fn active_project_id(&self) -> Option<String> {
        self.detail_project_id()
    }

    pub fn should_poll_runtime(&self) -> bool {
        matches!(self.sub, ProjectSubScreen::Detail)
            && !self.loading
            && self.tick.is_multiple_of(50)
            && self
                .detail
                .as_ref()
                .map(|detail| detail.project.runtime_status == "running")
                .unwrap_or(false)
    }

    fn selected_goal(&self) -> Option<&ProjectGoal> {
        self.detail.as_ref().and_then(|detail| {
            self.goal_state
                .selected()
                .and_then(|idx| detail.goals.get(idx))
        })
    }

    fn handle_list(&mut self, key: KeyEvent) -> ProjectAction {
        let total = self.projects.len().max(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state
                    .select(Some(if i == 0 { total - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1) % total));
            }
            KeyCode::Enter => {
                if let Some(project) = self.selected_project() {
                    let slug = project.slug.clone();
                    self.sub = ProjectSubScreen::Detail;
                    return ProjectAction::Resume(slug);
                }
            }
            KeyCode::Char('o') => {
                if let Some(project) = self.selected_project() {
                    return ProjectAction::OpenChat(project.slug.clone());
                }
            }
            KeyCode::Char('s') => {
                if let Some(project) = self.selected_project() {
                    return ProjectAction::StartRuntime(project.slug.clone());
                }
            }
            KeyCode::Char('n') => {
                self.create_step = 0;
                self.create_source_type = "local".to_string();
                self.create_name.clear();
                self.create_slug.clear();
                self.create_goal.clear();
                self.create_local_path.clear();
                self.create_github_full_name.clear();
                self.create_branch.clear();
                self.sub = ProjectSubScreen::Create;
            }
            KeyCode::Char('G') => {
                self.create_step = 0;
                self.create_source_type = "github".to_string();
                self.create_name.clear();
                self.create_slug.clear();
                self.create_goal.clear();
                self.create_local_path.clear();
                self.create_github_full_name.clear();
                self.create_branch.clear();
                self.sub = ProjectSubScreen::Create;
            }
            KeyCode::Char('d') => {
                if self.selected_project().is_some() {
                    self.sub = ProjectSubScreen::DeleteProjectConfirm;
                }
            }
            KeyCode::Char('r') => return ProjectAction::Refresh,
            _ => {}
        }
        ProjectAction::Continue
    }

    fn handle_detail(&mut self, key: KeyEvent) -> ProjectAction {
        match key.code {
            KeyCode::Esc => self.sub = ProjectSubScreen::List,
            KeyCode::Char('r') => {
                if let Some(id) = self.detail_project_id() {
                    return ProjectAction::Resume(id);
                }
            }
            KeyCode::Char('o') => {
                if let Some(id) = self.detail_project_id() {
                    return ProjectAction::OpenChat(id);
                }
            }
            KeyCode::Char('s') => {
                if let Some(id) = self.detail_project_id() {
                    return ProjectAction::StartRuntime(id);
                }
            }
            KeyCode::Char('x') => {
                if let Some(id) = self.detail_project_id() {
                    return ProjectAction::PauseRuntime(id);
                }
            }
            KeyCode::Char('S') => {
                if let Some(id) = self.detail_project_id() {
                    return ProjectAction::ResumeRuntime(id);
                }
            }
            KeyCode::Char('t') => {
                if let Some(id) = self.detail_project_id() {
                    return ProjectAction::TakeoverRuntime(id);
                }
            }
            KeyCode::Char('g') => {
                if self.detail_project_id().is_some() {
                    self.goal_step = 0;
                    self.goal_name.clear();
                    self.goal_check_command.clear();
                    self.goal_interval_secs = "300".to_string();
                    self.sub = ProjectSubScreen::GoalCreate;
                }
            }
            KeyCode::Char(']') => {
                if let Some(detail) = &self.detail {
                    if let Some(next) = next_phase(&detail.project.lifecycle_phase) {
                        return ProjectAction::SetLifecycle {
                            id_or_slug: detail.project.slug.clone(),
                            phase: next.to_string(),
                        };
                    }
                }
            }
            KeyCode::Char('[') => {
                if let Some(detail) = &self.detail {
                    if let Some(prev) = prev_phase(&detail.project.lifecycle_phase) {
                        return ProjectAction::SetLifecycle {
                            id_or_slug: detail.project.slug.clone(),
                            phase: prev.to_string(),
                        };
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let total = self
                    .detail
                    .as_ref()
                    .map(|detail| detail.goals.len().max(1))
                    .unwrap_or(1);
                let i = self.goal_state.selected().unwrap_or(0);
                self.goal_state
                    .select(Some(if i == 0 { total - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let total = self
                    .detail
                    .as_ref()
                    .map(|detail| detail.goals.len().max(1))
                    .unwrap_or(1);
                let i = self.goal_state.selected().unwrap_or(0);
                self.goal_state.select(Some((i + 1) % total));
            }
            KeyCode::Char('p') => {
                if let (Some(project_id), Some(goal)) =
                    (self.detail_project_id(), self.selected_goal())
                {
                    if goal.status == "active" {
                        return ProjectAction::PauseGoal {
                            project_id,
                            goal_id: goal.id.clone(),
                        };
                    }
                    return ProjectAction::ResumeGoal {
                        project_id,
                        goal_id: goal.id.clone(),
                    };
                }
            }
            KeyCode::Char('d') if self.selected_goal().is_some() => {
                self.sub = ProjectSubScreen::DeleteGoalConfirm;
            }
            _ => {}
        }
        ProjectAction::Continue
    }

    fn handle_create(&mut self, key: KeyEvent) -> ProjectAction {
        let last_step = if self.create_source_type == "github" {
            5
        } else {
            3
        };
        match key.code {
            KeyCode::Esc => {
                if self.create_step == 0 {
                    self.sub = ProjectSubScreen::List;
                } else {
                    self.create_step -= 1;
                }
            }
            KeyCode::Enter => {
                if self.create_step < last_step {
                    self.create_step += 1;
                } else if !self.create_name.trim().is_empty() && !self.create_goal.trim().is_empty()
                {
                    if self.create_source_type == "github"
                        && self.create_github_full_name.trim().is_empty()
                    {
                        return ProjectAction::Continue;
                    }
                    let slug = if self.create_slug.trim().is_empty() {
                        slugify(&self.create_name)
                    } else {
                        slugify(&self.create_slug)
                    };
                    let action = ProjectAction::CreateProject {
                        name: self.create_name.trim().to_string(),
                        slug,
                        goal: self.create_goal.trim().to_string(),
                        source_type: self.create_source_type.clone(),
                        local_path: self.create_local_path.trim().to_string(),
                        github_full_name: self.create_github_full_name.trim().to_string(),
                        branch: self.create_branch.trim().to_string(),
                    };
                    self.sub = ProjectSubScreen::List;
                    return action;
                }
            }
            KeyCode::Backspace => match self.create_step {
                0 => {
                    self.create_name.pop();
                }
                1 => {
                    self.create_slug.pop();
                }
                2 => {
                    self.create_goal.pop();
                }
                3 if self.create_source_type == "github" => {
                    self.create_github_full_name.pop();
                }
                4 if self.create_source_type == "github" => {
                    self.create_branch.pop();
                }
                _ => {
                    self.create_local_path.pop();
                }
            },
            KeyCode::Char(c) => match self.create_step {
                0 => self.create_name.push(c),
                1 => self.create_slug.push(c),
                2 => self.create_goal.push(c),
                3 if self.create_source_type == "github" => self.create_github_full_name.push(c),
                4 if self.create_source_type == "github" => self.create_branch.push(c),
                _ => self.create_local_path.push(c),
            },
            _ => {}
        }
        ProjectAction::Continue
    }

    fn handle_goal_create(&mut self, key: KeyEvent) -> ProjectAction {
        match key.code {
            KeyCode::Esc => {
                if self.goal_step == 0 {
                    self.sub = ProjectSubScreen::Detail;
                } else {
                    self.goal_step -= 1;
                }
            }
            KeyCode::Enter => {
                if self.goal_step < 2 {
                    self.goal_step += 1;
                } else if let Some(project_id) = self.detail_project_id() {
                    let interval = self
                        .goal_interval_secs
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(300)
                        .max(10);
                    if !self.goal_name.trim().is_empty()
                        && !self.goal_check_command.trim().is_empty()
                    {
                        let action = ProjectAction::CreateGoal {
                            project_id,
                            name: self.goal_name.trim().to_string(),
                            check_command: self.goal_check_command.trim().to_string(),
                            interval_secs: interval,
                        };
                        self.sub = ProjectSubScreen::Detail;
                        return action;
                    }
                }
            }
            KeyCode::Backspace => match self.goal_step {
                0 => {
                    self.goal_name.pop();
                }
                1 => {
                    self.goal_check_command.pop();
                }
                _ => {
                    self.goal_interval_secs.pop();
                }
            },
            KeyCode::Char(c) => match self.goal_step {
                0 => self.goal_name.push(c),
                1 => self.goal_check_command.push(c),
                _ if c.is_ascii_digit() => self.goal_interval_secs.push(c),
                _ => {}
            },
            _ => {}
        }
        ProjectAction::Continue
    }

    fn handle_delete_project_confirm(&mut self, key: KeyEvent) -> ProjectAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let id = self
                    .selected_project()
                    .map(|project| project.slug.clone())
                    .unwrap_or_default();
                self.sub = ProjectSubScreen::List;
                if !id.is_empty() {
                    return ProjectAction::DeleteProject(id);
                }
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.sub = ProjectSubScreen::List;
            }
            _ => {}
        }
        ProjectAction::Continue
    }

    fn handle_delete_goal_confirm(&mut self, key: KeyEvent) -> ProjectAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let project_id = self.detail_project_id().unwrap_or_default();
                let goal_id = self
                    .selected_goal()
                    .map(|goal| goal.id.clone())
                    .unwrap_or_default();
                self.sub = ProjectSubScreen::Detail;
                if !project_id.is_empty() && !goal_id.is_empty() {
                    return ProjectAction::DeleteGoal {
                        project_id,
                        goal_id,
                    };
                }
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.sub = ProjectSubScreen::Detail;
            }
            _ => {}
        }
        ProjectAction::Continue
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut ProjectState) {
    match state.sub {
        ProjectSubScreen::List | ProjectSubScreen::DeleteProjectConfirm => {
            draw_list(f, area, state)
        }
        ProjectSubScreen::Detail | ProjectSubScreen::DeleteGoalConfirm => {
            draw_detail(f, area, state)
        }
        ProjectSubScreen::Create => draw_project_form(f, area, state),
        ProjectSubScreen::GoalCreate => draw_goal_form(f, area, state),
    }
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut ProjectState) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).split(area);
    let items = if state.projects.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No projects. Press n for local or G for GitHub.",
            theme::dim_style(),
        )]))]
    } else {
        state
            .projects
            .iter()
            .map(|project| {
                let meta = format!(
                    "{} / {} / {} / run:{} {}% / {}/{} goals",
                    project.source_type,
                    project.status,
                    project.lifecycle_phase,
                    project.runtime_status,
                    project.runtime_progress,
                    project.active_goal_count,
                    project.goal_count
                );
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(project.name.clone(), theme::title_style()),
                        Span::raw("  "),
                        Span::styled(project.slug.clone(), theme::hint_style()),
                    ]),
                    Line::from(vec![Span::styled(meta, theme::dim_style())]),
                    Line::from(vec![Span::styled(
                        if project.repository.is_empty() {
                            project.workspace_path.clone()
                        } else {
                            format!("{}  {}", project.repository, project.workspace_path)
                        },
                        theme::hint_style(),
                    )]),
                    Line::from(project.goal.clone()),
                ])
            })
            .collect()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title("Projects")
                .borders(Borders::ALL)
                .border_style(theme::dim_style()),
        )
        .highlight_style(theme::selected_style())
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, chunks[0], &mut state.list_state);
    let mut help = "Enter resume  o chat  s start run  n local  G GitHub  d delete  r refresh";
    if state.sub == ProjectSubScreen::DeleteProjectConfirm {
        help = "Delete selected project and scoped goals? y confirm / n cancel";
    }
    f.render_widget(status_line(help, state), chunks[1]);
}

fn draw_detail(f: &mut Frame, area: Rect, state: &mut ProjectState) {
    let Some(detail) = state.detail.clone() else {
        f.render_widget(
            Paragraph::new("Select a project and press Enter.").block(
                Block::default()
                    .title("Project detail")
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    };
    let chunks = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(3),
        Constraint::Length(9),
        Constraint::Percentage(32),
        Constraint::Percentage(32),
        Constraint::Length(3),
    ])
    .split(area);
    draw_project_header(f, chunks[0], &detail.project);
    draw_lifecycle(f, chunks[1], &detail.project);
    draw_runtime(
        f,
        chunks[2],
        &detail.runtime_workers,
        &detail.runtime_events,
    );
    draw_goals(f, chunks[3], state, &detail.goals);
    draw_tasks(f, chunks[4], &detail.tasks, detail.checkpoint.as_deref());
    let mut help =
        "Esc list  o chat  s start  x pause  S resume  t takeover  [/] phase  g goal  r refresh";
    if state.sub == ProjectSubScreen::DeleteGoalConfirm {
        help = "Delete selected project goal? y confirm / n cancel";
    }
    f.render_widget(status_line(help, state), chunks[5]);
}

fn draw_project_header(f: &mut Frame, area: Rect, project: &ProjectInfo) {
    let lines = vec![
        Line::from(vec![
            Span::styled(project.name.clone(), theme::title_style()),
            Span::raw("  "),
            Span::styled(project.status.clone(), theme::dim_style()),
        ]),
        Line::from(vec![Span::styled(
            format!(
                "{} · {} · run:{} {}% · workers:{}",
                project.slug,
                project.id,
                project.runtime_status,
                project.runtime_progress,
                project.runtime_worker_count
            ),
            theme::hint_style(),
        )]),
        Line::from(vec![Span::styled(
            if project.repository.is_empty() {
                format!(
                    "{} workspace: {}",
                    project.source_type, project.workspace_path
                )
            } else {
                format!(
                    "{} repo: {} · {}",
                    project.source_type, project.repository, project.workspace_path
                )
            },
            theme::hint_style(),
        )]),
        Line::from(project.goal.clone()),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().title("Project").borders(Borders::ALL)),
        area,
    );
}

fn draw_lifecycle(f: &mut Frame, area: Rect, project: &ProjectInfo) {
    let current = project.lifecycle_phase.as_str();
    let lines = PROJECT_PHASES
        .iter()
        .map(|phase| {
            if *phase == current {
                Span::styled(
                    format!(" {} ", phase.to_ascii_uppercase()),
                    Style::default()
                        .fg(theme::BG_PRIMARY)
                        .bg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {} ", phase), theme::dim_style())
            }
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(Line::from(lines))
            .block(Block::default().title("Lifecycle").borders(Borders::ALL)),
        area,
    );
}

fn draw_goals(f: &mut Frame, area: Rect, state: &mut ProjectState, goals: &[ProjectGoal]) {
    let items = if goals.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No project goals. Press g to attach one.",
            theme::dim_style(),
        )]))]
    } else {
        goals
            .iter()
            .map(|goal| {
                let meta = format!("{} · {}", goal.status, goal.id);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(goal.name.clone(), theme::title_style()),
                        Span::raw("  "),
                        Span::styled(meta, theme::hint_style()),
                    ]),
                    Line::from(goal.check_command.clone()),
                    Line::from(vec![Span::styled(
                        goal.description.clone(),
                        theme::dim_style(),
                    )]),
                ])
            })
            .collect()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title("Project goals")
                .borders(Borders::ALL),
        )
        .highlight_style(theme::selected_style())
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, area, &mut state.goal_state);
}

fn draw_runtime(
    f: &mut Frame,
    area: Rect,
    workers: &[ProjectRuntimeWorker],
    events: &[ProjectRuntimeEvent],
) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)]).split(area);
    let worker_items = if workers.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No runtime workers yet.",
            theme::dim_style(),
        )]))]
    } else {
        workers
            .iter()
            .map(|worker| {
                let agent = if worker.agent_id.is_empty() {
                    String::new()
                } else {
                    format!(
                        " · agent {}",
                        worker.agent_id.chars().take(8).collect::<String>()
                    )
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(worker.role.clone(), theme::title_style()),
                        Span::raw("  "),
                        Span::styled(
                            format!("{} / {}{}", worker.status, worker.phase, agent),
                            theme::hint_style(),
                        ),
                    ]),
                    Line::from(vec![Span::styled(
                        worker.summary.clone(),
                        theme::dim_style(),
                    )]),
                ])
            })
            .collect()
    };
    f.render_widget(
        List::new(worker_items).block(
            Block::default()
                .title("Runtime workers")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let event_items = if events.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No runtime events yet.",
            theme::dim_style(),
        )]))]
    } else {
        events
            .iter()
            .rev()
            .take(6)
            .map(|event| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(event.title.clone(), theme::title_style()),
                        Span::raw("  "),
                        Span::styled(
                            format!("{} / {} / {}", event.actor, event.phase, event.status),
                            theme::hint_style(),
                        ),
                    ]),
                    Line::from(vec![Span::styled(event.detail.clone(), theme::dim_style())]),
                ])
            })
            .collect()
    };
    f.render_widget(
        List::new(event_items).block(Block::default().title("Live runtime").borders(Borders::ALL)),
        chunks[1],
    );
}

fn draw_tasks(f: &mut Frame, area: Rect, tasks: &[ProjectTask], checkpoint: Option<&str>) {
    let items = if tasks.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No tasks yet.",
            theme::dim_style(),
        )]))]
    } else {
        tasks
            .iter()
            .take(8)
            .map(|task| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(task.title.clone(), theme::title_style()),
                        Span::raw("  "),
                        Span::styled(task.status.clone(), theme::hint_style()),
                    ]),
                    Line::from(vec![Span::styled(
                        if task.description.is_empty() {
                            task.id.clone()
                        } else {
                            task.description.clone()
                        },
                        theme::dim_style(),
                    )]),
                ])
            })
            .collect()
    };
    let title = checkpoint
        .filter(|text| !text.is_empty())
        .map(|text| format!("Tasks · checkpoint: {}", text))
        .unwrap_or_else(|| "Tasks".to_string());
    f.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn draw_project_form(f: &mut Frame, area: Rect, state: &ProjectState) {
    let (labels, values): (Vec<&str>, Vec<&String>) = if state.create_source_type == "github" {
        (
            vec![
                "Name",
                "Slug",
                "Goal",
                "GitHub owner/repo",
                "Branch",
                "Local folder",
            ],
            vec![
                &state.create_name,
                &state.create_slug,
                &state.create_goal,
                &state.create_github_full_name,
                &state.create_branch,
                &state.create_local_path,
            ],
        )
    } else {
        (
            vec!["Name", "Slug", "Goal", "Local folder"],
            vec![
                &state.create_name,
                &state.create_slug,
                &state.create_goal,
                &state.create_local_path,
            ],
        )
    };
    draw_form(
        f,
        area,
        if state.create_source_type == "github" {
            "Connect GitHub project"
        } else {
            "Create local project"
        },
        labels.as_slice(),
        values.as_slice(),
        state.create_step,
        "Enter next/create  Esc back  empty folder = Captain default",
    );
}

fn draw_goal_form(f: &mut Frame, area: Rect, state: &ProjectState) {
    let labels = ["Goal name", "Check command", "Interval seconds"];
    let values = [
        &state.goal_name,
        &state.goal_check_command,
        &state.goal_interval_secs,
    ];
    draw_form(
        f,
        area,
        "Attach project goal",
        &labels,
        &values,
        state.goal_step,
        "Enter next/create  Esc back",
    );
}

fn draw_form(
    f: &mut Frame,
    area: Rect,
    title: &str,
    labels: &[&str],
    values: &[&String],
    active: usize,
    help: &str,
) {
    let mut lines = Vec::new();
    for (idx, label) in labels.iter().enumerate() {
        let style = if idx == active {
            theme::input_style()
        } else {
            theme::dim_style()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{label}: "), style),
            Span::raw(values.get(idx).map(|v| v.as_str()).unwrap_or("")),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(help, theme::hint_style())]));
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn status_line(help: &str, state: &ProjectState) -> Paragraph<'static> {
    let spinner = if state.loading {
        let frame = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        format!("{frame} loading")
    } else {
        String::new()
    };
    let text = if state.status_msg.is_empty() {
        format!("{help}  {spinner}")
    } else {
        format!("{help}  ·  {}  {spinner}", state.status_msg)
    };
    Paragraph::new(text)
        .style(theme::hint_style())
        .block(Block::default().borders(Borders::ALL))
}

fn next_phase(current: &str) -> Option<&'static str> {
    let idx = PROJECT_PHASES
        .iter()
        .position(|phase| *phase == current)
        .unwrap_or(0);
    PROJECT_PHASES
        .get((idx + 1).min(PROJECT_PHASES.len() - 1))
        .copied()
}

fn prev_phase(current: &str) -> Option<&'static str> {
    let idx = PROJECT_PHASES
        .iter()
        .position(|phase| *phase == current)
        .unwrap_or(0);
    PROJECT_PHASES.get(idx.saturating_sub(1)).copied()
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in input.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
        if out.len() >= 64 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}
