use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::output;
use crate::scanner::entry::{SafetyLevel, ScannedEntry};
use crate::tui::theme;
use crate::util;
use crate::virtual_entry;

/// Lines the dialog spends on everything but the item list.
const CHROME_LINES: usize = 11;

/// A merged description reads `first | second | third`; the first part is the
/// one that names the thing.
pub fn short_description(description: &str) -> &str {
    description.split(" | ").next().unwrap_or(description)
}

pub fn safety_word(safety: SafetyLevel) -> Span<'static> {
    let word = match safety {
        SafetyLevel::Safe => "safe",
        SafetyLevel::Caution => "caution",
        SafetyLevel::Danger => "danger",
        SafetyLevel::Error => "error",
    };
    Span::styled(
        format!("{word:<7}"),
        Style::default().fg(theme::safety_color(safety)),
    )
}

pub fn render(f: &mut Frame, area: Rect, entries: &[ScannedEntry]) {
    let dialog_area = centered_rect(80, 75, area);
    f.render_widget(Clear, dialog_area);

    let total_size: u64 = entries.iter().map(|e| e.size).sum();
    let danger = entries
        .iter()
        .filter(|e| e.safety == SafetyLevel::Danger)
        .count();

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Delete ", Style::default().fg(theme::FG).bold()),
            Span::styled(
                output::plural(entries.len(), "item", "items"),
                Style::default().fg(theme::FG).bold(),
            ),
            Span::styled(", ", Style::default().fg(theme::FG).bold()),
            Span::styled(
                util::human_size(total_size),
                Style::default().fg(theme::GREEN).bold(),
            ),
            Span::styled("?", Style::default().fg(theme::FG).bold()),
        ]),
        Line::from(""),
    ];
    let room = usize::from(dialog_area.height).saturating_sub(CHROME_LINES + 2);
    lines.extend(entry_lines(
        entries,
        room,
        usize::from(dialog_area.width).saturating_sub(4),
    ));

    lines.push(Line::from(""));
    lines.push(safety_totals(entries));
    if danger > 0 {
        lines.push(Line::from(""));
        let warning = if danger == 1 {
            "  1 danger item has no other copy. Deleting it loses it for good.".to_owned()
        } else {
            format!(
                "  {danger} danger items have no other copy. Deleting them loses them for good."
            )
        };
        lines.push(Line::from(Span::styled(
            warning,
            Style::default().fg(theme::RED).bold(),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  y ", Style::default().fg(theme::GREEN).bold()),
        Span::styled("delete", Style::default().fg(theme::FG)),
        Span::styled("     n / Esc ", Style::default().fg(theme::ACCENT).bold()),
        Span::styled("cancel", Style::default().fg(theme::FG)),
    ]));

    let border_color = if danger > 0 {
        theme::RED
    } else {
        theme::ACCENT
    };
    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Confirm deletion ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(theme::BG)),
    );
    f.render_widget(dialog, dialog_area);
}

/// The entries largest first, as many as fit in `room` lines.
fn entry_lines(entries: &[ScannedEntry], room: usize, inner_width: usize) -> Vec<Line<'static>> {
    let mut sorted: Vec<&ScannedEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.size));

    let shown = if sorted.len() > room {
        room.saturating_sub(1)
    } else {
        sorted.len()
    };
    let desc_col = sorted[..shown]
        .iter()
        .map(|e| short_description(&e.description).width())
        .max()
        .unwrap_or(0)
        .min(inner_width * 45 / 100);
    let path_room = inner_width.saturating_sub(2 + 10 + 2 + 7 + 2 + desc_col + 2);

    let mut lines = Vec::new();

    for entry in &sorted[..shown] {
        let description = output::truncate_end(short_description(&entry.description), desc_col);
        let pad = desc_col.saturating_sub(description.width());
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:>10}  ", util::human_size(entry.size)),
                Style::default().fg(theme::FG),
            ),
            safety_word(entry.safety),
            Span::styled(
                format!("  {description}{}  ", " ".repeat(pad)),
                Style::default().fg(theme::FG),
            ),
            Span::styled(
                output::truncate_start(&virtual_entry::display(&entry.path), path_room),
                Style::default().fg(theme::DIM),
            ),
        ]));
    }
    if shown < sorted.len() {
        let rest = &sorted[shown..];
        lines.push(Line::from(Span::styled(
            format!(
                "  {:>10}  + {} more",
                util::human_size(rest.iter().map(|e| e.size).sum()),
                rest.len()
            ),
            Style::default().fg(theme::DIM),
        )));
    }

    lines
}

/// `81.20 GiB safe · 53.10 GiB caution`
fn safety_totals(entries: &[ScannedEntry]) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    for level in [SafetyLevel::Safe, SafetyLevel::Caution, SafetyLevel::Danger] {
        let size: u64 = entries
            .iter()
            .filter(|e| e.safety == level)
            .map(|e| e.size)
            .sum();
        if size == 0 && !entries.iter().any(|e| e.safety == level) {
            continue;
        }
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", Style::default().fg(theme::DIM)));
        }
        spans.push(Span::styled(
            format!("{} ", util::human_size(size)),
            Style::default().fg(theme::FG),
        ));
        spans.push(Span::styled(
            safety_word(level).content.trim_end().to_owned(),
            Style::default().fg(theme::safety_color(level)),
        ));
    }
    Line::from(spans)
}

pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}
