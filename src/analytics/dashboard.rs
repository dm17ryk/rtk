//! Interactive local dashboard for RTK savings and integration health.

use crate::core::config::Config;
use crate::core::display_helpers::format_duration;
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
    SynchronizedUpdate,
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
    let mut dirty = true;
    loop {
        if dirty {
            let frame = build_frame(project, tab)?;
            present_frame(stdout, &frame)?;
            dirty = false;
        }
        if event::poll(Duration::from_secs(2)).context("Failed to poll dashboard input")? {
            let event = event::read().context("Failed to read dashboard input")?;
            match event {
                Event::Resize(_, _) => dirty = true,
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('1'..='5') => {
                        if let KeyCode::Char(value) = key.code {
                            let next = (value as usize) - ('1' as usize);
                            dirty = next != tab;
                            tab = next;
                        }
                    }
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('n') => {
                        tab = (tab + 1) % TAB_NAMES.len();
                        dirty = true;
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Char('p') => {
                        tab = (tab + TAB_NAMES.len() - 1) % TAB_NAMES.len();
                        dirty = true;
                    }
                    KeyCode::Char('r') => dirty = true,
                    _ => {}
                },
                _ => {}
            }
        }
    }
    Ok(())
}

fn render_once(project: bool, tab: usize) -> Result<()> {
    let mut stdout = io::stdout();
    stdout
        .write_all(&build_frame(project, tab)?)
        .context("Failed to render dashboard")?;
    stdout.flush().context("Failed to flush dashboard")
}

fn build_frame(project: bool, tab: usize) -> Result<Vec<u8>> {
    let mut frame = Vec::new();
    render_content(&mut frame, project, tab)?;
    Ok(frame)
}

fn present_frame(stdout: &mut impl Write, frame: &[u8]) -> Result<()> {
    stdout
        .sync_update(|output| -> io::Result<()> {
            queue!(output, cursor::MoveTo(0, 0))?;
            output.write_all(frame)?;
            // Remove stale rows only after the complete replacement frame has
            // been queued. The old frame therefore remains visible until the
            // terminal atomically presents the new one.
            queue!(output, Clear(ClearType::FromCursorDown))?;
            Ok(())
        })
        .context("Failed to synchronize dashboard frame")?
        .context("Failed to write dashboard frame")
}

fn render_content(stdout: &mut impl Write, project: bool, tab: usize) -> Result<()> {
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
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print(format!("RTK Dashboard v{}", env!("CARGO_PKG_VERSION"))),
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
        1 => render_commands(stdout, &summary, width)?,
        2 => render_activity(stdout, &summary, width)?,
        3 => render_health(stdout, project_path.as_deref(), &artifacts)?,
        4 => render_artifacts(stdout, &artifacts)?,
        _ => {}
    }
    Ok(())
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
    render_command_table(stdout, summary, width, 10)
}

fn render_commands(stdout: &mut impl Write, summary: &GainSummary, width: usize) -> Result<()> {
    render_command_table(stdout, summary, width, summary.by_command.len())
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

fn render_command_table(
    stdout: &mut impl Write,
    summary: &GainSummary,
    width: usize,
    limit: usize,
) -> Result<()> {
    if summary.by_command.is_empty() {
        return writeln!(stdout, "  No tracking data yet.").context("Failed to render dashboard");
    }
    heading(stdout, "Highest impact commands")?;
    let command_width = width.saturating_sub(54).clamp(18, 42);
    let impact_width = width.saturating_sub(command_width + 48).clamp(8, 28);
    writeln!(
        stdout,
        "  {:>2}  {:<command_width$} {:>6} {:>9} {:>6} {:>8}  Impact",
        "#", "Command", "Count", "Saved", "Avg%", "Time"
    )?;
    writeln!(
        stdout,
        "  {}",
        "─".repeat((command_width + impact_width + 43).min(width.saturating_sub(2)))
    )?;
    let max_saved = summary
        .by_command
        .iter()
        .map(|(_, _, saved, _, _)| *saved)
        .max()
        .unwrap_or(1);
    for (index, (command, count, saved, savings, time)) in
        summary.by_command.iter().take(limit).enumerate()
    {
        queue!(
            stdout,
            Print(format!("  {:>2}. ", index + 1)),
            SetForegroundColor(Color::Cyan),
            Print(format!(
                "{:<command_width$}",
                truncate(command, command_width)
            )),
            ResetColor,
            Print(format!(" {:>6} {:>9} ", count, format_tokens(*saved))),
            SetForegroundColor(if *savings >= 50.0 {
                Color::Green
            } else {
                Color::Red
            }),
            Print(format!("{:>5.1}%", savings)),
            ResetColor,
            Print(format!(" {:>8}  ", format_duration(*time))),
            SetForegroundColor(Color::Blue),
            Print(
                "█".repeat(
                    ((*saved as f64 / max_saved as f64) * impact_width as f64)
                        .round()
                        .max(1.0) as usize
                )
            ),
            ResetColor,
            Print("\n")
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synchronized_frame_does_not_clear_before_drawing() {
        let mut output = Vec::new();
        present_frame(&mut output, b"frame").expect("present frame");
        let rendered = String::from_utf8(output).expect("ANSI output");
        assert!(rendered.contains("\u{1b}[?2026h"));
        assert!(rendered.contains("frame"));
        assert!(rendered.contains("\u{1b}[J"));
        assert!(!rendered.contains("\u{1b}[2J"));
        assert!(
            rendered.find("frame").unwrap() < rendered.find("\u{1b}[J").unwrap(),
            "stale rows must be cleared only after the replacement frame"
        );
    }

    #[test]
    fn command_table_contains_gain_columns() {
        let summary = GainSummary {
            total_commands: 1,
            total_input: 100,
            total_output: 20,
            total_saved: 80,
            avg_savings_pct: 80.0,
            total_time_ms: 50,
            avg_time_ms: 50,
            by_command: vec![("rtk git status".to_string(), 1, 80, 80.0, 50)],
            by_day: Vec::new(),
        };
        let mut output = Vec::new();
        render_command_table(&mut output, &summary, 100, 10).expect("command table");
        let rendered = String::from_utf8(output).expect("UTF-8 table");
        for heading in ["Command", "Count", "Saved", "Avg%", "Time", "Impact"] {
            assert!(rendered.contains(heading), "{heading}");
        }
        assert!(rendered.contains("rtk git status"));
    }
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
    let config = Config::load().context("Failed to load RTK configuration")?;
    let Some(directory) = crate::core::tee::get_tee_dir(&config) else {
        return Ok(Vec::new());
    };
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
