//! Shared building blocks for the interactive TUIs (`ilog`, `ibranch`):
//! loading commits/files, side-by-side syntax-highlighted diff rendering (pure
//! Rust via syntect), and common widget helpers.

use crate::git::git_capture;
use ratatui::crossterm::event::{
    KeyCode, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashSet;
use std::io::stdout;
use std::process::Command;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// ASCII unit separator — a field delimiter that won't appear in our fields.
pub const SEP: char = '\u{1f}';

/// Key-hint labels shown in pane titles, defined once so every pane reads the
/// same. `Y_MOVE`/`X_MOVE` are plain vertical/horizontal navigation (arrows and
/// hjkl both work everywhere); the `CTRL_` variants are the modifier forms used
/// for the bottom pane — Ctrl-arrows are the terminal-safe way to send them,
/// since some terminals can't send a distinct Ctrl-letter.
pub const Y_MOVE: &str = "↑↓/jk";
pub const CTRL_Y_MOVE: &str = "ctrl-↑↓/jk";
pub const X_MOVE: &str = "←→/hl";
pub const CTRL_X_MOVE: &str = "ctrl-←→/hl";

/// Tint for changed cells (left = removed, right = added).
const REMOVED_BG: Color = Color::Rgb(55, 24, 24);
const ADDED_BG: Color = Color::Rgb(24, 46, 24);

// ── keyboard enhancement (kitty protocol) ───────────────────────────────────

/// Request DISAMBIGUATE_ESCAPE_CODES if the terminal supports the kitty
/// keyboard protocol. Returns true if pushed (caller must call pop).
/// This makes Ctrl+J distinct from Enter in terminals like VS Code.
pub fn push_keyboard_enhancement() -> bool {
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        true
    } else {
        false
    }
}

pub fn pop_keyboard_enhancement() {
    let _ = stdout().execute(PopKeyboardEnhancementFlags);
}

// ── external difftool ────────────────────────────────────────────────────────

/// git's magic empty-tree hash — the "before" side for a root commit that has no
/// parent to diff against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Is a difftool configured? `git difftool` uses `diff.tool`, falling back to
/// `merge.tool`. We check up front so that, if neither is set, we can show a
/// helpful message instead of tearing down the TUI only for git to error out.
fn has_difftool(dir: &str) -> bool {
    ["diff.tool", "merge.tool"]
        .iter()
        .any(|k| git_capture(dir, &["config", k]).is_some_and(|v| !v.is_empty()))
}

/// Open one commit's file in the user's difftool, comparing it against its first
/// parent (or the empty tree for a root commit) — mirroring what `git show`
/// displays. Returns a status line for the caller to surface.
pub fn difftool_commit(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    dir: &str,
    hash: &str,
    path: &str,
) -> String {
    let base = if commit_has_parent(dir, hash) {
        format!("{hash}^")
    } else {
        EMPTY_TREE.to_string()
    };
    run_difftool(terminal, enhanced, dir, &[&base, hash, "--", path])
}

fn commit_has_parent(dir: &str, hash: &str) -> bool {
    git_capture(dir, &["rev-list", "--parents", "-n", "1", hash])
        .map(|s| s.split_whitespace().count() > 1)
        .unwrap_or(false)
}

/// Suspend the TUI, run `git -C <dir> difftool -y <args>` with the terminal
/// handed over (so a terminal tool like vimdiff works), then re-enter. Bails
/// cleanly — no screen flicker — when no difftool is configured. Returns "" on
/// success, else a short message to show the user.
pub fn run_difftool(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    dir: &str,
    args: &[&str],
) -> String {
    if !has_difftool(dir) {
        return "no difftool set — configure one: git config --global diff.tool <tool>".to_string();
    }

    // Hand the terminal back to the external tool.
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("difftool")
        .arg("-y")
        .args(args)
        .status();

    // Re-enter the TUI exactly as run() first set it up.
    *terminal = ratatui::init();
    if enhanced {
        push_keyboard_enhancement();
    }
    let _ = terminal.clear();

    match status {
        Ok(s) if s.success() => String::new(),
        Ok(_) => "difftool exited with an error".to_string(),
        Err(e) => format!("could not launch difftool: {e}"),
    }
}

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

