use std::{
    env, io,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde::Deserialize;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const HTTP_TIMEOUT: Duration = Duration::from_secs(4);

const TASK_LANES: [&str; 6] = [
    "Backlog",
    "Ready",
    "Doing",
    "Blocked",
    "Done",
    "Waiting Room",
];
const CRON_LANES: [&str; 2] = ["Scheduled", "Disabled"];

#[derive(Debug, Clone, Deserialize, Default)]
struct KanbanSnapshot {
    #[serde(default)]
    generated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    agents: Vec<Agent>,
    #[serde(default)]
    tasks: Vec<TaskCard>,
    #[serde(default)]
    cron: Vec<CronCard>,
    #[serde(default)]
    events: Vec<ProgressEvent>,
}

#[derive(Debug, Clone, Deserialize)]
struct Agent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    current_card_id: Option<String>,
    #[serde(default)]
    last_event_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskCard {
    id: String,
    title: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    lane: String,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
    #[serde(default, alias = "precondition", alias = "pre_conditions")]
    preconditions: Option<String>,
    #[serde(
        default,
        alias = "expectedBehavior",
        alias = "expected_behaviour",
        alias = "expected"
    )]
    expected_behavior: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CronCard {
    id: String,
    name: String,
    enabled: bool,
    schedule: String,
    #[serde(default)]
    lane: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProgressEvent {
    #[serde(default)]
    at: Option<DateTime<Utc>>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemKind {
    Task,
    Cron,
}

#[derive(Debug, Clone)]
struct BoardItem {
    key: String,
    kind: ItemKind,
    id: String,
    title: String,
    lane: String,
    status: String,
    assignee: Option<String>,
    preconditions: Option<String>,
    expected_behavior: Option<String>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Details,
    Items,
    Log,
}

struct App {
    focus: FocusPane,
    selected: usize,
    log_scroll: usize,
    snapshot: Option<KanbanSnapshot>,
    items: Vec<BoardItem>,
    events: Vec<String>,
    error: Option<String>,
    last_refresh: Instant,
}

impl App {
    fn new() -> Self {
        Self {
            focus: FocusPane::Items,
            selected: 0,
            log_scroll: 0,
            snapshot: None,
            items: Vec::new(),
            events: Vec::new(),
            error: None,
            last_refresh: Instant::now() - POLL_INTERVAL,
        }
    }

    fn focus_next(&mut self) {
        self.focus = match self.focus {
            FocusPane::Details => FocusPane::Items,
            FocusPane::Items => FocusPane::Log,
            FocusPane::Log => FocusPane::Details,
        };
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }

        let len = self.items.len() as isize;
        let mut next = self.selected as isize + delta;
        if next < 0 {
            next = 0;
        }
        if next >= len {
            next = len - 1;
        }
        self.selected = next as usize;
    }

    fn move_log(&mut self, delta: isize) {
        if delta < 0 {
            self.log_scroll = self.log_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.log_scroll = (self.log_scroll + delta as usize).min(self.events.len());
        }
    }

    fn selected_item(&self) -> Option<&BoardItem> {
        self.items.get(self.selected)
    }

    fn should_refresh(&self) -> bool {
        self.last_refresh.elapsed() >= POLL_INTERVAL
    }

    fn refresh(&mut self, api: &mut ApiClient) {
        let selected_key = self.selected_item().map(|i| i.key.clone());
        self.last_refresh = Instant::now();

        match api.fetch_snapshot() {
            Ok(snapshot) => {
                self.error = None;
                self.events = build_events(&snapshot);
                self.items = build_items(&snapshot);
                self.snapshot = Some(snapshot);
                self.log_scroll = self.log_scroll.min(self.events.len());

                if self.items.is_empty() {
                    self.selected = 0;
                } else if let Some(key) = selected_key {
                    if let Some(ix) = self.items.iter().position(|i| i.key == key) {
                        self.selected = ix;
                    } else {
                        self.selected = self.selected.min(self.items.len() - 1);
                    }
                } else {
                    self.selected = self.selected.min(self.items.len() - 1);
                }
            }
            Err(err) => {
                self.error = Some(err);
                if self.snapshot.is_none() {
                    self.items.clear();
                    self.events.clear();
                    self.selected = 0;
                    self.log_scroll = 0;
                }
            }
        }
    }
}

struct ApiClient {
    http: Client,
    base_url: String,
    user: String,
    pass: String,
}

impl ApiClient {
    fn from_env() -> Result<Self> {
        let base_url = env::var("OCMC_URL").unwrap_or_else(|_| "http://127.0.0.1:9091".to_string());
        let user = env::var("OCMC_USER").unwrap_or_else(|_| "admin".to_string());
        let pass = env::var("OCMC_PASS").unwrap_or_else(|_| "change-me".to_string());

        let http = Client::builder()
            .cookie_store(true)
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("build reqwest client")?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            user,
            pass,
        })
    }

    fn fetch_snapshot(&mut self) -> Result<KanbanSnapshot, String> {
        let url = format!("{}/api/kanban", self.base_url);
        let first = self
            .http
            .get(&url)
            .send()
            .map_err(|e| format!("Cannot reach {}: {e}", self.base_url))?;

        if first.status() == StatusCode::UNAUTHORIZED {
            self.login()?;
            let retry = self
                .http
                .get(&url)
                .send()
                .map_err(|e| format!("Cannot reach {}: {e}", self.base_url))?;
            return parse_snapshot_response(retry);
        }

        parse_snapshot_response(first)
    }

    fn login(&mut self) -> Result<(), String> {
        let url = format!("{}/login", self.base_url);
        let res = self
            .http
            .post(&url)
            .form(&[
                ("username", self.user.as_str()),
                ("password", self.pass.as_str()),
            ])
            .send()
            .map_err(|e| format!("Cannot login at {url}: {e}"))?;

        if res.status().is_success() || res.status().is_redirection() {
            Ok(())
        } else {
            let status = res.status();
            let body = res.text().unwrap_or_default();
            Err(format!(
                "Login failed ({status}). Check OCMC_USER/OCMC_PASS. {}",
                one_line(&body)
            ))
        }
    }
}

