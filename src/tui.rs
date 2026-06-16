//! Shared building blocks for the interactive TUIs (`ilog`, `ibranch`):
//! loading commits/files, side-by-side syntax-highlighted diff rendering (pure
//! Rust via syntect), and common widget helpers.

use crate::git::git_capture;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, ListItem};
use ratatui::DefaultTerminal;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// ASCII unit separator — a field delimiter that won't appear in our fields.
pub const SEP: char = '\u{1f}';

/// Tint for changed cells (left = removed, right = added).
const REMOVED_BG: Color = Color::Rgb(55, 24, 24);
const ADDED_BG: Color = Color::Rgb(24, 46, 24);

pub struct Commit {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub committer: String,
    pub subject: String,
}

pub struct FileEntry {
    pub status: char,
    pub path: String,
}

// ── loaders ─────────────────────────────────────────────────────────────────

/// Load commits as `git log -n <limit> [extra…]`. `extra` is e.g. `["--all"]`
/// or a branch name, appended after the format args.
pub fn load_commits(extra: &[&str], limit: usize) -> Vec<Commit> {
    let n = limit.to_string();
    let fmt = format!("--pretty=format:%H{SEP}%h{SEP}%ad{SEP}%cn{SEP}%s");
    let mut args = vec!["log", "-n", &n, "--date=format:%Y-%m-%d %H:%M", &fmt];
    args.extend_from_slice(extra);
    git_capture(".", &args)
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    Some(Commit {
                        hash: f.next()?.to_string(),
                        short: f.next()?.to_string(),
                        date: f.next()?.to_string(),
                        committer: f.next()?.to_string(),
                        subject: f.next().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The files a commit touched, with their status (cheap — no diff content).
pub fn load_files(hash: &str) -> Vec<FileEntry> {
    git_capture(".", &["show", "--name-status", "--format=", hash])
        .map(|out| {
            out.lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| {
                    let mut parts = line.split('\t');
                    let status = parts.next()?.chars().next()?;
                    // last field handles renames ("R100\told\tnew" -> new path)
                    let path = parts.next_back()?.to_string();
                    Some(FileEntry { status, path })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One file's diff from a commit, rendered side-by-side at the given width.
pub fn load_file_diff(hash: &str, path: &str, width: u16) -> Text<'static> {
    let raw = git_capture(".", &["show", "--format=", hash, "--", path]).unwrap_or_default();
    side_by_side(&raw, width)
}

// ── widget helpers ──────────────────────────────────────────────────────────

pub fn commit_item(c: &Commit) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(format!("{:<8}", c.short), Style::default().fg(Color::Yellow)),
        Span::styled(format!("{}  ", c.date), Style::default().fg(Color::Green)),
        Span::styled(format!("<{}> ", c.committer), Style::default().fg(Color::Blue)),
        Span::raw(c.subject.clone()),
    ]))
}

pub fn file_item(f: &FileEntry) -> ListItem<'static> {
    let (color, ch) = status_glyph(f.status);
    ListItem::new(Line::from(vec![
        Span::styled(format!("{ch}  "), Style::default().fg(color)),
        Span::raw(f.path.clone()),
    ]))
}

fn status_glyph(status: char) -> (Color, char) {
    match status {
        'A' => (Color::Green, 'A'),
        'M' => (Color::Yellow, 'M'),
        'D' => (Color::Red, 'D'),
        'R' => (Color::Cyan, 'R'),
        c => (Color::Gray, c),
    }
}

/// A bordered block whose border is bright when the pane is focused.
pub fn pane_block(title: String, active: bool) -> Block<'static> {
    let color = if active { Color::Cyan } else { Color::DarkGray };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(title)
}

// ── shared key predicates (arrow keys mirror j/k everywhere) ────────────────

/// Down = j or ↓.
pub fn is_down(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('j') | KeyCode::Down)
}
/// Up = k or ↑.
pub fn is_up(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('k') | KeyCode::Up)
}
/// Open/drill-in = Enter, l, or →.
pub fn is_open(c: KeyCode) -> bool {
    matches!(c, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l'))
}
/// Back/step-out = Esc, h, or ←.
pub fn is_back(c: KeyCode) -> bool {
    matches!(c, KeyCode::Esc | KeyCode::Left | KeyCode::Char('h'))
}

pub fn pane_width(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| s.width.saturating_sub(2))
        .unwrap_or(120)
}

/// Half the height of the lower (~60%) pane, for vim Ctrl-d/Ctrl-u.
pub fn half_page(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| {
            let pane = (s.height as u32 * 6 / 10).saturating_sub(2);
            ((pane / 2).max(1)) as u16
        })
        .unwrap_or(10)
}

// ── side-by-side diff rendering ─────────────────────────────────────────────

/// Two highlighters per file — one for the old side, one for the new — so a
/// removed line's syntax state never corrupts the added side and vice versa.
struct FileHl {
    old: HighlightLines<'static>,
    new: HighlightLines<'static>,
}

