use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::output;
use crate::tui::deletion::{DeletionJob, Outcome};
use crate::tui::theme;
use crate::tui::views::confirm::centered_rect;
use crate::util;

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Lines the dialog spends on everything but the list of finished items.
const CHROME_LINES: usize = 10;

fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `estimate` marks a figure that includes the in-progress item's share,
/// which is judged by parts removed rather than bytes.
fn gauge(done: u64, total: u64, width: usize, estimate: bool) -> Line<'static> {
    let ratio = if total == 0 {
        1.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0)
    };
    let filled = (ratio * width as f64).round() as usize;
    Line::from(vec![
        Span::raw("  "),
        Span::styled("━".repeat(filled), Style::default().fg(theme::ACCENT)),
        Span::styled(
            "─".repeat(width.saturating_sub(filled)),
            Style::default().fg(theme::DIM),
        ),
        Span::styled(
            format!(
                "  {:>5}",
                format!("{}{:.0}%", if estimate { "~" } else { "" }, ratio * 100.0)
            ),
            Style::default().fg(theme::FG),
        ),
    ])
}

fn outcome_line(outcome: &Outcome, width: usize) -> Line<'static> {
    let (mark, color, size) = match &outcome.error {
        None => ("✓", theme::GREEN, util::human_size(outcome.freed)),
        Some(_) => ("✗", theme::RED, String::new()),
    };
    let reason = outcome
        .error
        .as_deref()
        .map_or_else(String::new, |e| format!("  {e}"));
    let label_room = width.saturating_sub(2 + 2 + 10 + 2 + reason.len().min(width / 2));
    Line::from(vec![
        Span::styled(format!("  {mark} "), Style::default().fg(color)),
        Span::styled(format!("{size:>10}  "), Style::default().fg(theme::FG)),
        Span::styled(
            output::truncate_start(&outcome.label, label_room),
            Style::default().fg(theme::FG),
        ),
        Span::styled(
            output::truncate_end(&reason, width / 2),
            Style::default().fg(theme::DIM),
        ),
    ])
}

fn headline(job: &DeletionJob) -> Line<'static> {
    let total = job.entries.len();
    let done = job.outcomes.len();
    let failed = job.outcomes.iter().filter(|o| o.error.is_some()).count();
    let elapsed = job.finished.unwrap_or_else(|| job.started.elapsed());

    if job.is_finished() {
        let verb = if done < total { "Stopped." } else { "Done." };
        return Line::from(vec![
            Span::styled(
                format!(" {verb} Freed "),
                Style::default().fg(theme::FG).bold(),
            ),
            Span::styled(
                util::human_size(job.freed()),
                Style::default().fg(theme::GREEN).bold(),
            ),
            Span::styled(
                format!(
                    " from {} in {}",
                    output::plural(done - failed, "item", "items"),
                    clock(elapsed.as_secs())
                ),
                Style::default().fg(theme::FG).bold(),
            ),
        ]);
    }
    Line::from(vec![
        Span::styled(
            format!(" Deleting {} of {total}", (done + 1).min(total)),
            Style::default().fg(theme::FG).bold(),
        ),
        Span::styled(
            format!(
                " · {} freed · {}",
                util::human_size(job.freed()),
                clock(elapsed.as_secs())
            ),
            Style::default().fg(theme::DIM),
        ),
    ])
}

fn current_line(job: &DeletionJob, width: usize) -> Option<Line<'static>> {
    let index = job.current?;
    let entry = &job.entries[index];
    let frame = (job.started.elapsed().as_millis() / 100) as usize % SPINNER.len();
    Some(Line::from(vec![
        Span::styled(
            format!("  {} ", SPINNER[frame]),
            Style::default().fg(theme::ACCENT),
        ),
        Span::styled(
            format!("{:>10}  ", util::human_size(entry.size)),
            Style::default().fg(theme::DIM),
        ),
        Span::styled(
            output::truncate_start(
                &crate::virtual_entry::display(&entry.path),
                width.saturating_sub(16),
            ),
            Style::default().fg(theme::FG).bold(),
        ),
    ]))
}

/// While running, the most recent results scroll past; once finished, only
/// the failures stay, since those are what needs attention.
fn outcome_lines(job: &DeletionJob, room: usize, width: usize) -> Vec<Line<'static>> {
    if !job.is_finished() {
        return job
            .outcomes
            .iter()
            .rev()
            .take(room)
            .map(|o| outcome_line(o, width))
            .collect();
    }

    let failures: Vec<&Outcome> = job.outcomes.iter().filter(|o| o.error.is_some()).collect();
    if failures.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "  {} failed:",
            output::plural(failures.len(), "item", "items")
        ),
        Style::default().fg(theme::RED).bold(),
    ))];
    lines.extend(failures.iter().take(room).map(|o| outcome_line(o, width)));
    if failures.len() > room {
        lines.push(Line::from(Span::styled(
            format!("  + {} more", failures.len() - room),
            Style::default().fg(theme::DIM),
        )));
    }
    lines
}

fn key_hint(job: &DeletionJob) -> Line<'static> {
    if job.is_finished() {
        return Line::from(vec![
            Span::styled("  Enter ", Style::default().fg(theme::ACCENT).bold()),
            Span::styled("close", Style::default().fg(theme::FG)),
        ]);
    }
    if job.stopping() {
        return Line::from(Span::styled(
            "  Stopping after this item…",
            Style::default().fg(theme::YELLOW),
        ));
    }
    Line::from(vec![
        Span::styled("  Esc ", Style::default().fg(theme::ACCENT).bold()),
        Span::styled("stop after this item", Style::default().fg(theme::FG)),
    ])
}

pub fn render(f: &mut Frame, area: Rect, job: &DeletionJob) {
    let dialog_area = centered_rect(80, 70, area);
    f.render_widget(Clear, dialog_area);

    let width = usize::from(dialog_area.width).saturating_sub(4);
    let room = usize::from(dialog_area.height).saturating_sub(CHROME_LINES + 2);

    let mut lines = vec![
        Line::from(""),
        headline(job),
        Line::from(""),
        gauge(
            job.processed(),
            job.planned(),
            width.saturating_sub(9),
            !job.is_finished(),
        ),
        Line::from(""),
    ];
    lines.extend(current_line(job, width));
    lines.extend(outcome_lines(job, room, width));
    lines.push(Line::from(""));
    lines.push(key_hint(job));

    let title = if job.is_finished() {
        " Deletion finished "
    } else {
        " Deleting "
    };
    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ACCENT))
            .style(Style::default().bg(theme::BG)),
    );
    f.render_widget(dialog, dialog_area);
}
