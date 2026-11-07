//! Interactive dashboard with complete diagnostic history, sorting and paging.

use crate::core::config::Config;
use crate::core::display_helpers::format_duration;
use crate::core::tracking::{
    DashboardGrouping, DashboardMetrics, GainSummary, Tracker, current_project_path_string,
};
use crate::core::utils::format_tokens;
use crate::hooks::hook_check::{self, HookStatus};
use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs},
};
use std::{
    cmp::Ordering,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

const TAB_NAMES: [&str; 6] = [
    "Overview",
    "Commands",
    "Error Commands",
    "Activity",
    "Health",
    "Artifacts",
];
const EFFICIENCY_LABEL_WIDTH: usize = 25;
const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortField {
    Command,
    Count,
    Saved,
    Avg,
    Time,
    Impact,
    Date,
    Output,
    Error,
    Name,
    Value,
    Size,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum DashboardTab {
    #[default]
    Overview,
    Commands,
    Errors,
    Activity,
    Health,
    Artifacts,
}

#[derive(Debug, Clone, Default, Args)]
pub struct Options {
    /// Filter the dashboard to the current project and its subdirectories
    #[arg(short, long)]
    pub project: bool,
    /// Initial tab (also selects the complete table for non-interactive output)
    #[arg(long, value_enum, default_value = "overview")]
    pub tab: DashboardTab,
    /// Initial sort column; tabs without this column retain their default
    #[arg(long, value_enum)]
    pub sort: Option<SortField>,
    /// Initial sort direction (default: descending)
    #[arg(long, value_enum)]
    pub order: Option<SortOrder>,
    /// Print every row of the selected tab without interactive terminal controls
    #[arg(long)]
    pub plain: bool,
}

pub fn run(options: &Options) -> Result<()> {
    if options.plain || !io::stdout().is_terminal() {
        let data = DashboardData::load(options.project)?;
        let state = DashboardState::new(options, &data);
        write!(io::stdout(), "{}", plain_dashboard_text(&data, &state))?;
        return Ok(());
    }

    // Declared before the terminal, so queued diagnostics flush only after
    // terminal cleanup (including early initialization errors).
    let _deferred_debug = io::stderr()
        .is_terminal()
        .then(crate::core::debug_log::defer);
    terminal::enable_raw_mode().context("Failed to enable dashboard input")?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, cursor::Hide) {
        let _ = terminal::disable_raw_mode();
        return Err(error).context("Failed to enter dashboard screen");
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
            return Err(error).context("Failed to initialize dashboard terminal");
        }
    };

    let result = dashboard_loop(&mut terminal, options);
    let disable_result =
        terminal::disable_raw_mode().context("Failed to disable dashboard raw mode");
    let leave_result = execute!(terminal.backend_mut(), cursor::Show, LeaveAlternateScreen)
        .context("Failed to leave dashboard screen");

    result.and(disable_result).and(leave_result)
}

fn dashboard_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    options: &Options,
) -> Result<()> {
    let mut data = DashboardData::load(options.project)?;
    let mut state = DashboardState::new(options, &data);
    let mut last_refresh = Instant::now();
    let mut status = None;
    let mut dirty = true;
    loop {
        if dirty {
            terminal
                .draw(|frame| draw(frame, &mut state, &data, status.as_deref()))
                .context("Failed to draw dashboard")?;
            dirty = false;
        }
        if event::poll(EVENT_POLL_INTERVAL).context("Failed to poll dashboard input")? {
            match event::read().context("Failed to read dashboard input")? {
                Event::Resize(_, _) => {
                    dirty = true;
                    debug_event("resize", &state);
                }
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('r') => {
                            refresh_data(&mut data, options.project, &mut status);
                            last_refresh = Instant::now();
                            dirty = true;
                        }
                        code => {
                            let height = terminal
                                .size()
                                .context("Failed to read terminal size")?
                                .height;
                            dirty |= state.handle_key(code, &data, height);
                        }
                    }
                }
                _ => {}
            }
        }
        if refresh_due(last_refresh.elapsed()) {
            refresh_data(&mut data, options.project, &mut status);
            last_refresh = Instant::now();
            dirty = true;
        }
    }
    Ok(())
}

fn refresh_due(elapsed: Duration) -> bool {
    elapsed >= AUTO_REFRESH_INTERVAL
}

fn refresh_data(data: &mut DashboardData, project: bool, status: &mut Option<String>) {
    match DashboardData::load(project) {
        Ok(refreshed) => {
            *data = refreshed;
            *status = None;
        }
        Err(error) => {
            *status = Some(format!("Refresh failed: {error:#}"));
        }
    }
    crate::core::debug_log::log(format_args!(
        "dashboard.refresh decision={} project={project}",
        if status.is_none() {
            "replaced"
        } else {
            "retained_previous"
        }
    ));
}

#[derive(Debug, Clone, Copy)]
struct TabState {
    column: usize,
    order: SortOrder,
    offset: usize,
}

struct DashboardState {
    tab: usize,
    tabs: [TabState; 6],
}

