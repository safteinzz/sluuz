//! `slu istatus` — interactive `git status` (TUI) for the current repo.
//!
//! Top pane: the changed files. Bottom pane: the selected file's diff
//! (syntax-highlighted, via the shared `tui` renderer). `h`/`l` (or `←`/`→`)
//! slide the scope between **staged**, **all**, and **unstaged**; `s`/`u`/Space
//! stage / unstage / toggle the selected file; Ctrl-↑/↓ (or Ctrl-j/k) and
//! Ctrl-d/u scroll the diff. `j`/`k` move the file list. `q` / `Esc` / `Ctrl-C`
//! quit.
//!
//! This is the interactive counterpart to `slu repos` (which is cross-repo).

use crate::git::{git_capture, git_run};
use crate::tui::{
    clamp_scroll, diff_scrollbar, half_page, is_down, is_left, is_right, is_up, list_scrollbar,
    norm_esc, pane_block, pane_height, pane_width, pop_keyboard_enhancement, prepare_diff,
    push_keyboard_enhancement, render_prepared, run_difftool, RenderedDiff, CTRL_Y_MOVE, X_MOVE,
    Y_MOVE,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {}

/// Which slice of the working tree the file list is showing.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Staged,
    All,
    Unstaged,
}

/// Left→right order for the `h`/`l` slider; `All` sits in the middle.
const SCOPES: [Scope; 3] = [Scope::Staged, Scope::All, Scope::Unstaged];

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Staged => "staged",
            Scope::All => "all",
            Scope::Unstaged => "unstaged",
        }
    }
}

/// One `git status` entry: index status `x`, worktree status `y`, and the path.
struct Entry {
    x: char,
    y: char,
    path: String,
}

impl Entry {
    /// Has staged (index) changes.
    fn staged(&self) -> bool {
        self.x != ' ' && self.x != '?'
    }
    /// Has unstaged (worktree) changes, including untracked files.
    fn unstaged(&self) -> bool {
        self.x == '?' || (self.y != ' ' && self.y != '?')
    }
    fn untracked(&self) -> bool {
        self.x == '?'
    }
    fn in_scope(&self, s: Scope) -> bool {
        match s {
            Scope::Staged => self.staged(),
            Scope::Unstaged => self.unstaged(),
            Scope::All => self.staged() || self.unstaged(),
        }
    }
}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu istatus needs an interactive terminal — use `git status` instead");
        return;
    }
    // Anchor every git call at the repo root. `git status` reports paths
    // relative to the root, so if we ran diff/add from a subdirectory the
    // pathspecs wouldn't resolve ("Could not access '…'").
    let root = match git_capture(".", &["rev-parse", "--show-toplevel"]) {
        Some(r) if !r.is_empty() => r,
        _ => {
            eprintln!("slu istatus: not inside a git repository");
            return;
        }
    };

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &root, enhanced);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu istatus: {e}");
    }
}

