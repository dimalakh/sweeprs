use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::output;
use crate::tui::app::App;
use crate::tui::theme;
use crate::tui::tree::{CheckState, EntryNode, RowRef};
use crate::util;
use crate::virtual_entry;

const BAR_WIDTH: usize = 10;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let viewport_height = inner.height as usize;
    let width = inner.width as usize;
    if viewport_height == 0 {
        return;
    }

    app.clamp_scroll_to_viewport(viewport_height);

    // Compute all mutable-access data upfront, then borrow tree immutably for rendering.
    let visible = app.tree.visible_rows();
    let total_reclaimable = app.tree.total_reclaimable();

    if visible.is_empty() {
        let empty = Paragraph::new("No items found. Press 'r' to scan.")
            .style(Style::default().fg(theme::DIM));
        f.render_widget(empty, inner);
        return;
    }

    // Pre-compute check states for all visible categories and groups in this window.
    let end = (app.scroll_offset + viewport_height).min(visible.len());
    let window = &visible[app.scroll_offset..end];

    // Ensure check states are cached before we borrow tree immutably.
    app.tree.ensure_check_cache();

    let lines: Vec<Line<'_>> = window
        .iter()
        .enumerate()
        .map(|(vi, row)| {
            let abs_index = app.scroll_offset + vi;
            let is_cursor = abs_index == app.cursor;

            match *row {
                RowRef::Category(ci) => {
                    render_category_row(&app.tree, ci, is_cursor, total_reclaimable, width)
                }
                RowRef::Group(ci, gi) => render_group_row(&app.tree, ci, gi, is_cursor, width),
                RowRef::Entry(ci, gi, ei) => {
                    render_entry_row(&app.tree, ci, gi, ei, is_cursor, width)
                }
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn size_bar(
    size: u64,
    max_size: u64,
    width: usize,
    fill_color: ratatui::style::Color,
) -> Span<'static> {
    if max_size == 0 {
        return Span::styled(
            "\u{2591}".repeat(width),
            Style::default().fg(theme::BAR_FILL),
        );
    }

    let ratio = size as f64 / max_size as f64;
    let filled = (ratio * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut bar = "\u{2588}".repeat(filled);
    bar.push_str(&"\u{2591}".repeat(empty));

    Span::styled(bar, Style::default().fg(fill_color))
}

/// Right-hand columns: item count, size, share bar.
const COUNT_COL: usize = 9;
const SIZE_COL: usize = 10;
const RIGHT_COLS: usize = COUNT_COL + 2 + SIZE_COL + 2 + BAR_WIDTH + 1;

fn count_label(count: usize, always: bool) -> String {
    match count {
        1 if !always => String::new(),
        1 => "1 item".to_owned(),
        n => format!("{n} items"),
    }
}

fn check_box(state: CheckState) -> &'static str {
    match state {
        CheckState::Checked => "[x]",
        CheckState::Partial => "[-]",
        CheckState::Unchecked => "[ ]",
    }
}

/// One tree row: a prefix, a name that takes whatever width is left, then
/// count, size and bar in fixed columns so they line up down the panel.
struct Row<'a> {
    prefix: String,
    prefix_style: Style,
    name: &'a str,
    name_style: Style,
    /// Paths keep their end when shortened; labels keep their start.
    keep_end: bool,
    count: String,
    size: u64,
    size_style: Style,
    bar: Span<'static>,
}

impl Row<'_> {
    fn render(self, width: usize, is_cursor: bool) -> Line<'static> {
        use unicode_width::UnicodeWidthStr;

        let name_room = width.saturating_sub(self.prefix.width() + RIGHT_COLS + 1);
        let name = if self.keep_end {
            output::truncate_start(self.name, name_room)
        } else {
            output::truncate_end(self.name, name_room)
        };
        let fill = name_room.saturating_sub(name.width());

        let spans = vec![
            Span::styled(self.prefix, self.prefix_style),
            Span::styled(name, self.name_style),
            Span::raw(" ".repeat(fill + 1)),
            Span::styled(
                format!("{:>COUNT_COL$}  ", self.count),
                Style::default().fg(theme::DIM),
            ),
            Span::styled(
                format!("{:>SIZE_COL$}  ", util::human_size(self.size)),
                self.size_style,
            ),
            self.bar,
        ];

        let style = if is_cursor {
            Style::default().bg(theme::SURFACE)
        } else {
            Style::default()
        };
        Line::from(spans).style(style)
    }
}

fn render_category_row(
    tree: &crate::tui::tree::Tree,
    ci: usize,
    is_cursor: bool,
    total_reclaimable: u64,
    width: usize,
) -> Line<'static> {
    let cat = &tree.categories[ci];
    let arrow = if cat.expanded { "▾" } else { "▸" };
    let check = check_box(tree.cached_category_check_state(ci));
    let safety_color = theme::safety_color(cat.category.default_safety());

    Row {
        prefix: format!(" {arrow} {check} "),
        prefix_style: Style::default().fg(theme::ACCENT),
        name: &cat.category.to_string(),
        name_style: Style::default().fg(theme::FG).bold(),
        keep_end: false,
        count: count_label(cat.entry_count, true),
        size: cat.total_size,
        size_style: Style::default().fg(safety_color).bold(),
        bar: size_bar(cat.total_size, total_reclaimable, BAR_WIDTH, safety_color),
    }
    .render(width, is_cursor)
}

fn render_group_row(
    tree: &crate::tui::tree::Tree,
    ci: usize,
    gi: usize,
    is_cursor: bool,
    width: usize,
) -> Line<'static> {
    let cat = &tree.categories[ci];
    let group = &cat.groups[gi];
    let arrow = if group.expanded { "▾" } else { "▸" };
    let selectable = group.entries.iter().any(EntryNode::selectable);
    let check = if selectable {
        check_box(tree.cached_group_check_state(ci, gi))
    } else {
        "[!]"
    };
    let safety_color = theme::safety_color(group.safety);

    Row {
        prefix: format!("   {arrow} {check} "),
        prefix_style: Style::default().fg(theme::ACCENT),
        name: &group.name,
        name_style: Style::default().fg(theme::FG),
        keep_end: false,
        count: count_label(group.entries.len(), false),
        size: group.total_size,
        size_style: Style::default().fg(safety_color).bold(),
        bar: size_bar(group.total_size, cat.total_size, BAR_WIDTH, safety_color),
    }
    .render(width, is_cursor)
}

fn render_entry_row(
    tree: &crate::tui::tree::Tree,
    ci: usize,
    gi: usize,
    ei: usize,
    is_cursor: bool,
    width: usize,
) -> Line<'static> {
    let group = &tree.categories[ci].groups[gi];
    let entry = &group.entries[ei];
    let check = match (entry.selectable(), entry.checked) {
        (false, _) => "[!]",
        (true, true) => "[x]",
        (true, false) => "[ ]",
    };
    let safety_color = theme::safety_color(entry.safety);

    Row {
        prefix: format!("       {check} "),
        prefix_style: Style::default().fg(safety_color),
        name: &virtual_entry::display(&entry.path),
        name_style: Style::default().fg(theme::FG),
        keep_end: true,
        count: String::new(),
        size: entry.size,
        size_style: Style::default().fg(safety_color),
        bar: size_bar(entry.size, group.total_size, BAR_WIDTH, safety_color),
    }
    .render(width, is_cursor)
}