fn parse_snapshot_response(res: Response) -> Result<KanbanSnapshot, String> {
    let status = res.status();
    if !status.is_success() {
        let body = res.text().unwrap_or_default();
        return Err(format!(
            "GET /api/kanban failed ({status}): {}",
            one_line(&body)
        ));
    }

    res.json::<KanbanSnapshot>()
        .map_err(|e| format!("Invalid /api/kanban response: {e}"))
}

fn one_line(s: &str) -> String {
    s.replace('\n', " ").chars().take(180).collect::<String>()
}

fn build_items(snapshot: &KanbanSnapshot) -> Vec<BoardItem> {
    let mut out = Vec::new();

    for lane in TASK_LANES {
        for task in snapshot.tasks.iter().filter(|t| t.lane == lane) {
            out.push(BoardItem {
                key: format!("task:{}", task.id),
                kind: ItemKind::Task,
                id: task.id.clone(),
                title: task.title.clone(),
                lane: task.lane.clone(),
                status: if task.status.is_empty() {
                    "unknown".to_string()
                } else {
                    task.status.clone()
                },
                assignee: task.assignee.clone(),
                preconditions: task.preconditions.clone(),
                expected_behavior: task.expected_behavior.clone(),
                updated_at: task.updated_at,
            });
        }
    }

    for task in snapshot
        .tasks
        .iter()
        .filter(|t| !TASK_LANES.contains(&t.lane.as_str()))
    {
        out.push(BoardItem {
            key: format!("task:{}", task.id),
            kind: ItemKind::Task,
            id: task.id.clone(),
            title: task.title.clone(),
            lane: if task.lane.is_empty() {
                "Uncategorized".to_string()
            } else {
                task.lane.clone()
            },
            status: if task.status.is_empty() {
                "unknown".to_string()
            } else {
                task.status.clone()
            },
            assignee: task.assignee.clone(),
            preconditions: task.preconditions.clone(),
            expected_behavior: task.expected_behavior.clone(),
            updated_at: task.updated_at,
        });
    }

    for lane in CRON_LANES {
        for cron in snapshot.cron.iter().filter(|c| c.lane == lane) {
            out.push(BoardItem {
                key: format!("cron:{}", cron.id),
                kind: ItemKind::Cron,
                id: cron.id.clone(),
                title: cron.name.clone(),
                lane: cron.lane.clone(),
                status: if cron.enabled {
                    format!("enabled | {}", cron.schedule)
                } else {
                    format!("disabled | {}", cron.schedule)
                },
                assignee: None,
                preconditions: None,
                expected_behavior: None,
                updated_at: None,
            });
        }
    }

    for cron in snapshot
        .cron
        .iter()
        .filter(|c| !CRON_LANES.contains(&c.lane.as_str()))
    {
        out.push(BoardItem {
            key: format!("cron:{}", cron.id),
            kind: ItemKind::Cron,
            id: cron.id.clone(),
            title: cron.name.clone(),
            lane: if cron.lane.is_empty() {
                "Uncategorized".to_string()
            } else {
                cron.lane.clone()
            },
            status: if cron.enabled {
                format!("enabled | {}", cron.schedule)
            } else {
                format!("disabled | {}", cron.schedule)
            },
            assignee: None,
            preconditions: None,
            expected_behavior: None,
            updated_at: None,
        });
    }

    out
}