/// Hashes reachable from local branches but on **no** remote — i.e. commits you
/// haven't pushed anywhere. A commit not in this set is on some remote (pushed).
/// Empty when the repo has no remotes (nothing is "pushed").
pub fn load_unpushed() -> HashSet<String> {
    git_capture(".", &["rev-list", "--branches", "--not", "--remotes"])
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

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
/// When `pathspec` is non-empty, only files matching it are returned (so a
/// path-filtered `ilog` shows just that file's change in each commit).
pub fn load_files(hash: &str, pathspec: &[&str]) -> Vec<FileEntry> {
    let mut args = vec!["show", "--name-status", "--format=", hash];
    if !pathspec.is_empty() {
        args.push("--");
        args.extend_from_slice(pathspec);
    }
    git_capture(".", &args)
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

/// One file's raw `git show` diff text (fetched once, then re-rendered locally
/// for scrolling without re-shelling out to git).
pub fn load_diff_raw(hash: &str, path: &str) -> String {
    git_capture(".", &["show", "--format=", hash, "--", path]).unwrap_or_default()
}

/// Longest content line in a raw diff (minus its +/-/space prefix), for
/// clamping horizontal scroll.
pub fn max_line_width(raw: &str) -> usize {
    raw.lines()
        .map(|l| l.chars().count().saturating_sub(1))
        .max()
        .unwrap_or(0)
}

// ── widget helpers ──────────────────────────────────────────────────────────

/// Render a commit row. `unpushed` prepends a yellow `↑` marker (this commit is
/// on no remote yet); pushed commits get an aligning blank so columns line up.
pub fn commit_item(c: &Commit, unpushed: bool) -> ListItem<'static> {
    let mark = if unpushed {
        Span::styled(
            "↑ ",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    ListItem::new(Line::from(vec![
        mark,
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
/// Open/drill-in = Enter only (so l/→ are free for horizontal diff scroll).
pub fn is_open(c: KeyCode) -> bool {
    matches!(c, KeyCode::Enter)
}
/// Back/step-out = Esc (Ctrl-[ sends Esc too), so h/← are free to scroll.
pub fn is_back(c: KeyCode) -> bool {
    matches!(c, KeyCode::Esc)
}
/// Pan left = h or ←.
pub fn is_left(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('h') | KeyCode::Left)
}
/// Pan right = l or →.
pub fn is_right(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('l') | KeyCode::Right)
}

/// Fold Ctrl+[ back into Esc. Terminals send Ctrl+[ as the raw ESC byte, but the
/// kitty protocol we push (DISAMBIGUATE_ESCAPE_CODES) turns it into a distinct
/// Ctrl+[ event — so map it back, since Ctrl+[ is Esc in vim muscle memory. Call
/// it once per key event before matching.
pub fn norm_esc(code: KeyCode, ctrl: bool) -> KeyCode {
    if ctrl && matches!(code, KeyCode::Char('[')) {
        KeyCode::Esc
    } else {
        code
    }
}

pub fn pane_width(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| s.width.saturating_sub(2))
        .unwrap_or(120)
}

/// Inner height of the lower (~60%) pane — the diff viewport, in rows.
pub fn pane_height(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| ((s.height as u32 * 6 / 10).saturating_sub(2).max(1)) as u16)
        .unwrap_or(20)
}

/// Half the height of the lower (~60%) pane, for vim Ctrl-d/Ctrl-u.
pub fn half_page(terminal: &DefaultTerminal) -> u16 {
    (pane_height(terminal) / 2).max(1)
}

/// Clamp a scroll offset so the last line can't scroll above the viewport —
/// stops you from scrolling off the bottom into empty space.
pub fn clamp_scroll(scroll: u16, total_lines: usize, viewport: u16) -> u16 {
    let max = (total_lines.min(u16::MAX as usize) as u16).saturating_sub(viewport);
    scroll.min(max)
}

/// Clamp horizontal scroll so you can't pan past the longest line. `cell_w` is
/// roughly one side's visible width.
pub fn clamp_hscroll(hscroll: u16, max_line: usize, cell_w: u16) -> u16 {
    let max = (max_line.min(u16::MAX as usize) as u16).saturating_sub(cell_w);
    hscroll.min(max)
}

/// Draw a vertical scrollbar down the right edge of `area`, with the thumb at
/// `top` of `total` rows. No bar is drawn when everything already fits.
fn render_vscrollbar(frame: &mut Frame, area: Rect, total: usize, top: usize) {
    let viewport = area.height.saturating_sub(2) as usize;
    if total <= viewport {
        return; // everything fits; no scrollbar needed
    }
    let mut state = ScrollbarState::new(total - viewport).position(top);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None);
    frame.render_stateful_widget(bar, area.inner(Margin::new(0, 1)), &mut state);
}

/// Scrollbar for a diff pane: thumb tracks the scroll line.
pub fn diff_scrollbar(frame: &mut Frame, area: Rect, total_lines: usize, scroll: u16) {
    render_vscrollbar(frame, area, total_lines, scroll as usize);
}

/// Scrollbar for a list pane: thumb tracks the visible window (the list's
/// `offset`). Call it right after rendering the list so the offset is current.
pub fn list_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize) {
    render_vscrollbar(frame, area, total, offset);
}

/// Horizontal scrollbar along a diff pane's bottom border. `max_line` is the
/// widest content line, `cell_w` the visible columns per side, `hscroll` the pan
/// offset. Drawn only when the content is wider than one cell (else there's
/// nothing to pan). Kept off the corners with a 1-col horizontal inset.
pub fn diff_hscrollbar(frame: &mut Frame, area: Rect, max_line: usize, cell_w: u16, hscroll: u16) {
    let cell = cell_w as usize;
    if max_line <= cell {
        return;
    }
    let mut state = ScrollbarState::new(max_line - cell).position(hscroll as usize);
    // `■` renders vertically centered and medium-weight — between the too-thin,
    // low-sitting `▬` and the full-cell block `█`.
    let bar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_symbol("■");
    frame.render_stateful_widget(bar, area.inner(Margin::new(1, 0)), &mut state);
}

// ── side-by-side diff rendering ─────────────────────────────────────────────

/// Two highlighters per file — one for the old side, one for the new — so a
/// removed line's syntax state never corrupts the added side and vice versa.
struct FileHl {
    old: HighlightLines<'static>,
    new: HighlightLines<'static>,
}

/// One accumulated changed line: its line number and highlighted spans.
type NumLine = (usize, Vec<Span<'static>>);

/// One row of a parsed diff: either a full-width header line, or a side-by-side
/// pair whose cells are already syntax-highlighted (so re-laying out for a
/// scroll is cheap — no re-highlighting).
enum DiffRow {
    Header(Line<'static>),
    Pair {
        lnum: Option<usize>,
        lspans: Vec<Span<'static>>,
        lbg: Option<Color>,
        rnum: Option<usize>,
        rspans: Vec<Span<'static>>,
        rbg: Option<Color>,
    },
}

/// A diff parsed and highlighted once. Cheap to re-render at any width/hscroll.
#[derive(Default)]
pub struct RenderedDiff {
    rows: Vec<DiffRow>,
    gutter_w: usize,
    max_line: usize,
}

impl RenderedDiff {
    /// Longest content line, for clamping horizontal scroll.
    pub fn max_line(&self) -> usize {
        self.max_line
    }

    /// Visible content columns per side at total pane `width`. Mirrors the layout
    /// math in `render_prepared` (gutters + " │ " separator eat into each side),
    /// so horizontal-scroll clamping matches what's actually drawn — otherwise the
    /// tail of a medium-length line stays hidden because panning stops short.
    pub fn cell_width(&self, width: u16) -> u16 {
        let avail = (width as usize).saturating_sub(2 * self.gutter_w + 5);
        (avail / 2).max(1) as u16
    }
}

/// Parse `git show` output and syntax-highlight every line **once**. This is
/// the expensive step (syntect); call it when a file is opened, then re-render
/// with `render_prepared` for free on every scroll.
pub fn prepare_diff(raw: &str) -> RenderedDiff {
    let ps = syntaxes();
    let theme = theme();

    let mut rows: Vec<DiffRow> = Vec::new();
    let mut hl: Option<FileHl> = None;
    let mut in_patch = false;
    let mut old_ln = 0usize;
    let mut new_ln = 0usize;
    let mut rem: Vec<NumLine> = Vec::new();
    let mut add: Vec<NumLine> = Vec::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_pairs(&mut rows, &mut rem, &mut add);
            in_patch = true;
            let syntax = syntax_for(ps, rest.rsplit(" b/").next().unwrap_or(""));
            hl = Some(FileHl {
                old: HighlightLines::new(syntax, theme),
                new: HighlightLines::new(syntax, theme),
            });
            rows.push(DiffRow::Header(plain(line, dim().add_modifier(Modifier::BOLD))));
            continue;
        }

        if !in_patch {
            if line.is_empty() {
                continue;
            }
            rows.push(DiffRow::Header(plain(line, Style::default())));
            continue;
        }

        if line.starts_with("@@") {
            flush_pairs(&mut rows, &mut rem, &mut add);
            if let Some((a, _, c, _)) = parse_hunk(line) {
                old_ln = a;
                new_ln = c;
            }
            rows.push(DiffRow::Header(plain(line, Style::default().fg(Color::Cyan))));
            continue;
        }
        if is_meta(line) {
            flush_pairs(&mut rows, &mut rem, &mut add);
            rows.push(DiffRow::Header(plain(line, dim())));
            continue;
        }

        if let Some(code) = line.strip_prefix('-') {
            let spans = hl_old(&mut hl, ps, code);
            rem.push((old_ln, spans));
            old_ln += 1;
        } else if let Some(code) = line.strip_prefix('+') {
            let spans = hl_new(&mut hl, ps, code);
            add.push((new_ln, spans));
            new_ln += 1;
        } else if let Some(code) = line.strip_prefix(' ') {
            flush_pairs(&mut rows, &mut rem, &mut add);
            let l = hl_old(&mut hl, ps, code);
            let r = hl_new(&mut hl, ps, code);
            rows.push(DiffRow::Pair {
                lnum: Some(old_ln),
                lspans: l,
                lbg: None,
                rnum: Some(new_ln),
                rspans: r,
                rbg: None,
            });
            old_ln += 1;
            new_ln += 1;
        } else {
            flush_pairs(&mut rows, &mut rem, &mut add);
            rows.push(DiffRow::Header(plain(line, dim())));
        }
    }
    flush_pairs(&mut rows, &mut rem, &mut add);

    RenderedDiff {
        rows,
        gutter_w: gutter_width(raw),
        max_line: max_line_width(raw),
    }
}

/// Pair up accumulated removed/added lines into side-by-side `Pair` rows.
fn flush_pairs(rows: &mut Vec<DiffRow>, rem: &mut Vec<NumLine>, add: &mut Vec<NumLine>) {
    let n = rem.len().max(add.len());
    for i in 0..n {
        let (lnum, lspans, lbg) = match rem.get(i) {
            Some((num, s)) => (Some(*num), s.clone(), Some(REMOVED_BG)),
            None => (None, Vec::new(), None),
        };
        let (rnum, rspans, rbg) = match add.get(i) {
            Some((num, s)) => (Some(*num), s.clone(), Some(ADDED_BG)),
            None => (None, Vec::new(), None),
        };
        rows.push(DiffRow::Pair {
            lnum,
            lspans,
            lbg,
            rnum,
            rspans,
            rbg,
        });
    }
    rem.clear();
    add.clear();
}

/// Lay out a prepared diff at `width`, panned right by `hscroll` columns. Cheap:
/// just slices the already-highlighted spans into columns — no highlighting.
pub fn render_prepared(d: &RenderedDiff, width: u16, hscroll: u16) -> Text<'static> {
    let g = d.gutter_w;
    let inner = width as usize;
    // layout: [num g][ ][left lw] " │ " [num g][ ][right rw]
    let avail = inner.saturating_sub(2 * g + 5);
    let lw = (avail / 2).max(1);
    let rw = avail.saturating_sub(lw).max(1);
    let hs = hscroll as usize;

    let lines: Vec<Line<'static>> = d
        .rows
        .iter()
        .map(|row| match row {
            DiffRow::Header(l) => l.clone(),
            DiffRow::Pair {
                lnum,
                lspans,
                lbg,
                rnum,
                rspans,
                rbg,
            } => {
                let mut spans = gutter(*lnum, g, *lbg);
                spans.extend(fit_cell(lspans.clone(), lw, *lbg, hs));
                spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
                spans.extend(gutter(*rnum, g, *rbg));
                spans.extend(fit_cell(rspans.clone(), rw, *rbg, hs));
                Line::from(spans)
            }
        })
        .collect();

    Text::from(lines)
}

