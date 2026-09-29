use std::io::IsTerminal;

use rustc_hash::FxHashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use yansi::Paint;

use crate::scanner::entry::{Category, DiskInfo, SafetyLevel, ScanResult, ScannedEntry};
use crate::util;
use crate::virtual_entry;

/// Entries listed per category before the rest are summed into one line.
const COMPACT_ENTRIES: usize = 6;
/// Entries listed even when they are small next to the category's largest.
const ALWAYS_LISTED: usize = 3;
/// Beyond `ALWAYS_LISTED`, an entry must hold this share of its category.
const MIN_LISTED_SHARE: f64 = 0.01;

const SIZE_COL: usize = 10;
const BADGE_COL: usize = 7;
const NAME_COL: usize = 22;
const BAR_COL: usize = 12;
const MIN_WIDTH_FOR_BAR: usize = 96;
const DESC_MAX: usize = 48;
const DESC_SHARE_PCT: usize = 45;

/// How many entries a listing shows per category.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Detail {
    /// The largest few, with the rest summed into one line.
    #[default]
    Compact,
    /// Every entry.
    Full,
}

impl Detail {
    pub fn from_verbose(verbose: bool) -> Self {
        if verbose { Self::Full } else { Self::Compact }
    }
}

/// How a category listing is laid out.
#[derive(Clone, Copy, Default)]
pub struct Listing {
    /// Number categories, for the clean prompt to refer back to.
    pub numbered: bool,
    pub detail: Detail,
}

/// Columns available for a line, or `None` when output is not a terminal and
/// nothing should be truncated.
fn terminal_width() -> Option<usize> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    crossterm::terminal::size()
        .ok()
        .map(|(w, _)| usize::from(w))
}

fn paint_safety(text: &str, safety: SafetyLevel) -> String {
    match safety {
        SafetyLevel::Safe => text.green().to_string(),
        SafetyLevel::Caution => text.yellow().to_string(),
        SafetyLevel::Danger => text.red().to_string(),
        SafetyLevel::Error => text.magenta().to_string(),
    }
}

fn safety_label(safety: SafetyLevel) -> &'static str {
    match safety {
        SafetyLevel::Safe => "safe",
        SafetyLevel::Caution => "caution",
        SafetyLevel::Danger => "danger",
        SafetyLevel::Error => "error",
    }
}

/// The safety word padded to one column width, so sizes and descriptions line
/// up across categories whatever the level.
pub fn badge(safety: SafetyLevel) -> String {
    paint_safety(&format!("{:<BADGE_COL$}", safety_label(safety)), safety)
}

/// Cut `text` to `max` display columns, keeping the end, which for a path is
/// the part that names it.
fn truncate_start(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    let mut kept = Vec::new();
    let mut width = 1;
    for c in text.chars().rev() {
        let w = c.width().unwrap_or(0);
        if width + w > max {
            break;
        }
        width += w;
        kept.push(c);
    }
    kept.push('…');
    kept.into_iter().rev().collect()
}

/// Cut `text` to `max` display columns, keeping the start.
fn truncate_end(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut width = 1;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if width + w > max {
            break;
        }
        width += w;
        out.push(c);
    }
    out.push('…');
    out
}

fn pad_to(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(fill))
}

fn share_bar(part: u64, whole: u64) -> String {
    let filled = if whole == 0 {
        0
    } else {
        ((part as f64 / whole as f64) * BAR_COL as f64).round() as usize
    }
    .clamp(usize::from(part > 0), BAR_COL);
    format!(
        "{}{}",
        "━".repeat(filled).cyan(),
        "─".repeat(BAR_COL - filled).dim()
    )
}