fn build_events(snapshot: &KanbanSnapshot) -> Vec<String> {
    let mut out = Vec::new();

    for ev in &snapshot.events {
        let ts = ev
            .at
            .map(|d| d.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "--:--:--".to_string());
        let text = ev
            .message
            .as_ref()
            .or(ev.reason.as_ref())
            .cloned()
            .unwrap_or_else(|| "event".to_string());
        out.push(format!("{ts}  {text}"));
    }

    if out.is_empty() {
        for agent in &snapshot.agents {
            if let Some(at) = agent.last_event_at {
                let who = if agent.display_name.is_empty() {
                    agent.id.clone()
                } else {
                    format!("{} ({})", agent.display_name, agent.id)
                };
                let state = if agent.state.is_empty() {
                    "state unknown".to_string()
                } else {
                    format!("state={}", agent.state)
                };
                out.push(format!("{}  {}  {}", at.format("%H:%M:%S"), who, state));
            }
        }
    }

    out
}

fn main() -> Result<()> {
    let mut terminal = init_terminal()?;
    let mut api = ApiClient::from_env()?;
    let mut app = App::new();

    let run_result = run_app(&mut terminal, &mut app, &mut api);

    restore_terminal(&mut terminal)?;
    run_result
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("create terminal backend")?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).context("leave alternate screen")?;
    terminal.show_cursor().context("show cursor")?;
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    api: &mut ApiClient,
) -> Result<()> {
    app.refresh(api);

    loop {
        terminal.draw(|frame| draw_ui(frame, app))?;

        let wait = POLL_INTERVAL.saturating_sub(app.last_refresh.elapsed());
        if event::poll(wait)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('r') => app.refresh(api),
                    KeyCode::Tab => app.focus_next(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        if app.focus == FocusPane::Log {
                            app.move_log(1);
                        } else {
                            app.move_selection(1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if app.focus == FocusPane::Log {
                            app.move_log(-1);
                        } else {
                            app.move_selection(-1);
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.should_refresh() {
            app.refresh(api);
        }
    }
}

fn draw_ui(frame: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(1)])
        .split(frame.area());

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(43), Constraint::Percentage(57)])
        .split(root[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(top[1]);

    let details = render_details(app);
    frame.render_widget(
        details.block(focus_block("Details", app.focus == FocusPane::Details)),
        top[0],
    );

    let mut list_state = ListState::default();
    if !app.items.is_empty() {
        list_state.select(Some(app.selected));
    }
    frame.render_stateful_widget(render_items_list(app), right[0], &mut list_state);

    frame.render_widget(render_log(app), right[1]);
    frame.render_widget(render_footer(app), root[1]);
}

fn focus_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Block::default()
        .title(title)
        .title_style(style)
        .borders(Borders::ALL)
}

fn render_details(app: &App) -> Paragraph<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(item) = app.selected_item() {
        let kind = match item.kind {
            ItemKind::Task => "Task",
            ItemKind::Cron => "Cron",
        };
        lines.push(Line::from(vec![
            Span::styled("Title: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(item.title.clone()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Type: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(kind),
            Span::raw("  "),
            Span::styled("ID: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(item.id.clone()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Lane: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(item.lane.clone()),
            Span::raw("  "),
            Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(item.status.clone()),
        ]));

        if let Some(assignee) = &item.assignee {
            lines.push(Line::from(vec![
                Span::styled("Assignee: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(assignee.clone()),
            ]));
        }

        if let Some(updated_at) = item.updated_at {
            lines.push(Line::from(vec![
                Span::styled("Updated: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
            ]));
        }

        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Preconditions",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(
            item.preconditions
                .clone()
                .unwrap_or_else(|| "Not available".to_string()),
        ));

        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Expected Behavior",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(
            item.expected_behavior
                .clone()
                .unwrap_or_else(|| "Not available".to_string()),
        ));
    } else if let Some(err) = &app.error {
        lines.push(Line::from(Span::styled(
            "Unable to load board data",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::default());
        lines.push(Line::from(err.clone()));
        lines.push(Line::default());
        lines.push(Line::from(
            "Check `OCMC_URL` and ensure mission_control is running.",
        ));
    } else {
        lines.push(Line::from("No items found."));
    }

    Paragraph::new(lines).wrap(Wrap { trim: false })
}

fn render_items_list(app: &App) -> List<'static> {
    let mut rows = Vec::new();
    for item in &app.items {
        let kind = match item.kind {
            ItemKind::Task => "task",
            ItemKind::Cron => "cron",
        };
        rows.push(ListItem::new(format!(
            "[{}] {}  ({})",
            item.lane, item.title, kind
        )));
    }

    if rows.is_empty() {
        rows.push(ListItem::new("No items"));
    }

    List::new(rows)
        .block(focus_block("Items", app.focus == FocusPane::Items))
        .highlight_symbol(">> ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
}

fn render_log(app: &App) -> Paragraph<'static> {
    let text = if app.events.is_empty() {
        "No events".to_string()
    } else {
        app.events.join("\n")
    };

    Paragraph::new(text)
        .block(focus_block("Progress Log", app.focus == FocusPane::Log))
        .scroll((app.log_scroll as u16, 0))
        .wrap(Wrap { trim: false })
}

fn render_footer(app: &App) -> Paragraph<'static> {
    let active_worker = app
        .snapshot
        .as_ref()
        .and_then(|s| {
            s.agents
                .iter()
                .find(|a| a.state.eq_ignore_ascii_case("doing"))
        })
        .map(|a| {
            let name = if a.display_name.is_empty() {
                a.id.clone()
            } else {
                format!("{} ({})", a.display_name, a.id)
            };
            if let Some(card) = &a.current_card_id {
                format!("Active worker: {name} on {card}")
            } else {
                format!("Active worker: {name}")
            }
        })
        .unwrap_or_else(|| "Active worker: none".to_string());

    let generated = app
        .snapshot
        .as_ref()
        .and_then(|s| s.generated_at)
        .map(|d| format!("snapshot {}", d.format("%H:%M:%S")))
        .unwrap_or_else(|| "snapshot --:--:--".to_string());

    let mut pieces = vec![
        active_worker,
        generated,
        "keys: j/k or Up/Down move  Tab focus  r refresh  q quit".to_string(),
    ];
    if let Some(err) = &app.error {
        pieces.push(format!("error: {}", one_line(err)));
    }

    Paragraph::new(pieces.join("  |  ")).style(Style::default().bg(Color::DarkGray))
}