impl DashboardState {
    fn new(options: &Options, data: &DashboardData) -> Self {
        let tabs = std::array::from_fn(|index| {
            let table = data.table(index);
            let column = options
                .sort
                .and_then(|field| table.columns.iter().position(|c| c.field == field))
                .unwrap_or(table.default_column);
            TabState {
                column,
                order: options.order.unwrap_or(SortOrder::Desc),
                offset: 0,
            }
        });
        Self {
            tab: options.tab as usize,
            tabs,
        }
    }

    fn handle_key(&mut self, key: KeyCode, data: &DashboardData, height: u16) -> bool {
        let old_tab = self.tab;
        match key {
            KeyCode::Char(value @ '1'..='6') => self.tab = value as usize - '1' as usize,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('n') => self.tab = (self.tab + 1) % 6,
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('p') => self.tab = (self.tab + 5) % 6,
            _ => {}
        }
        if old_tab != self.tab {
            debug_event("tab", self);
            return true;
        }
        let capacity = viewport_capacity(self.tab, height);
        let table = data.table(self.tab);
        let tab = &mut self.tabs[self.tab];
        let max_offset = table.rows.len().saturating_sub(capacity.max(1));
        tab.offset = tab.offset.min(max_offset);
        match key {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let step = if key == KeyCode::Char('s') {
                    1
                } else {
                    table.columns.len() - 1
                };
                tab.column = (tab.column + step) % table.columns.len();
                tab.offset = 0;
            }
            KeyCode::Char('o') => {
                tab.order = if tab.order == SortOrder::Asc {
                    SortOrder::Desc
                } else {
                    SortOrder::Asc
                };
                tab.offset = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => tab.offset = (tab.offset + 1).min(max_offset),
            KeyCode::Up | KeyCode::Char('k') => tab.offset = tab.offset.saturating_sub(1),
            KeyCode::PageDown => {
                tab.offset = tab.offset.saturating_add(capacity.max(1)).min(max_offset)
            }
            KeyCode::PageUp => tab.offset = tab.offset.saturating_sub(capacity.max(1)),
            KeyCode::Home => tab.offset = 0,
            KeyCode::End => tab.offset = max_offset,
            _ => return false,
        }
        debug_event("sort_or_scroll", self);
        true
    }
}

fn debug_event(event: &str, state: &DashboardState) {
    let tab = state.tabs[state.tab];
    crate::core::debug_log::log(format_args!(
        "dashboard.input event={event} tab={} column={} order={:?} offset={}",
        state.tab, tab.column, tab.order, tab.offset
    ));
}

#[derive(Debug, Clone)]
enum SortValue {
    Text(String),
    Number(f64),
}
impl SortValue {
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(_), Self::Number(_)) => Ordering::Less,
            (Self::Number(_), Self::Text(_)) => Ordering::Greater,
        }
    }
}

struct ViewRow {
    cells: Vec<String>,
    values: Vec<SortValue>,
}
struct Column {
    field: SortField,
    label: &'static str,
    width: Constraint,
}
struct TableData {
    /// Optional discovery failures belong to this table, never the whole dashboard.
    warning: Option<String>,
    title: &'static str,
    columns: Vec<Column>,
    rows: Vec<ViewRow>,
    default_column: usize,
}
impl TableData {
    fn sorted(&self, state: TabState) -> Vec<&ViewRow> {
        let mut rows: Vec<_> = self.rows.iter().collect();
        rows.sort_by(|a, b| {
            let order = a.values[state.column].compare(&b.values[state.column]);
            let order = if state.order == SortOrder::Asc {
                order
            } else {
                order.reverse()
            };
            // Ascending identity tie-break keeps numeric ties stable in either direction.
            order.then_with(|| a.cells.cmp(&b.cells))
        });
        rows
    }
}

fn draw(frame: &mut Frame, state: &mut DashboardState, data: &DashboardData, status: Option<&str>) {
    frame.render_widget(Clear, frame.area());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let scope = if data.project_path.is_some() {
        "project"
    } else {
        "global"
    };
    let tabs = Tabs::new(TAB_NAMES)
        .block(panel(format!(
            "RTK Dashboard v{} [{scope}]",
            env!("CARGO_PKG_VERSION")
        )))
        .select(state.tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);
    let area = if state.tab == 0 {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(10), Constraint::Min(0)])
            .split(chunks[1]);
        draw_overview(frame, data, parts[0]);
        parts[1]
    } else {
        chunks[1]
    };
    draw_table(
        frame,
        data.table(state.tab),
        &mut state.tabs[state.tab],
        area,
    );
    draw_status(frame, status, chunks[2]);
}

fn draw_status(frame: &mut Frame, status: Option<&str>, area: Rect) {
    let text = vec![
        Line::from(" s/S: sort column | o: asc/desc | Up/Down/PgUp/PgDn/Home/End: scroll"),
        Line::from(status.map(str::to_owned).unwrap_or_else(|| {
            format!(
                " Tab/1-6: tabs | r: refresh | q/Esc: quit | auto-refresh: {}s",
                AUTO_REFRESH_INTERVAL.as_secs()
            )
        })),
    ];
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(if status.is_some() {
            Color::Red
        } else {
            Color::DarkGray
        })),
        area,
    );
}