/// Group entries by category, largest category first, largest entry first.
pub fn group_by_category<'a>(
    entries: impl IntoIterator<Item = &'a ScannedEntry>,
) -> Vec<(Category, Vec<&'a ScannedEntry>)> {
    let mut groups: indexmap::IndexMap<Category, Vec<&'a ScannedEntry>> = indexmap::IndexMap::new();
    for entry in entries {
        groups.entry(entry.category).or_default().push(entry);
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    for (_, entries) in &mut groups {
        entries.sort_by_key(|e| std::cmp::Reverse(e.size));
    }
    groups
        .sort_by_key(|(_, entries)| std::cmp::Reverse(entries.iter().map(|e| e.size).sum::<u64>()));
    groups
}

/// `12.4 GiB safe · 3.1 GiB caution`, largest share first.
fn safety_mix(entries: &[&ScannedEntry], max_width: Option<usize>) -> String {
    let mut by_level: Vec<(SafetyLevel, u64, usize)> = Vec::new();
    for level in [
        SafetyLevel::Safe,
        SafetyLevel::Caution,
        SafetyLevel::Danger,
        SafetyLevel::Error,
    ] {
        let matching: Vec<_> = entries.iter().filter(|e| e.safety == level).collect();
        if !matching.is_empty() {
            by_level.push((level, matching.iter().map(|e| e.size).sum(), matching.len()));
        }
    }
    if let [(level, _, _)] = by_level.as_slice() {
        return paint_safety(safety_label(*level), *level);
    }

    // Error entries hold no reclaimable bytes; how many there are is the
    // useful part.
    let amount = |level: SafetyLevel, size: u64, count: usize| {
        if level == SafetyLevel::Error {
            count.to_string()
        } else {
            util::human_size(size)
        }
    };
    let full_width: usize = by_level
        .iter()
        .map(|&(level, size, count)| {
            amount(level, size, count).width() + 1 + safety_label(level).len() + 3
        })
        .sum();
    let with_amounts = max_width.is_none_or(|max| full_width <= max);
    let labels_width: usize = by_level
        .iter()
        .map(|&(level, _, _)| safety_label(level).len() + 3)
        .sum();
    if max_width.is_some_and(|max| labels_width > max) {
        // Too narrow even for the labels: name the riskiest level present.
        let worst = by_level
            .iter()
            .map(|&(level, _, _)| level)
            .filter(|l| *l != SafetyLevel::Error)
            .max()
            .unwrap_or(SafetyLevel::Error);
        return paint_safety(safety_label(worst), worst);
    }

    by_level
        .iter()
        .map(|&(level, size, count)| {
            let label = paint_safety(safety_label(level), level);
            if with_amounts {
                format!("{} {label}", amount(level, size, count))
            } else {
                label
            }
        })
        .collect::<Vec<_>>()
        .join(&format!(" {} ", "·".dim()))
}

/// How many of a category's size-sorted entries to list individually.
fn listed_count(entries: &[&ScannedEntry], total: u64, detail: Detail) -> usize {
    if detail == Detail::Full {
        return entries.len();
    }
    entries
        .iter()
        .take(COMPACT_ENTRIES)
        .enumerate()
        .take_while(|(i, e)| *i < ALWAYS_LISTED || e.size as f64 >= total as f64 * MIN_LISTED_SHARE)
        .count()
}

/// What the entries left out of a compact listing mostly are, by the name of
/// the directory or file each one removes.
fn dominant_name(rest: &[&ScannedEntry]) -> Option<String> {
    let mut counts: FxHashMap<String, usize> = FxHashMap::default();
    for entry in rest {
        if virtual_entry::is_virtual(&entry.path) {
            continue;
        }
        if let Some(name) = entry.path.file_name() {
            *counts
                .entry(name.to_string_lossy().into_owned())
                .or_default() += 1;
        }
    }
    let (name, count) = counts.into_iter().max_by_key(|(_, c)| *c)?;
    (count > 1 && count * 2 >= rest.len()).then(|| format!("{count} × {name}"))
}

fn print_category_header(
    index: Option<usize>,
    category: Category,
    entries: &[&ScannedEntry],
    grand_total: u64,
    width: Option<usize>,
) {
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let pct = if grand_total > 0 {
        total as f64 / grand_total as f64 * 100.0
    } else {
        0.0
    };
    let number = index.map_or_else(String::new, |i| {
        format!("{} ", format!("{:>3}", i + 1).bold())
    });
    let share = if pct > 0.0 && pct < 1.0 {
        "<1%".to_owned()
    } else {
        format!("{pct:.0}%")
    };
    let count = entries.len();
    let items = if count == 1 { "item" } else { "items" };
    // The share bar is the first thing to go on a narrow terminal; the
    // numbers beside it say the same.
    let show_bar = width.is_none_or(|w| w >= MIN_WIDTH_FOR_BAR);
    let bar = if show_bar {
        format!("{}  ", share_bar(total, grand_total))
    } else {
        String::new()
    };
    let used = index.map_or(0, |_| 4)
        + NAME_COL
        + 2
        + SIZE_COL
        + 2
        + if show_bar { BAR_COL + 2 } else { 0 }
        + 4
        + 2
        + 10
        + 2;
    let mix_room = width.map(|w| w.saturating_sub(used));
    println!(
        "{number}{}  {:>SIZE_COL$}  {bar}{share:>4}  {:>4} {items}  {}",
        pad_to(&category.to_string(), NAME_COL).bold(),
        util::human_size(total).bold(),
        count,
        safety_mix(entries, mix_room),
    );
}

/// The description, unless all it says is the path printed beside it.
fn description_of(entry: &ScannedEntry) -> &str {
    if entry.description == virtual_entry::display(&entry.path) {
        ""
    } else {
        &entry.description
    }
}

/// Column widths shared by every entry line in one category.
struct Columns {
    indent: usize,
    desc: usize,
    path: usize,
}

fn print_entry_line(entry: &ScannedEntry, cols: &Columns, width: Option<usize>) {
    let Columns {
        indent,
        desc: desc_col,
        path: path_col,
    } = *cols;
    let path = virtual_entry::display(&entry.path);
    let fixed = indent + SIZE_COL + 2 + BADGE_COL + 2;
    let description = description_of(entry);

    let (desc, path) = match width {
        None => (pad_to(description, desc_col), path),
        Some(width) => {
            let room = width.saturating_sub(fixed);
            // Descriptions get what the paths leave over, but never so much
            // that a long path, which is what gets deleted, is cut to nothing.
            let desc_share = room * DESC_SHARE_PCT / 100;
            let desc_width = desc_col.min(desc_share.max(room.saturating_sub(path_col + 2)));
            let desc = pad_to(&truncate_end(description, desc_width), desc_width);
            // Whatever is left after the description and a gap goes to the path;
            // too little to be useful and the path is dropped.
            let path_room = room.saturating_sub(desc_width + 2);
            let path = if path_room >= 12 {
                truncate_start(&path, path_room)
            } else {
                String::new()
            };
            (desc, path)
        }
    };

    println!(
        "{}{:>SIZE_COL$}  {}  {desc}  {}",
        " ".repeat(indent),
        util::human_size(entry.size),
        badge(entry.safety),
        path.dim(),
    );
}

/// The per-category listing shared by `scan` and `clean`.
pub fn print_listing(groups: &[(Category, Vec<&ScannedEntry>)], listing: Listing) {
    let grand_total: u64 = groups
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|e| e.size))
        .sum();
    let width = terminal_width();
    let indent = if listing.numbered { 6 } else { 2 };

    for (i, (category, entries)) in groups.iter().enumerate() {
        println!();
        print_category_header(
            listing.numbered.then_some(i),
            *category,
            entries,
            grand_total,
            width,
        );

        let total: u64 = entries.iter().map(|e| e.size).sum();
        let shown = listed_count(entries, total, listing.detail);
        let listed = &entries[..shown];
        let cols = Columns {
            indent,
            desc: listed
                .iter()
                .map(|e| description_of(e).width())
                .max()
                .unwrap_or(0)
                .min(DESC_MAX),
            path: listed
                .iter()
                .map(|e| virtual_entry::display(&e.path).width())
                .max()
                .unwrap_or(0),
        };
        for entry in listed {
            print_entry_line(entry, &cols, width);
        }

        let rest = &entries[shown..];
        if !rest.is_empty() {
            let rest_size: u64 = rest.iter().map(|e| e.size).sum();
            let what = dominant_name(rest).map_or_else(String::new, |n| format!(", mostly {n}"));
            println!(
                "{}{}",
                " ".repeat(indent + SIZE_COL + 2),
                format!(
                    "+ {} more, {}{what}",
                    rest.len(),
                    util::human_size(rest_size)
                )
                .dim()
            );
        }
    }
}

pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The footer's total line, with the safety split on a second line when the
/// entries are not all at one level.
pub fn print_totals(entries: &[&ScannedEntry], categories: usize, qualifier: &str) {
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let uniform = entries
        .first()
        .filter(|first| entries.iter().all(|e| e.safety == first.safety))
        .map(|first| first.safety);
    let all_one = uniform.map_or_else(String::new, |level| {
        format!(", all {}", paint_safety(safety_label(level), level))
    });
    println!(
        "  {}{qualifier} in {} across {}{all_one}",
        util::human_size(total).bold().green(),
        plural(entries.len(), "item", "items"),
        plural(categories, "category", "categories"),
    );
    if uniform.is_none() && !entries.is_empty() {
        println!("  {}", safety_mix(entries, None));
    }
}

pub fn rule() {
    let width = terminal_width().unwrap_or(80).min(100);
    println!("{}", "─".repeat(width).dim());
}

pub fn print_table(result: &ScanResult, detail: Detail) {
    print_header(result);

    if let Some(ref disk) = result.disk_info {
        print_disk_info(disk);
    }

    let groups = group_by_category(&result.entries);
    if groups.is_empty() {
        println!("\n{}", "Nothing reclaimable found.".dim());
        return;
    }

    print_listing(
        &groups,
        Listing {
            numbered: false,
            detail,
        },
    );

    println!();
    rule();
    let all: Vec<&ScannedEntry> = result.entries.iter().collect();
    print_totals(&all, groups.len(), " reclaimable");
    let mut hints = Vec::new();
    let truncated = groups.iter().any(|(_, entries)| {
        let total = entries.iter().map(|e| e.size).sum();
        listed_count(entries, total, Detail::Compact) < entries.len()
    });
    if detail == Detail::Compact && truncated {
        hints.push(format!("{} to list every item", "-v".bold()));
    }
    hints.push(format!("{} to review and delete", "sweeprs clean".bold()));
    println!("  {}", hints.join(&format!(" {} ", "·".dim())).dim());
}

