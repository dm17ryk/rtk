//! Interactive local dashboard for RTK savings and integration health.

use crate::core::tracking::{current_project_path_string, GainSummary, Tracker};
use crate::core::utils::format_tokens;
use crate::hooks::hook_check::{self, HookStatus};
use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

const TAB_NAMES: [&str; 5] = ["Overview", "Commands", "Activity", "Health", "Artifacts"];

pub fn run(project: bool) -> Result<()> {
    if !io::stdout().is_terminal() {
        render_once(project, 0)?;
        return Ok(());
    }

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)
        .context("Failed to enter dashboard screen")?;
    if let Err(error) = terminal::enable_raw_mode() {
        let _ = execute!(stdout, cursor::Show, LeaveAlternateScreen);
        return Err(error).context("Failed to enable dashboard input");
    }

    let result = dashboard_loop(&mut stdout, project);
    let restore = terminal::disable_raw_mode()
        .and_then(|_| execute!(stdout, cursor::Show, LeaveAlternateScreen));
    result.and(restore.context("Failed to restore terminal after dashboard"))
}

fn dashboard_loop(stdout: &mut impl Write, project: bool) -> Result<()> {
    let mut tab = 0usize;
    loop {
        render(stdout, project, tab)?;
        if event::poll(Duration::from_secs(2)).context("Failed to poll dashboard input")? {
            let event = event::read().context("Failed to read dashboard input")?;
            let Event::Key(key) = event else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('1'..='5') => {
                    if let KeyCode::Char(value) = key.code {
                        tab = (value as usize) - ('1' as usize);
                    }
                }
                KeyCode::Tab | KeyCode::Right | KeyCode::Char('n') => {
                    tab = (tab + 1) % TAB_NAMES.len();
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Char('p') => {
                    tab = (tab + TAB_NAMES.len() - 1) % TAB_NAMES.len();
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn render_once(project: bool, tab: usize) -> Result<()> {
    let mut stdout = io::stdout();
    render(&mut stdout, project, tab)
}

fn render(stdout: &mut impl Write, project: bool, tab: usize) -> Result<()> {
    let tracker = Tracker::new().context("Failed to initialize tracking database")?;
    let project_path = if project {
        Some(current_project_path_string())
    } else {
        None
    };
    let summary = tracker
        .get_summary_filtered(project_path.as_deref())
        .context("Failed to load dashboard statistics")?;
    let artifacts = list_artifacts()?;
    let width = terminal::size()
        .map(|(width, _)| width as usize)
        .unwrap_or(100);

    queue!(
        stdout,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print("RTK Dashboard"),
        ResetColor,
        SetAttribute(Attribute::Reset),
        Print(if project { "  [project]" } else { "  [global]" }),
        Print("\n\n"),
    )?;
    for (index, name) in TAB_NAMES.iter().enumerate() {
        if index == tab {
            queue!(
                stdout,
                SetForegroundColor(Color::Yellow),
                SetAttribute(Attribute::Bold),
                Print(format!(" {} {} ", index + 1, name)),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        } else {
            queue!(stdout, Print(format!(" {} {} ", index + 1, name)))?;
        }
        if index + 1 < TAB_NAMES.len() {
            queue!(stdout, Print("│"))?;
        }
    }
    queue!(
        stdout,
        Print("\n"),
        SetForegroundColor(Color::DarkGrey),
        Print("  Tab/←→ or n/p: switch   r: refresh   q/Esc: exit"),
        ResetColor,
        Print("\n\n")
    )?;

    match tab {
        0 => render_overview(stdout, &summary, width)?,
        1 => render_commands(stdout, &summary)?,
        2 => render_activity(stdout, &summary, width)?,
        3 => render_health(stdout, project_path.as_deref(), &artifacts)?,
        4 => render_artifacts(stdout, &artifacts)?,
        _ => {}
    }
    stdout.flush().context("Failed to refresh dashboard")
}

fn render_overview(stdout: &mut impl Write, summary: &GainSummary, width: usize) -> Result<()> {
    heading(stdout, "Savings overview")?;
    metric(stdout, "Commands", &summary.total_commands.to_string())?;
    metric(stdout, "Input tokens", &format_tokens(summary.total_input))?;
    metric(
        stdout,
        "Output tokens",
        &format_tokens(summary.total_output),
    )?;
    metric(
        stdout,
        "Tokens saved",
        &format!(
            "{} ({:.1}%)",
            format_tokens(summary.total_saved),
            summary.avg_savings_pct
        ),
    )?;
    metric(
        stdout,
        "Execution time",
        &format!(
            "{} ms total / {} ms avg",
            summary.total_time_ms, summary.avg_time_ms
        ),
    )?;
    queue!(
        stdout,
        Print("\n"),
        SetForegroundColor(Color::Green),
        Print(format!(
            "  [{}] {:.1}% savings\n",
            bar(summary.avg_savings_pct, width.saturating_sub(18).min(70)),
            summary.avg_savings_pct
        )),
        ResetColor,
        Print("\n")
    )?;
    render_top_commands(stdout, summary)
}

fn render_commands(stdout: &mut impl Write, summary: &GainSummary) -> Result<()> {
    heading(stdout, "Top commands by tokens saved")?;
    if summary.by_command.is_empty() {
        return writeln!(stdout, "  No tracking data yet.").context("Failed to render dashboard");
    }
    for (command, count, saved, savings, time) in &summary.by_command {
        writeln!(
            stdout,
            "  {:<28} {:>5} runs  {:>8} saved  {:>5.1}%  {:>5} ms",
            truncate(command, 28),
            count,
            format_tokens(*saved),
            savings,
            time
        )?;
    }
    Ok(())
}

fn render_activity(stdout: &mut impl Write, summary: &GainSummary, width: usize) -> Result<()> {
    heading(stdout, "Daily activity")?;
    if summary.by_day.is_empty() {
        return writeln!(stdout, "  No tracking data yet.").context("Failed to render dashboard");
    }
    let max_saved = summary
        .by_day
        .iter()
        .map(|(_, saved)| *saved)
        .max()
        .unwrap_or(1);
    for (date, saved) in summary.by_day.iter().rev().take(30) {
        let bar_width = width.saturating_sub(34).min(55);
        let count = ((*saved as f64 / max_saved as f64) * bar_width as f64) as usize;
        writeln!(
            stdout,
            "  {} {:>8} {}",
            date,
            format_tokens(*saved),
            "█".repeat(count.max(1))
        )?;
    }
    Ok(())
}

fn render_health(
    stdout: &mut impl Write,
    project_path: Option<&str>,
    artifacts: &[Artifact],
) -> Result<()> {
    heading(stdout, "Integration health")?;
    let hook = match hook_check::status() {
        HookStatus::Ok => "ok",
        HookStatus::Outdated => "outdated",
        HookStatus::Missing => "missing",
    };
    metric(stdout, "Hook/plugin status", hook)?;
    metric(
        stdout,
        "Integration detected",
        if hook_check::any_integration_installed() {
            "yes"
        } else {
            "no"
        },
    )?;
    metric(
        stdout,
        "Tracking scope",
        project_path.unwrap_or("all projects"),
    )?;
    metric(stdout, "Tee artifacts", &artifacts.len().to_string())?;
    metric(stdout, "MCP execution", "local stdio / bounded")?;
    Ok(())
}

fn render_artifacts(stdout: &mut impl Write, artifacts: &[Artifact]) -> Result<()> {
    heading(stdout, "Recent tee artifacts")?;
    if artifacts.is_empty() {
        return writeln!(stdout, "  No tee artifacts found.").context("Failed to render dashboard");
    }
    for artifact in artifacts {
        writeln!(
            stdout,
            "  {:>8}  {}",
            format_bytes(artifact.size),
            artifact.path.display()
        )?;
    }
    Ok(())
}

fn render_top_commands(stdout: &mut impl Write, summary: &GainSummary) -> Result<()> {
    heading(stdout, "Highest impact commands")?;
    for (command, count, saved, savings, _) in summary.by_command.iter().take(5) {
        writeln!(
            stdout,
            "  {:<28} {:>4} runs  {:>8} saved  {:>5.1}%",
            truncate(command, 28),
            count,
            format_tokens(*saved),
            savings
        )?;
    }
    Ok(())
}

fn heading(stdout: &mut impl Write, title: &str) -> Result<()> {
    writeln!(stdout, "\n  {}", title.to_uppercase()).context("Failed to render dashboard")?;
    writeln!(stdout, "  {}", "─".repeat(title.len() + 2)).context("Failed to render dashboard")
}

fn metric(stdout: &mut impl Write, label: &str, value: &str) -> Result<()> {
    writeln!(stdout, "  {:<22} {}", label, value).context("Failed to render dashboard")
}

fn bar(pct: f64, width: usize) -> String {
    let filled = ((pct.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut result: String = value.chars().take(max.saturating_sub(1)).collect();
    result.push('…');
    result
}

#[derive(Debug)]
struct Artifact {
    path: PathBuf,
    size: u64,
}

fn list_artifacts() -> Result<Vec<Artifact>> {
    let Some(data_dir) = dirs::data_local_dir() else {
        return Ok(Vec::new());
    };
    let directory = data_dir
        .join(crate::core::constants::RTK_DATA_DIR)
        .join("tee");
    let mut artifacts = std::fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "log") {
                return None;
            }
            let size = entry.metadata().ok()?.len();
            Some(Artifact { path, size })
        })
        .collect::<Vec<_>>();
    artifacts.sort_by(|left, right| right.path.cmp(&left.path));
    artifacts.truncate(20);
    Ok(artifacts)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}