fn event_loop(terminal: &mut DefaultTerminal, root: &str, enhanced: bool) -> io::Result<()> {
    let mut width = pane_width(terminal);
    let mut entries = load_status(root);
    let mut scope_idx = 1usize; // default: All
    let mut sel = 0usize;
    let mut msg: Option<String> = None; // transient status (e.g. difftool result)

    let mut visible = visible_indices(&entries, SCOPES[scope_idx]);
    let mut prepared = RenderedDiff::default();
    let mut diff = Text::default();
    let mut diff_scroll = 0u16;
    refresh_diff(root, &entries, &visible, sel, SCOPES[scope_idx], width, &mut prepared, &mut diff);

    let mut state = ListState::default();
    state.select((!visible.is_empty()).then_some(0));

    loop {
        let scope = SCOPES[scope_idx];
        terminal.draw(|frame| {
            draw(
                frame,
                View {
                    entries: &entries,
                    visible: &visible,
                    sel,
                    scope,
                    diff: &diff,
                    diff_scroll,
                    msg: msg.as_deref(),
                },
                &mut state,
            )
        })?;

        match event::read()? {
            Event::Resize(_, _) => {
                let w = pane_width(terminal);
                if w != width {
                    width = w;
                    diff = render_prepared(&prepared, width, 0);
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let code = norm_esc(key.code, ctrl);
                msg = None; // any keypress clears a stale status message

                if matches!(code, KeyCode::Char('q') | KeyCode::Esc)
                    || (ctrl && code == KeyCode::Char('c'))
                {
                    break;
                }

                let half = half_page(terminal);
                let mut dirty = false; // selection/scope changed → reload diff
                let mut reload = false; // working tree changed → reload status

                if ctrl && is_down(code) {
                    diff_scroll = diff_scroll.saturating_add(3);
                } else if ctrl && is_up(code) {
                    diff_scroll = diff_scroll.saturating_sub(3);
                } else if ctrl && code == KeyCode::Char('d') {
                    diff_scroll = diff_scroll.saturating_add(half);
                } else if ctrl && code == KeyCode::Char('u') {
                    diff_scroll = diff_scroll.saturating_sub(half);
                } else if code == KeyCode::PageDown {
                    diff_scroll = diff_scroll.saturating_add(10);
                } else if code == KeyCode::PageUp {
                    diff_scroll = diff_scroll.saturating_sub(10);
                } else if is_down(code) && sel + 1 < visible.len() {
                    sel += 1;
                    dirty = true;
                } else if is_up(code) && sel > 0 {
                    sel -= 1;
                    dirty = true;
                } else if is_left(code) && scope_idx > 0 {
                    scope_idx -= 1;
                    sel = 0;
                    dirty = true;
                } else if is_right(code) && scope_idx + 1 < SCOPES.len() {
                    scope_idx += 1;
                    sel = 0;
                    dirty = true;
                } else if code == KeyCode::Char('s') {
                    reload = stage_selected(root, &entries, &visible, sel);
                } else if code == KeyCode::Char('u') {
                    reload = unstage_selected(root, &entries, &visible, sel);
                } else if code == KeyCode::Char(' ') {
                    reload = toggle_selected(root, &entries, &visible, sel);
                } else if code == KeyCode::Char('r') {
                    reload = true;
                } else if code == KeyCode::Enter {
                    // Open the selected file in the user's difftool, matching the
                    // comparison the pane shows.
                    let target = visible.get(sel).map(|&i| {
                        let e = &entries[i];
                        let cached = matches!(scope, Scope::Staged)
                            || (matches!(scope, Scope::All) && !e.unstaged());
                        (e.path.clone(), e.untracked(), cached)
                    });
                    if let Some((path, untracked, cached)) = target {
                        if untracked {
                            msg = Some("untracked — nothing to compare".to_string());
                        } else {
                            let dt = if cached {
                                run_difftool(terminal, enhanced, root, &["--cached", "--", &path])
                            } else {
                                run_difftool(terminal, enhanced, root, &["--", &path])
                            };
                            width = pane_width(terminal);
                            if !dt.is_empty() {
                                msg = Some(dt);
                            }
                            reload = true; // a difftool edit may have changed the file
                        }
                    }
                }

                if reload {
                    entries = load_status(root);
                    dirty = true;
                }
                if dirty {
                    visible = visible_indices(&entries, SCOPES[scope_idx]);
                    if sel >= visible.len() {
                        sel = visible.len().saturating_sub(1);
                    }
                    state.select((!visible.is_empty()).then_some(sel));
                    diff_scroll = 0;
                    refresh_diff(
                        root,
                        &entries,
                        &visible,
                        sel,
                        SCOPES[scope_idx],
                        width,
                        &mut prepared,
                        &mut diff,
                    );
                }
                diff_scroll = clamp_scroll(diff_scroll, diff.lines.len(), pane_height(terminal));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Indices of `entries` that belong in `scope`, preserving order.
fn visible_indices(entries: &[Entry], scope: Scope) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.in_scope(scope))
        .map(|(i, _)| i)
        .collect()
}

/// Recompute the diff for the currently selected file (expensive syntect pass).
#[allow(clippy::too_many_arguments)]
fn refresh_diff(
    root: &str,
    entries: &[Entry],
    visible: &[usize],
    sel: usize,
    scope: Scope,
    width: u16,
    prepared: &mut RenderedDiff,
    diff: &mut Text<'static>,
) {
    match visible.get(sel).map(|&i| &entries[i]) {
        Some(entry) => {
            let raw = diff_for(root, entry, scope);
            *prepared = prepare_diff(&raw);
            *diff = render_prepared(prepared, width, 0);
        }
        None => {
            *prepared = RenderedDiff::default();
            *diff = Text::default();
        }
    }
}

/// The raw diff for one entry. Staged scope shows the index-vs-HEAD diff;
/// unstaged shows worktree-vs-index; `All` prefers the worktree diff when the
/// file has unstaged changes, else the staged one. Untracked files are shown as
/// an all-added diff against the null device.
fn diff_for(root: &str, entry: &Entry, scope: Scope) -> String {
    if entry.untracked() {
        let nul = if cfg!(windows) { "NUL" } else { "/dev/null" };
        // `--no-index` exits non-zero when files differ, so read it via git_run.
        let (_, out) = git_run(root, &["diff", "--no-index", "--", nul, &entry.path]);
        return out;
    }
    let cached = match scope {
        Scope::Staged => true,
        Scope::Unstaged => false,
        Scope::All => !entry.unstaged(),
    };
    let args: &[&str] = if cached {
        &["diff", "--cached", "--", &entry.path]
    } else {
        &["diff", "--", &entry.path]
    };
    git_capture(root, args).unwrap_or_default()
}

fn stage_selected(root: &str, entries: &[Entry], visible: &[usize], sel: usize) -> bool {
    if let Some(e) = visible.get(sel).map(|&i| &entries[i]) {
        git_run(root, &["add", "--", &e.path]).0
    } else {
        false
    }
}

fn unstage_selected(root: &str, entries: &[Entry], visible: &[usize], sel: usize) -> bool {
    if let Some(e) = visible.get(sel).map(|&i| &entries[i]) {
        git_run(root, &["restore", "--staged", "--", &e.path]).0
    } else {
        false
    }
}

/// Space: stage a file that has unstaged changes, else unstage it.
fn toggle_selected(root: &str, entries: &[Entry], visible: &[usize], sel: usize) -> bool {
    match visible.get(sel).map(|&i| &entries[i]) {
        Some(e) if e.unstaged() => git_run(root, &["add", "--", &e.path]).0,
        Some(e) => git_run(root, &["restore", "--staged", "--", &e.path]).0,
        None => false,
    }
}

/// Parse `git status --porcelain -z` into entries. `-z` NUL-separates records
/// (so paths with spaces/newlines are safe) and, for renames/copies, follows the
/// record with an extra NUL-terminated original path, which we skip.
fn load_status(root: &str) -> Vec<Entry> {
    let raw = match git_capture(root, &["status", "--porcelain", "-z"]) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());
    let mut entries = Vec::new();
    while let Some(tok) = tokens.next() {
        let bytes = tok.as_bytes();
        if bytes.len() < 3 {
            continue;
        }
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == 'R' || x == 'C' {
            tokens.next(); // consume the original path of a rename/copy
        }
        entries.push(Entry {
            x,
            y,
            path: tok[3..].to_string(),
        });
    }
    entries
}

struct View<'a> {
    entries: &'a [Entry],
    visible: &'a [usize],
    sel: usize,
    scope: Scope,
    diff: &'a Text<'static>,
    diff_scroll: u16,
    msg: Option<&'a str>,
}

fn draw(frame: &mut ratatui::Frame, v: View, state: &mut ListState) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());

    // ── top: file list ──
    let items: Vec<ListItem> = v.visible.iter().map(|&i| status_item(&v.entries[i])).collect();
    let top_title = if v.visible.is_empty() {
        format!(" {}  clean   {X_MOVE} scope · q quit ", v.scope.label())
    } else {
        format!(
            " {}  {}/{}   {Y_MOVE} · {X_MOVE} scope · s/u/space stage · q quit ",
            v.scope.label(),
            v.sel + 1,
            v.visible.len()
        )
    };
    let list = List::new(items)
        .block(pane_block(top_title, true))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[0], state);
    list_scrollbar(frame, areas[0], v.visible.len(), state.offset());

    // ── bottom: diff of the selected file ──
    let (path, tag) = match v.visible.get(v.sel).map(|&i| &v.entries[i]) {
        Some(e) => (e.path.as_str(), diff_tag(e, v.scope)),
        None => ("", ""),
    };
    let title = match v.msg {
        Some(m) => format!(" {path}  ⚠ {m} "),
        None if path.is_empty() => " (nothing to show) ".to_string(),
        None => format!(" {path} {tag}  enter difftool · {CTRL_Y_MOVE} scroll "),
    };
    let diff = Paragraph::new(v.diff.clone())
        .block(pane_block(title, true))
        .scroll((v.diff_scroll, 0));
    frame.render_widget(diff, areas[1]);
    diff_scrollbar(frame, areas[1], v.diff.lines.len(), v.diff_scroll);
}

/// Which side of the diff the bottom pane is showing.
fn diff_tag(e: &Entry, scope: Scope) -> &'static str {
    if e.untracked() {
        "[untracked]"
    } else if scope == Scope::Staged || (scope == Scope::All && !e.unstaged()) {
        "[staged]"
    } else {
        "[worktree]"
    }
}

/// `git status`-style two-column code (staged left, unstaged right) + path.
fn status_item(e: &Entry) -> ListItem<'static> {
    let staged = Style::default().fg(Color::Green);
    let unstaged = Style::default().fg(Color::Red);
    let none = Style::default().fg(Color::DarkGray);

    let (xc, xs) = if e.staged() { (e.x, staged) } else { (' ', none) };
    let (yc, ys) = if e.untracked() {
        ('?', unstaged)
    } else if e.y != ' ' {
        (e.y, unstaged)
    } else {
        (' ', none)
    };

    ListItem::new(Line::from(vec![
        Span::styled(xc.to_string(), xs),
        Span::styled(yc.to_string(), ys),
        Span::raw("  "),
        Span::raw(e.path.clone()),
    ]))
}