pub fn print_json(result: &ScanResult) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(result)?;
    println!("{json}");
    Ok(())
}

fn print_header(result: &ScanResult) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
    let duration = result
        .scan_duration_secs
        .map(|d| format!(" · {d:.1}s"))
        .unwrap_or_default();
    println!(
        "{} {}",
        "sweeprs scan".bold(),
        format!("· {timestamp}{duration}").dim()
    );
}

fn usage_bar(pct: f64, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let color = if pct >= 95.0 {
        yansi::Color::Red
    } else if pct >= 85.0 {
        yansi::Color::Yellow
    } else {
        yansi::Color::Green
    };
    format!(
        "{}{}",
        "█".repeat(filled).fg(color),
        "░".repeat(width - filled).dim()
    )
}

/// Width of the label column in the disk summary.
const DISK_LABEL: usize = 16;

fn print_volume(name: &str, pct: f64, used: u64, total: u64, width: Option<usize>) {
    let figures = format!(
        "{pct:.0}%  {} of {} used",
        util::human_size(used),
        util::human_size(total)
    );
    let fixed = 2 + DISK_LABEL + 2 + 2 + figures.width();
    let bar = width.map_or(30, |w| w.saturating_sub(fixed).min(30));
    let bar = if bar >= 8 {
        format!("{}  ", usage_bar(pct, bar))
    } else {
        String::new()
    };
    println!(
        "  {}  {bar}{figures}",
        pad_to(&truncate_end(name, DISK_LABEL), DISK_LABEL).bold(),
    );
}