/// A right-aligned line-number gutter `g` wide plus a trailing space.
fn gutter(num: Option<usize>, g: usize, bg: Option<Color>) -> Vec<Span<'static>> {
    let text = match num {
        Some(n) => format!("{n:>g$} "),
        None => " ".repeat(g + 1),
    };
    let mut style = Style::default().fg(Color::DarkGray);
    if let Some(bg) = bg {
        style = style.bg(bg);
    }
    vec![Span::styled(text, style)]
}

/// Column width for the line-number gutter: digits of the largest line number
/// any hunk reaches, at least 3.
fn gutter_width(raw: &str) -> usize {
    let mut max = 1usize;
    for line in raw.lines() {
        if line.starts_with("@@")
            && let Some((a, b, c, d)) = parse_hunk(line)
        {
            max = max.max(a + b).max(c + d);
        }
    }
    digits(max).max(3)
}

fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// Parse a hunk header `@@ -a,b +c,d @@` into `(a, b, c, d)`. The counts `b`/`d`
/// default to 1 when omitted (`@@ -a +c @@`).
fn parse_hunk(line: &str) -> Option<(usize, usize, usize, usize)> {
    let body = line.strip_prefix("@@ ")?.split(" @@").next()?; // "-a,b +c,d"
    let mut parts = body.split(' ');
    let (a, b) = parse_pair(parts.next()?.strip_prefix('-')?)?;
    let (c, d) = parse_pair(parts.next()?.strip_prefix('+')?)?;
    Some((a, b, c, d))
}

fn parse_pair(s: &str) -> Option<(usize, usize)> {
    let mut it = s.split(',');
    let start = it.next()?.parse().ok()?;
    let count = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    Some((start, count))
}

/// Slide a cell left by `skip` display columns (horizontal scroll), then
/// truncate/pad to exactly `width` columns, applying an optional background to
/// the whole cell (including the padding).
fn fit_cell(
    spans: Vec<Span<'static>>,
    width: usize,
    bg: Option<Color>,
    skip: usize,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut skipped = 0usize; // columns dropped off the left so far
    let mut used = 0usize; // columns emitted into the cell
    for sp in spans {
        if used >= width {
            break;
        }
        let chars: Vec<char> = sp.content.chars().collect();
        let mut start = 0usize;
        if skipped < skip {
            let drop = (skip - skipped).min(chars.len());
            start = drop;
            skipped += drop;
            if start >= chars.len() {
                continue; // whole span scrolled off the left edge
            }
        }
        let take = (chars.len() - start).min(width - used);
        let text: String = chars[start..start + take].iter().collect();
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