/// Parse `git show` output into side-by-side rows: removed lines on the left
/// (red), added on the right (green), context on both, headers full-width.
fn side_by_side(raw: &str, width: u16) -> Text<'static> {
    let ps = syntaxes();
    let theme = theme();

    let inner = width as usize;
    let col = inner.saturating_sub(3) / 2; // " │ " separator = 3 cols
    let lw = col.max(1);
    let rw = inner.saturating_sub(3).saturating_sub(lw).max(1);

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut hl: Option<FileHl> = None;
    let mut in_patch = false;
    let mut rem: Vec<Vec<Span<'static>>> = Vec::new();
    let mut add: Vec<Vec<Span<'static>>> = Vec::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_change(&mut out, &mut rem, &mut add, lw, rw);
            in_patch = true;
            let syntax = syntax_for(ps, rest.rsplit(" b/").next().unwrap_or(""));
            hl = Some(FileHl {
                old: HighlightLines::new(syntax, theme),
                new: HighlightLines::new(syntax, theme),
            });
            out.push(plain(line, dim().add_modifier(Modifier::BOLD)));
            continue;
        }

        if !in_patch {
            if line.is_empty() {
                continue;
            }
            out.push(plain(line, Style::default()));
            continue;
        }

        if line.starts_with("@@") {
            flush_change(&mut out, &mut rem, &mut add, lw, rw);
            out.push(plain(line, Style::default().fg(Color::Cyan)));
            continue;
        }
        if is_meta(line) {
            flush_change(&mut out, &mut rem, &mut add, lw, rw);
            out.push(plain(line, dim()));
            continue;
        }

        if let Some(code) = line.strip_prefix('-') {
            let spans = hl_old(&mut hl, ps, code);
            rem.push(spans);
        } else if let Some(code) = line.strip_prefix('+') {
            let spans = hl_new(&mut hl, ps, code);
            add.push(spans);
        } else if let Some(code) = line.strip_prefix(' ') {
            flush_change(&mut out, &mut rem, &mut add, lw, rw);
            let l = hl_old(&mut hl, ps, code);
            let r = hl_new(&mut hl, ps, code);
            out.push(row(l, lw, None, r, rw, None));
        } else {
            flush_change(&mut out, &mut rem, &mut add, lw, rw);
            out.push(plain(line, dim()));
        }
    }
    flush_change(&mut out, &mut rem, &mut add, lw, rw);

    Text::from(out)
}

fn flush_change(
    out: &mut Vec<Line<'static>>,
    rem: &mut Vec<Vec<Span<'static>>>,
    add: &mut Vec<Vec<Span<'static>>>,
    lw: usize,
    rw: usize,
) {
    let n = rem.len().max(add.len());
    for i in 0..n {
        let (lspans, lbg) = match rem.get(i) {
            Some(s) => (s.clone(), Some(REMOVED_BG)),
            None => (Vec::new(), None),
        };
        let (rspans, rbg) = match add.get(i) {
            Some(s) => (s.clone(), Some(ADDED_BG)),
            None => (Vec::new(), None),
        };
        out.push(row(lspans, lw, lbg, rspans, rw, rbg));
    }
    rem.clear();
    add.clear();
}

fn row(
    lspans: Vec<Span<'static>>,
    lw: usize,
    lbg: Option<Color>,
    rspans: Vec<Span<'static>>,
    rw: usize,
    rbg: Option<Color>,
) -> Line<'static> {
    let mut spans = fit_cell(lspans, lw, lbg);
    spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
    spans.extend(fit_cell(rspans, rw, rbg));
    Line::from(spans)
}

/// Truncate/pad a cell's spans to exactly `width` display columns, applying an
/// optional background to the whole cell (including the padding).
fn fit_cell(spans: Vec<Span<'static>>, width: usize, bg: Option<Color>) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for sp in spans {
        if used >= width {
            break;
        }
        let chars: Vec<char> = sp.content.chars().collect();
        let take = chars.len().min(width - used);
        let text: String = chars[..take].iter().collect();
        let mut style = sp.style;
        if let Some(bg) = bg {
            style = style.bg(bg);
        }
        out.push(Span::styled(text, style));
        used += take;
    }
    if used < width {
        let mut style = Style::default();
        if let Some(bg) = bg {
            style = style.bg(bg);
        }
        out.push(Span::styled(" ".repeat(width - used), style));
    }
    out
}

fn hl_old(hl: &mut Option<FileHl>, ps: &SyntaxSet, code: &str) -> Vec<Span<'static>> {
    match hl {
        Some(h) => hl_spans(&mut h.old, ps, code),
        None => vec![Span::raw(code.to_string())],
    }
}

fn hl_new(hl: &mut Option<FileHl>, ps: &SyntaxSet, code: &str) -> Vec<Span<'static>> {
    match hl {
        Some(h) => hl_spans(&mut h.new, ps, code),
        None => vec![Span::raw(code.to_string())],
    }
}

fn hl_spans(h: &mut HighlightLines, ps: &SyntaxSet, code: &str) -> Vec<Span<'static>> {
    let with_nl = format!("{code}\n");
    match h.highlight_line(&with_nl, ps) {
        Ok(ranges) => ranges
            .into_iter()
            .filter_map(|(sty, text)| {
                let text = text.trim_end_matches('\n');
                (!text.is_empty()).then(|| Span::styled(text.to_string(), syn_to_ratatui(sty)))
            })
            .collect(),
        Err(_) => vec![Span::raw(code.to_string())],
    }
}

fn is_meta(line: &str) -> bool {
    line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("new file")
        || line.starts_with("deleted file")
        || line.starts_with("similarity ")
        || line.starts_with("rename ")
        || line.starts_with("\\ No newline")
}

fn syntax_for<'a>(ps: &'a SyntaxSet, path: &str) -> &'a SyntaxReference {
    path.rsplit('.')
        .next()
        .filter(|ext| *ext != path)
        .and_then(|ext| ps.find_syntax_by_extension(ext))
        .unwrap_or_else(|| ps.find_syntax_plain_text())
}

fn plain(text: &str, style: Style) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), style))
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn syn_to_ratatui(s: SynStyle) -> Style {
    let fg = s.foreground;
    let mut style = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
    if s.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let mut ts = ThemeSet::load_defaults();
        ts.themes
            .remove("base16-eighties.dark")
            .or_else(|| ts.themes.remove("base16-ocean.dark"))
            .unwrap_or_else(|| ts.themes.values().next().cloned().unwrap_or_default())
    })
}