fn draw_table(frame: &mut Frame, table: &TableData, state: &mut TabState, area: Rect) {
    let capacity = table_row_capacity(area);
    state.offset = state
        .offset
        .min(table.rows.len().saturating_sub(capacity.max(1)));
    let shown = capacity.min(table.rows.len().saturating_sub(state.offset));
    let start = if shown == 0 { 0 } else { state.offset + 1 };
    let end = if shown == 0 { 0 } else { state.offset + shown };
    let arrow = if state.order == SortOrder::Asc {
        "↑"
    } else {
        "↓"
    };
    let title = format!(
        "{} [{start}-{end}/{}] {} {arrow}",
        table.title,
        table.rows.len(),
        table.columns[state.column].label
    );
    let mut block = panel(title);
    if let Some(warning) = &table.warning {
        block = block.title_bottom(
            Line::from(format!(" Warning: {} ", display_cell(warning)))
                .style(Style::default().fg(Color::Yellow)),
        );
    }
    if table.rows.is_empty() {
        frame.render_widget(Paragraph::new("  No data available.").block(block), area);
        return;
    }
    let sorted = table.sorted(*state);
    let rows = sorted.into_iter().skip(state.offset).take(capacity).map(|row| {
        Row::new(row.cells.iter().enumerate().map(|(index, value)| {
            let color = match table.columns[index].field {
                SortField::Avg if matches!(row.values[index], SortValue::Number(n) if n < 50.0) => Color::Red,
                SortField::Error => Color::Red,
                SortField::Impact => Color::Blue,
                _ => Color::Cyan,
            };
            // Tables are one visual row; embedded newlines must not consume hidden rows.
            Cell::from(display_cell(value)).style(Style::default().fg(color))
        }))
    });
    let header = Row::new(table.columns.iter().enumerate().map(|(index, column)| {
        if index == state.column {
            format!("{}{arrow}", column.label)
        } else {
            column.label.to_string()
        }
    }))
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);
    frame.render_widget(
        Table::new(rows, table.columns.iter().map(|c| c.width))
            .header(header)
            .column_spacing(1)
            .block(block),
        area,
    );
}

fn table_row_capacity(area: Rect) -> usize {
    area.height.saturating_sub(4) as usize
}
fn viewport_capacity(tab: usize, height: u16) -> usize {
    table_row_capacity(Rect::new(
        0,
        0,
        0,
        height.saturating_sub(if tab == 0 { 15 } else { 5 }),
    ))
}

fn scope_lines(data: &DashboardData) -> [String; 2] {
    [
        format!(
            "All input sizes included; errors excluded: {}",
            data.summary.error_count
        ),
        "Metrics measure RTK output reduction before client processing.".to_string(),
    ]
}

fn draw_overview(frame: &mut Frame, data: &DashboardData, area: Rect) {
    let summary = &data.summary;
    let suffix_width = [summary.avg_savings_pct, summary.supported_avg_savings_pct]
        .iter()
        .map(|pct| format!(" {pct:.1}%").chars().count())
        .max()
        .unwrap_or(0);
    let bar_width = area
        .width
        .saturating_sub((2 + EFFICIENCY_LABEL_WIDTH + 1 + 2 + suffix_width) as u16)
        .min(60) as usize;
    let scope = scope_lines(data);
    let stats = vec![
        Line::from(format!(
            "  Commands: {}    Input: {}    Output: {}",
            summary.total_commands,
            format_tokens(summary.total_input),
            format_tokens(summary.total_output)
        )),
        Line::from(format!(
            "  Tokens saved: {} ({:.1}%)",
            format_tokens(summary.total_saved),
            summary.avg_savings_pct
        )),
        Line::from(format!(
            "  Execution time: {} total    Average: {}",
            format_duration(summary.total_time_ms),
            format_duration(summary.avg_time_ms)
        )),
        Line::from(""),
        efficiency_line("global", summary.avg_savings_pct, bar_width),
        efficiency_line("supported", summary.supported_avg_savings_pct, bar_width),
        Line::from(format!("  {}", scope[0])),
        Line::from(format!("  {}", scope[1])),
    ];
    frame.render_widget(Paragraph::new(stats).block(panel("Savings Overview")), area);
}

fn panel(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(format!(" {} ", title.into()))
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
}