fn print_disk_info(disk: &DiskInfo) {
    let width = terminal_width();
    println!();
    print_volume(
        &disk.name,
        disk.usage_percent,
        disk.used_bytes,
        disk.total_bytes,
        width,
    );

    // (plain text for measuring, painted text for printing)
    let mut facts: Vec<(String, String)> = Vec::new();
    let mut fact = |amount: u64, what: &str, paint: fn(&str) -> String| {
        let amount = util::human_size(amount);
        facts.push((
            format!("{amount} {what}"),
            format!("{} {what}", paint(&amount)),
        ));
    };
    fact(disk.available_bytes, "free", |a| a.green().to_string());
    if let Some(purgeable) = disk.purgeable_bytes.filter(|b| *b > 0) {
        fact(purgeable, "purgeable", str::to_owned);
    }
    if disk.snapshot_bytes > 0 {
        fact(disk.snapshot_bytes, "in snapshots", |a| {
            a.yellow().to_string()
        });
    }
    if let Some(icloud) = disk.icloud_local_bytes.filter(|b| *b > 0) {
        fact(icloud, "iCloud local", str::to_owned);
    }
    if let Some(system_app) = disk.system_app_bytes.filter(|b| *b > 0) {
        fact(system_app, "system + apps", str::to_owned);
    }
    if let Some(tm) = disk.tm_reclaimable_bytes.filter(|b| *b > 0) {
        fact(tm, "in older Time Machine snapshots", |a| {
            a.yellow().to_string()
        });
    }

    // Wrap the facts under the bar rather than past the terminal's edge.
    let lead = " ".repeat(2 + DISK_LABEL + 2);
    let room = width.map_or(usize::MAX, |w| w.saturating_sub(lead.len()));
    let separator = format!(" {} ", "·".dim());
    let mut line: Vec<String> = Vec::new();
    let mut line_width = 0;
    for (plain, painted) in facts {
        let added = plain.width() + if line.is_empty() { 0 } else { 3 };
        if !line.is_empty() && line_width + added > room {
            println!("{lead}{}", line.join(&separator));
            Vec::clear(&mut line);
            line_width = 0;
        }
        line_width += plain.width() + if line.is_empty() { 0 } else { 3 };
        line.push(painted);
    }
    if !line.is_empty() {
        println!("{lead}{}", line.join(&separator));
    }

    for vol in &disk.other_volumes {
        print_volume(
            &vol.name,
            vol.usage_percent,
            vol.used_bytes,
            vol.total_bytes,
            width,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(path: &str, size: u64) -> ScannedEntry {
        ScannedEntry {
            path: PathBuf::from(path),
            size,
            category: Category::BuildArtifact,
            safety: SafetyLevel::Safe,
            description: String::new(),
            item_count: None,
        }
    }

    #[test]
    fn a_long_path_keeps_its_end() {
        let cut = truncate_start("~/Workspace/acme/project/target", 16);
        assert_eq!(cut.width(), 16);
        assert!(cut.ends_with("project/target"));
        assert!(cut.starts_with('…'));
    }

    #[test]
    fn wide_characters_are_measured_by_display_width() {
        let cut = truncate_end("レポート資料フォルダ", 9);
        assert!(cut.width() <= 9);
    }

    #[test]
    fn compact_listing_drops_the_tail_but_keeps_a_few() {
        let owned = [
            entry("/a/target", 60_000),
            entry("/b/__pycache__", 12),
            entry("/c/__pycache__", 12),
            entry("/d/__pycache__", 12),
            entry("/e/__pycache__", 12),
        ];
        let entries: Vec<&ScannedEntry> = owned.iter().collect();
        let total = entries.iter().map(|e| e.size).sum();

        assert_eq!(
            listed_count(&entries, total, Detail::Compact),
            ALWAYS_LISTED
        );
        assert_eq!(listed_count(&entries, total, Detail::Full), entries.len());
        assert_eq!(
            dominant_name(&entries[ALWAYS_LISTED..]).as_deref(),
            Some("2 × __pycache__")
        );
    }
}