fn efficiency_line(scope: &'static str, pct: f64, bar_width: usize) -> Line<'static> {
    let label = format!("  Efficiency ({scope}):");
    Line::from(vec![
        Span::styled(
            format!("{label:<width$} ", width = EFFICIENCY_LABEL_WIDTH),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("[{}]", bar(pct, bar_width)),
            Style::default().fg(Color::Green),
        ),
        Span::raw(format!(" {:.1}%", pct)),
    ])
}

// Stored command arguments may contain form feeds, ANSI escapes or Unicode
// line separators. Keep every displayed group on one inert visual/export row.
fn display_cell(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn plain_dashboard_text(data: &DashboardData, state: &DashboardState) -> String {
    let mut lines = vec![format!(
        "RTK Dashboard v{} [{}]",
        env!("CARGO_PKG_VERSION"),
        if data.project_path.is_some() {
            "project"
        } else {
            "global"
        }
    )];
    if state.tab == 0 {
        let summary = &data.summary;
        lines.extend([
            format!("Commands:      {}", summary.total_commands),
            format!("Input tokens:  {}", format_tokens(summary.total_input)),
            format!("Output tokens: {}", format_tokens(summary.total_output)),
            format!(
                "Tokens saved:  {} ({:.1}%)",
                format_tokens(summary.total_saved),
                summary.avg_savings_pct
            ),
            format!(
                "Execution:     {} total / {} average",
                format_duration(summary.total_time_ms),
                format_duration(summary.avg_time_ms)
            ),
            format!("Efficiency (global):    {:.1}%", summary.avg_savings_pct),
            format!(
                "Efficiency (supported): {:.1}%",
                summary.supported_avg_savings_pct
            ),
        ]);
        lines.extend(scope_lines(data));
    }
    let table = data.table(state.tab);
    let tab = state.tabs[state.tab];
    lines.push(format!(
        "{} (all {} rows; {} {:?})",
        table.title,
        table.rows.len(),
        table.columns[tab.column].label,
        tab.order
    ));
    lines.push(
        table
            .columns
            .iter()
            .map(|c| c.label)
            .collect::<Vec<_>>()
            .join("\t"),
    );
    for row in table.sorted(tab) {
        lines.push(
            row.cells
                .iter()
                .map(|s| display_cell(s))
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    if let Some(warning) = &table.warning {
        lines.push(format!("Warning: {}", display_cell(warning)));
    }
    format!("{}\n", lines.join("\n"))
}

struct DashboardData {
    summary: GainSummary,
    tables: Vec<TableData>,
    project_path: Option<String>,
}

impl DashboardData {
    fn table(&self, tab: usize) -> &TableData {
        &self.tables[tab.saturating_sub(1)]
    }
    fn load(project: bool) -> Result<Self> {
        let tracker = Tracker::new().context("Failed to initialize tracking database")?;
        let project_path = project.then(current_project_path_string);
        Self::from_tracker(&tracker, project_path, list_artifacts())
    }

    fn from_tracker(
        tracker: &Tracker,
        project_path: Option<String>,
        artifacts: Result<ArtifactDiscovery>,
    ) -> Result<Self> {
        let project = project_path.as_deref();
        let summary = tracker
            .get_dashboard_headline(project)
            .context("Failed to load dashboard headline")?;
        let commands = tracker.get_dashboard_metrics(project, DashboardGrouping::Command)?;
        let errors = tracker.get_dashboard_metrics(project, DashboardGrouping::Error)?;
        let activity = tracker.get_dashboard_metrics(project, DashboardGrouping::Day)?;
        let all_count = commands.iter().map(|row| row.count).sum::<usize>();
        let artifacts = artifacts.unwrap_or_else(|error| {
            crate::core::debug_log::log(format_args!(
                "dashboard.artifacts decision=discovery_failed"
            ));
            ArtifactDiscovery {
                warning: Some(format!("Artifact discovery unavailable: {error:#}")),
                ..Default::default()
            }
        });
        let tables = vec![
            metric_table("Commands (all recorded)", &commands, false, false),
            metric_table("Error Commands", &errors, true, false),
            metric_table("Daily Activity (all recorded)", &activity, false, true),
            health_table(&summary, project, artifacts.artifacts.len()),
            artifact_table(&artifacts.artifacts, artifacts.warning),
        ];
        crate::core::debug_log::log(format_args!(
            "dashboard.load project={} successful={all_count} errors={} tables={} limits=none",
            project.is_some(),
            summary.error_count,
            tables.len()
        ));
        Ok(Self {
            summary,
            tables,
            project_path,
        })
    }
}

fn column(field: SortField, label: &'static str, width: u16) -> Column {
    Column {
        field,
        label,
        width: if width == 0 {
            Constraint::Min(18)
        } else {
            Constraint::Length(width)
        },
    }
}
fn text(value: &str) -> SortValue {
    SortValue::Text(value.to_string())
}
fn number(value: impl Into<f64>) -> SortValue {
    SortValue::Number(value.into())
}
fn signed_tokens(value: i64) -> String {
    if value < 0 {
        format!("-{}", format_tokens(value.unsigned_abs() as usize))
    } else {
        format_tokens(value as usize)
    }
}

fn metric_table(
    title: &'static str,
    metrics: &[DashboardMetrics],
    errors: bool,
    daily: bool,
) -> TableData {
    let mut columns = vec![column(
        if daily {
            SortField::Date
        } else {
            SortField::Command
        },
        if daily { "Date" } else { "Command" },
        if daily { 10 } else { 0 },
    )];
    if errors {
        columns.push(column(SortField::Error, "Error", 18));
    }
    columns.extend([
        column(SortField::Count, "Count", 6),
        column(SortField::Saved, "Saved", 9),
        column(SortField::Avg, "Avg%", 7),
        column(SortField::Time, "Time", 8),
        column(SortField::Impact, "Impact", 8),
    ]);
    if !daily {
        columns.push(column(SortField::Date, "Date", 10));
    }
    columns.push(column(SortField::Output, "Output", 9));
    let max_saved = metrics.iter().map(|m| m.saved).max().unwrap_or(0).max(1) as f64;
    let rows = metrics
        .iter()
        .map(|m| {
            let avg = if m.input == 0 {
                0.0
            } else {
                m.saved as f64 * 100.0 / m.input as f64
            };
            let impact = ((m.saved.max(0) as f64 / max_saved) * 8.0).round() as usize;
            let mut cells = vec![m.command.clone()];
            let mut values = vec![text(&m.command)];
            if errors {
                cells.push(m.error.clone());
                values.push(text(&m.error));
            }
            cells.extend([
                m.count.to_string(),
                signed_tokens(m.saved),
                format!("{avg:.1}%"),
                format_duration(m.average_time_ms),
                "█".repeat(impact),
            ]);
            values.extend([
                number(m.count as f64),
                number(m.saved as f64),
                number(avg),
                number(m.average_time_ms as f64),
                number(m.saved as f64),
            ]);
            if !daily {
                cells.push(m.latest.clone());
                values.push(text(&m.latest));
            }
            cells.push(format_tokens(m.output));
            values.push(number(m.output as f64));
            ViewRow { cells, values }
        })
        .collect();
    let default_column = columns
        .iter()
        .position(|c| {
            c.field
                == if daily || errors {
                    SortField::Date
                } else {
                    SortField::Saved
                }
        })
        .unwrap_or(0);
    TableData {
        warning: None,
        title,
        columns,
        rows,
        default_column,
    }
}

fn health_table(summary: &GainSummary, project: Option<&str>, artifacts: usize) -> TableData {
    let hook = match hook_check::status() {
        HookStatus::Ok => "ok",
        HookStatus::Outdated => "outdated",
        HookStatus::Missing => "missing",
    };
    let entries = vec![
        ("Hook/plugin status", hook.to_string()),
        (
            "Integration detected",
            if hook_check::any_integration_installed() {
                "yes"
            } else {
                "no"
            }
            .to_string(),
        ),
        (
            "Tracking scope",
            project.unwrap_or("all projects").to_string(),
        ),
        ("Errored executions", summary.error_count.to_string()),
        ("Input size accounting", "all sizes included".to_string()),
        ("Tee artifacts", artifacts.to_string()),
        ("MCP execution", "local stdio / bounded".to_string()),
    ];
    TableData {
        warning: None,
        title: "Integration Health",
        columns: vec![
            column(SortField::Name, "Name", 30),
            column(SortField::Value, "Value", 0),
        ],
        rows: entries
            .into_iter()
            .map(|(name, value)| ViewRow {
                values: vec![text(name), text(&value)],
                cells: vec![name.to_string(), value],
            })
            .collect(),
        default_column: 0,
    }
}

struct Artifact {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}
#[derive(Default)]
struct ArtifactDiscovery {
    artifacts: Vec<Artifact>,
    warning: Option<String>,
}

fn list_artifacts() -> Result<ArtifactDiscovery> {
    let config = Config::load().context("Failed to load RTK configuration")?;
    let Some(directory) = crate::core::tee::get_tee_dir(&config) else {
        return Ok(ArtifactDiscovery::default());
    };
    Ok(discover_artifacts(&directory))
}

fn discover_artifacts(directory: &std::path::Path) -> ArtifactDiscovery {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            crate::core::debug_log::log(format_args!("dashboard.artifacts decision=missing"));
            return ArtifactDiscovery::default();
        }
        Err(error) => {
            crate::core::debug_log::log(format_args!(
                "dashboard.artifacts decision=unavailable kind={:?}",
                error.kind()
            ));
            return ArtifactDiscovery {
                warning: Some(format!("Cannot list tee artifacts: {error}")),
                ..Default::default()
            };
        }
    };
    collect_artifacts(entries.map(|entry| {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "log") {
            return Ok(None);
        }
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            return Ok(None);
        }
        Ok(Some(Artifact {
            path,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }))
    }))
}

fn collect_artifacts(
    entries: impl IntoIterator<Item = io::Result<Option<Artifact>>>,
) -> ArtifactDiscovery {
    let mut discovery = ArtifactDiscovery::default();
    let mut skipped = 0usize;
    for entry in entries {
        match entry {
            Ok(Some(artifact)) => discovery.artifacts.push(artifact),
            Ok(None) => {}
            // RTK can rotate artifacts while this directory is being enumerated.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                skipped += 1;
                discovery
                    .warning
                    .get_or_insert_with(|| format!("Cannot inspect a tee artifact: {error}"));
            }
        }
    }
    if let Some(warning) = &mut discovery.warning {
        *warning = format!("Skipped {skipped} artifact entries. {warning}");
    }
    crate::core::debug_log::log(format_args!(
        "dashboard.artifacts decision=enumerated found={} skipped={skipped}",
        discovery.artifacts.len()
    ));
    discovery
}

fn artifact_table(artifacts: &[Artifact], warning: Option<String>) -> TableData {
    let rows = artifacts
        .iter()
        .map(|artifact| {
            let date = artifact
                .modified
                .map(|time| {
                    chrono::DateTime::<chrono::Utc>::from(time)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default();
            let path = artifact.path.to_string_lossy().into_owned();
            ViewRow {
                cells: vec![format_bytes(artifact.size), date.clone(), path.clone()],
                values: vec![number(artifact.size as f64), text(&date), text(&path)],
            }
        })
        .collect();
    TableData {
        warning,
        title: "Tee Artifacts",
        columns: vec![
            column(SortField::Size, "Size", 10),
            column(SortField::Date, "Date (UTC)", 19),
            column(SortField::Path, "Path", 0),
        ],
        rows,
        default_column: 1,
    }
}

fn bar(pct: f64, width: usize) -> String {
    let filled = ((pct.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(width.saturating_sub(filled))
    )
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn metric(command: &str, count: usize, saved: i64, time: u64, date: &str) -> DashboardMetrics {
        DashboardMetrics {
            command: command.into(),
            count,
            input: 1000,
            output: (1000 - saved) as usize,
            saved,
            average_time_ms: time,
            latest: date.into(),
            ..Default::default()
        }
    }

    fn fixture(count: usize) -> DashboardData {
        let commands = (0..count)
            .map(|i| {
                metric(
                    &format!("rtk command-{i:03}"),
                    i + 1,
                    i as i64 * 10,
                    i as u64 * 2,
                    &format!("2026-07-{:02} 12:00:00", i % 28 + 1),
                )
            })
            .collect::<Vec<_>>();
        let summary = GainSummary {
            total_commands: count,
            total_input: 100,
            total_output: 20,
            total_saved: 80,
            avg_savings_pct: 80.0,
            supported_avg_savings_pct: 80.0,
            error_count: 0,
            oversized_excluded: 0,
            total_time_ms: 50,
            avg_time_ms: 25,
            by_command: Vec::new(),
            by_day: Vec::new(),
            activity_total_days: count,
            error_commands: Vec::new(),
            error_command_groups: 0,
        };
        let tables = vec![
            metric_table("Commands (all recorded)", &commands, false, false),
            metric_table("Error Commands", &commands, true, false),
            metric_table("Daily Activity (all recorded)", &commands, false, true),
            TableData {
                warning: None,
                title: "Health",
                columns: vec![column(SortField::Name, "Name", 0)],
                rows: commands
                    .iter()
                    .map(|m| ViewRow {
                        cells: vec![m.command.clone()],
                        values: vec![text(&m.command)],
                    })
                    .collect(),
                default_column: 0,
            },
            artifact_table(
                &(0..count)
                    .map(|i| Artifact {
                        path: format!("artifact-{i:03}.log").into(),
                        size: i as u64,
                        modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64)),
                    })
                    .collect::<Vec<_>>(),
                None,
            ),
        ];
        DashboardData {
            summary,
            tables,
            project_path: None,
        }
    }

    fn screen(data: &DashboardData, state: &mut DashboardState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, state, data, None))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn unavailable_artifacts_do_not_block_any_other_tab_or_plain_export() {
        let tracker = Tracker::new_in_memory().unwrap();
        tracker.record("test", "rtk test", 100, 20, 3).unwrap();
        let data = DashboardData::from_tracker(
            &tracker,
            None,
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "inaccessible tee directory",
            )
            .into()),
        )
        .unwrap();
        assert_eq!(data.summary.total_commands, 1);
        assert_eq!(data.table(1).rows.len(), 1);
        assert!(data.summary.by_command.is_empty());
        assert!(data.summary.by_day.is_empty());
        for tab in 0..6 {
            let mut state = DashboardState::new(&Options::default(), &data);
            state.tab = tab;
            let plain = plain_dashboard_text(&data, &state);
            let rendered = screen(&data, &mut state, 130, 24);
            if tab == 5 {
                assert!(plain.contains("Warning: Artifact discovery unavailable:"));
                assert!(rendered.contains("Warning:"));
            } else {
                assert!(!plain.contains("Artifact discovery unavailable"));
                assert!(!rendered.contains("Warning:"));
            }
        }
        // A subsequent refresh with a recovered directory clears only its warning.
        let recovered =
            DashboardData::from_tracker(&tracker, None, Ok(ArtifactDiscovery::default())).unwrap();
        assert!(recovered.table(5).warning.is_none());
        assert_eq!(recovered.table(1).rows.len(), data.table(1).rows.len());
    }

    #[test]
    fn artifact_scan_continues_after_an_unreadable_entry() {
        let artifact = |name: &str| {
            Some(Artifact {
                path: name.into(),
                size: 1,
                modified: None,
            })
        };
        let scan = collect_artifacts([
            Ok(artifact("before.log")),
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            Err(io::Error::new(io::ErrorKind::NotFound, "rotated")),
            Ok(artifact("after.log")),
        ]);
        assert_eq!(scan.artifacts.len(), 2);
        assert!(
            scan.warning
                .as_ref()
                .unwrap()
                .contains("Skipped 1 artifact entries")
        );
        let table = artifact_table(&scan.artifacts, scan.warning);
        assert_eq!(table.rows.len(), 2);
        assert!(table.warning.is_some());
    }

    #[test]
    fn artifact_directory_io_failure_becomes_a_table_warning() {
        let temporary = tempfile::tempdir().unwrap();
        let not_a_directory = temporary.path().join("file");
        std::fs::write(&not_a_directory, b"test").unwrap();
        let scan = discover_artifacts(&not_a_directory);
        assert!(scan.artifacts.is_empty());
        assert!(scan.warning.unwrap().contains("Cannot list tee artifacts:"));
        assert!(
            discover_artifacts(&temporary.path().join("missing"))
                .warning
                .is_none()
        );
    }

    #[test]
    fn numeric_sorts_include_the_worst_group_before_paging() {
        let data = fixture(50);
        for field in [
            SortField::Count,
            SortField::Saved,
            SortField::Avg,
            SortField::Time,
            SortField::Impact,
            SortField::Output,
        ] {
            let options = Options {
                tab: DashboardTab::Commands,
                sort: Some(field),
                order: Some(SortOrder::Asc),
                ..Default::default()
            };
            let mut state = DashboardState::new(&options, &data);
            let sorted = data.table(1).sorted(state.tabs[1]);
            let expected = if field == SortField::Output {
                "rtk command-049"
            } else {
                "rtk command-000"
            };
            assert_eq!(sorted[0].cells[0], expected, "{field:?}");
            assert!(screen(&data, &mut state, 130, 24).contains(expected));
            state.handle_key(KeyCode::Char('o'), &data, 24);
            assert_ne!(
                data.table(1).sorted(state.tabs[1])[0].cells[0],
                expected,
                "{field:?}"
            );
        }
    }

    #[test]
    fn command_and_full_timestamp_sorts_are_deterministic() {
        let rows = vec![
            metric("z", 1, 10, 1, "2026-01-01 23:59:59"),
            metric("b", 1, 10, 1, "2026-01-02 00:00:01"),
            metric("a", 1, 10, 1, "2026-01-02 00:00:00"),
        ];
        let table = metric_table("Commands", &rows, false, false);
        let date = table
            .columns
            .iter()
            .position(|c| c.field == SortField::Date)
            .unwrap();
        for (column, expected) in [
            (0, vec!["a", "b", "z"]),
            (date, vec!["z", "a", "b"]),
            (table.default_column, vec!["a", "b", "z"]),
        ] {
            let sorted = table.sorted(TabState {
                column,
                order: SortOrder::Asc,
                offset: 0,
            });
            assert_eq!(
                sorted
                    .iter()
                    .map(|r| r.cells[0].as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn every_tab_can_sort_scroll_to_end_and_preserve_its_state() {
        let data = fixture(50);
        let mut state = DashboardState::new(&Options::default(), &data);
        for tab in 0..6 {
            state.tab = tab;
            state.handle_key(KeyCode::Char('s'), &data, 24);
            state.handle_key(KeyCode::Char('o'), &data, 24);
            state.handle_key(KeyCode::End, &data, 24);
            let end = state.tabs[tab].offset;
            let rendered = screen(&data, &mut state, 130, 24);
            assert!(end > 0, "tab {tab}");
            assert!(rendered.contains("/50]"), "{rendered}");
            assert_eq!(state.tabs[tab].offset, end);
            state.handle_key(KeyCode::PageUp, &data, 24);
            assert!(state.tabs[tab].offset < end);
            state.handle_key(KeyCode::Home, &data, 24);
            assert_eq!(state.tabs[tab].offset, 0);
        }
        let remembered = state.tabs[1];
        state.tab = 1;
        state.handle_key(KeyCode::Tab, &data, 24);
        state.handle_key(KeyCode::BackTab, &data, 24);
        assert_eq!(state.tab, 1);
        assert_eq!(state.tabs[1].column, remembered.column);
        assert_eq!(state.tabs[1].order, remembered.order);
    }

    #[test]
    fn resize_and_refresh_clamp_offsets_without_resetting_sort() {
        let data = fixture(50);
        let mut state = DashboardState::new(
            &Options {
                tab: DashboardTab::Commands,
                ..Default::default()
            },
            &data,
        );
        state.handle_key(KeyCode::End, &data, 24);
        screen(&data, &mut state, 130, 40);
        assert_eq!(state.tabs[1].offset, 50 - viewport_capacity(1, 40));
        let small = fixture(2);
        let column = state.tabs[1].column;
        screen(&small, &mut state, 130, 24);
        assert_eq!(state.tabs[1].offset, 0);
        assert_eq!(state.tabs[1].column, column);
    }

    #[test]
    fn plain_export_has_every_row_for_every_tab_and_honors_sort() {
        let data = fixture(50);
        for tab in 0..6 {
            let mut state = DashboardState::new(&Options::default(), &data);
            state.tab = tab;
            state.tabs[tab].order = SortOrder::Asc;
            state.tabs[tab].offset = 30;
            let output = plain_dashboard_text(&data, &state);
            assert!(output.contains("all 50 rows"));
            assert!(output.contains("000"));
            assert!(output.contains("049"));
            assert!(!output.contains('\u{1b}'));
        }
    }

    #[test]
    fn recorded_control_characters_cannot_add_export_rows_or_terminal_escapes() {
        let mut data = fixture(1);
        data.tables[0].rows[0].cells[0] =
            "rtk cmd\u{c}one\u{1b}[31m\u{2028}two\u{2029}three".into();
        let state = DashboardState::new(
            &Options {
                tab: DashboardTab::Commands,
                ..Default::default()
            },
            &data,
        );
        let output = plain_dashboard_text(&data, &state);
        assert_eq!(output.lines().count(), 4);
        assert!(
            !output
                .chars()
                .any(|c| matches!(c, '\u{c}' | '\u{1b}' | '\u{2028}' | '\u{2029}'))
        );
        assert!(output.contains("rtk cmd one [31m two three"));
    }

    #[test]
    fn output_expansion_and_zero_impact_are_visible() {
        let table = metric_table(
            "Commands",
            &[
                metric("expanded", 1, -10, 1, ""),
                metric("zero", 1, 0, 1, ""),
            ],
            false,
            false,
        );
        assert_eq!(table.rows[0].cells[2], "-10");
        assert_eq!(table.rows[0].cells[3], "-1.0%");
        assert!(table.rows.iter().all(|row| row.cells[5].is_empty()));
    }

    #[test]
    fn overview_explains_all_sizes_are_included_and_errors_are_separate() {
        let mut data = fixture(2);
        data.summary.error_count = 4;
        let mut state = DashboardState::new(&Options::default(), &data);
        let rendered = screen(&data, &mut state, 120, 30);
        assert!(rendered.contains("All input sizes included; errors excluded: 4"));
        let plain = plain_dashboard_text(&data, &state);
        assert!(plain.contains("All input sizes included"));
        assert!(plain.contains("Efficiency (global):    80.0%"));
    }

    #[test]
    fn efficiency_bars_align_and_narrow_screen_keeps_percentages() {
        let data = fixture(2);
        let mut state = DashboardState::new(&Options::default(), &data);
        for width in [80, 120] {
            let rendered = screen(&data, &mut state, width, 30);
            let lines = rendered
                .lines()
                .filter(|line| line.contains("Efficiency ("))
                .collect::<Vec<_>>();
            assert_eq!(lines.len(), 2);
            assert!(lines.iter().all(|line| line.contains("80.0%")));
            assert_eq!(lines[0].find('['), lines[1].find('['));
            assert_eq!(lines[0].find('%'), lines[1].find('%'));
        }
    }

    #[test]
    fn tiny_empty_and_tab_switch_screens_render_cleanly() {
        let data = fixture(0);
        let mut state = DashboardState::new(&Options::default(), &data);
        for height in [0, 1, 3, 8, 24] {
            for tab in 0..6 {
                state.tab = tab;
                screen(&data, &mut state, 80, height);
            }
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        state.tab = 0;
        terminal
            .draw(|frame| draw(frame, &mut state, &data, None))
            .unwrap();
        state.tab = 3;
        terminal
            .draw(|frame| draw(frame, &mut state, &data, None))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..30)
            .flat_map(|y| (0..120).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();
        assert!(text.contains("Daily Activity"));
        assert!(!text.contains("Savings Overview"));
    }

    #[test]
    fn cli_options_parse_and_reject_invalid_sort() {
        use clap::Parser;
        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            options: Options,
        }
        let cli = TestCli::try_parse_from([
            "dashboard",
            "--tab",
            "commands",
            "--sort",
            "avg",
            "--order",
            "asc",
            "--plain",
            "--project",
        ])
        .unwrap();
        assert_eq!(cli.options.sort, Some(SortField::Avg));
        assert_eq!(cli.options.order, Some(SortOrder::Asc));
        assert!(cli.options.plain && cli.options.project);
        assert!(TestCli::try_parse_from(["dashboard", "--sort", "invalid"]).is_err());
    }

    #[test]
    fn refresh_cadence_and_footer_remain_discoverable() {
        assert!(!refresh_due(Duration::from_secs(29)));
        assert!(refresh_due(Duration::from_secs(30)));
        let data = fixture(2);
        let mut state = DashboardState::new(&Options::default(), &data);
        let rendered = screen(&data, &mut state, 120, 24);
        for label in [
            "s/S: sort column",
            "o: asc/desc",
            "scroll",
            "auto-refresh: 30s",
        ] {
            assert!(rendered.contains(label));
        }
    }
}
