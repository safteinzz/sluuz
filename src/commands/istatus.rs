//! `slu istatus` - interactive `git status` (TUI) for the current repo.
//!
//! Top pane: the changed files. Bottom pane: the selected file's diff
//! (syntax-highlighted, via the shared `tui` renderer). `h`/`l` (or `←`/`→`)
//! slide the scope between **staged**, **all**, and **unstaged**; `s`/`u`/Space
//! stage / unstage / toggle the selected file; Ctrl-↑/↓ (or Ctrl-j/k) and
//! Ctrl-d/u scroll the diff. `j`/`k` move the file list. `q` / `Esc` / `Ctrl-C`
//! quit.
//!
//! This is the interactive counterpart to `slu repos` (which is cross-repo).

use crate::git::{git_capture, git_capture_raw, git_run};
use crate::tui::difftool::run_difftool;
use crate::tui::highlight::{prepare_diff, render_prepared, RenderedDiff};
use crate::tui::input::{
    is_down, is_left, is_right, is_up, norm_esc, CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE,
};
use crate::tui::widgets::{diff_hscrollbar, diff_scrollbar, list_scrollbar, pane_block};
use crate::tui::{
    clamp_hscroll, clamp_scroll, half_page, pane_height, pane_width, pop_keyboard_enhancement,
    push_keyboard_enhancement,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::io::{self, IsTerminal};

/// Rows a Ctrl-j/k moves the diff, columns a Ctrl-h/l pans it, and the PgUp/PgDn
/// jump.
const SCROLL_STEP: u16 = 3;
const PAN_STEP: u16 = 8;
const PAGE_STEP: u16 = 10;

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

/// Everything the view holds. `root` is on it because every git call needs it,
/// which is what used to make the helpers below take four arguments each.
struct App {
    root: String,
    enhanced: bool,
    width: u16,
    entries: Vec<Entry>,
    /// Indices into `entries` that the current scope keeps.
    visible: Vec<usize>,
    sel: usize,
    scope_idx: usize,
    state: ListState,
    /// Transient status line (a difftool result, mostly).
    msg: Option<String>,
    prepared: RenderedDiff,
    diff: Text<'static>,
    diff_scroll: u16,
    diff_hscroll: u16,
}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu istatus needs an interactive terminal - use `git status` instead");
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

    let mut app = App {
        root,
        enhanced: false,
        width: 120,
        entries: Vec::new(),
        visible: Vec::new(),
        sel: 0,
        scope_idx: 1, // default: All
        state: ListState::default(),
        msg: None,
        prepared: RenderedDiff::default(),
        diff: Text::default(),
        diff_scroll: 0,
        diff_hscroll: 0,
    };

    let mut terminal = ratatui::init();
    app.enhanced = push_keyboard_enhancement();
    app.width = pane_width(&terminal);
    app.reload();
    let result = app.event_loop(&mut terminal);
    if app.enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu istatus: {e}");
    }
}

impl App {
    fn scope(&self) -> Scope {
        SCOPES[self.scope_idx]
    }

    /// The entry the cursor is on.
    fn current(&self) -> Option<&Entry> {
        self.visible.get(self.sel).map(|&i| &self.entries[i])
    }

    /// Re-read the working tree, then re-filter and re-diff.
    fn reload(&mut self) {
        self.entries = load_status(&self.root);
        self.rescope();
    }

    /// Re-filter for the current scope, keep the cursor in range, and refresh
    /// the diff pane under it.
    fn rescope(&mut self) {
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.in_scope(self.scope()))
            .map(|(i, _)| i)
            .collect();
        if self.sel >= self.visible.len() {
            self.sel = self.visible.len().saturating_sub(1);
        }
        self.state.select((!self.visible.is_empty()).then_some(self.sel));
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.refresh_diff();
    }

    /// Highlight the selected file's diff. This is the expensive syntect pass,
    /// so it runs on selection changes only, never on a scroll.
    fn refresh_diff(&mut self) {
        match self.current() {
            Some(entry) => {
                let raw = diff_for(&self.root, entry, self.scope());
                self.prepared = prepare_diff(&raw);
                self.diff = render_prepared(&self.prepared, self.width, 0);
            }
            None => {
                self.prepared = RenderedDiff::default();
                self.diff = Text::default();
            }
        }
    }

    /// Run a git command against the selected file, reporting whether the
    /// working tree changed.
    fn on_current(&self, args: &[&str]) -> bool {
        match self.current() {
            Some(e) => {
                let mut argv = args.to_vec();
                argv.extend_from_slice(&["--", &e.path]);
                git_run(&self.root, &argv).0
            }
            None => false,
        }
    }

    /// Space: stage a file that has unstaged changes, else unstage it.
    fn toggle(&self) -> bool {
        match self.current() {
            Some(e) if e.unstaged() => self.on_current(&["add"]),
            Some(_) => self.on_current(&["restore", "--staged"]),
            None => false,
        }
    }

    /// Open the selected file in the user's difftool, matching the comparison
    /// the pane shows. Returns whether the tree may have changed under it.
    fn difftool(&mut self, terminal: &mut DefaultTerminal) -> bool {
        let Some(e) = self.current() else {
            return false;
        };
        let scope = self.scope();
        let cached = scope == Scope::Staged || (scope == Scope::All && !e.unstaged());
        let (path, untracked) = (e.path.clone(), e.untracked());

        if untracked {
            self.msg = Some("untracked - nothing to compare".to_string());
            return false;
        }
        let args: &[&str] = if cached {
            &["--cached", "--"]
        } else {
            &["--"]
        };
        let mut argv = args.to_vec();
        argv.push(&path);
        let dt = run_difftool(terminal, self.enhanced, &self.root, &argv);
        self.width = pane_width(terminal);
        if !dt.is_empty() {
            self.msg = Some(dt);
        }
        true // a difftool edit may have changed the file
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|frame| draw(frame, self))?;

            match event::read()? {
                Event::Resize(_, _) => {
                    let w = pane_width(terminal);
                    if w != self.width {
                        self.width = w;
                        self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    let code = norm_esc(key.code, ctrl);
                    self.msg = None; // any keypress clears a stale status message

                    if matches!(code, KeyCode::Char('q') | KeyCode::Esc)
                        || (ctrl && code == KeyCode::Char('c'))
                    {
                        break;
                    }
                    self.on_key(code, ctrl, terminal);
                    self.diff_scroll =
                        clamp_scroll(self.diff_scroll, self.diff.lines.len(), pane_height(terminal));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn on_key(&mut self, code: KeyCode, ctrl: bool, terminal: &mut DefaultTerminal) {
        let half = half_page(terminal);
        let mut moved = false; // selection or scope changed → re-diff
        let mut reload = false; // working tree changed → re-read status

        if ctrl && is_down(code) {
            self.diff_scroll = self.diff_scroll.saturating_add(SCROLL_STEP);
        } else if ctrl && is_up(code) {
            self.diff_scroll = self.diff_scroll.saturating_sub(SCROLL_STEP);
        } else if ctrl && code == KeyCode::Char('d') {
            self.diff_scroll = self.diff_scroll.saturating_add(half);
        } else if ctrl && code == KeyCode::Char('u') {
            self.diff_scroll = self.diff_scroll.saturating_sub(half);
        } else if ctrl && is_right(code) {
            self.diff_hscroll = clamp_hscroll(
                self.diff_hscroll.saturating_add(PAN_STEP),
                self.prepared.max_line(),
                self.prepared.cell_width(self.width),
            );
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
        } else if ctrl && is_left(code) {
            self.diff_hscroll = self.diff_hscroll.saturating_sub(PAN_STEP);
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
        } else if code == KeyCode::PageDown {
            self.diff_scroll = self.diff_scroll.saturating_add(PAGE_STEP);
        } else if code == KeyCode::PageUp {
            self.diff_scroll = self.diff_scroll.saturating_sub(PAGE_STEP);
        } else if is_down(code) && self.sel + 1 < self.visible.len() {
            self.sel += 1;
            moved = true;
        } else if is_up(code) && self.sel > 0 {
            self.sel -= 1;
            moved = true;
        } else if !ctrl && is_left(code) && self.scope_idx > 0 {
            self.scope_idx -= 1;
            self.sel = 0;
            moved = true;
        } else if !ctrl && is_right(code) && self.scope_idx + 1 < SCOPES.len() {
            self.scope_idx += 1;
            self.sel = 0;
            moved = true;
        } else if code == KeyCode::Char('s') {
            reload = self.on_current(&["add"]);
        } else if code == KeyCode::Char('u') {
            reload = self.on_current(&["restore", "--staged"]);
        } else if code == KeyCode::Char(' ') {
            reload = self.toggle();
        } else if code == KeyCode::Char('r') {
            reload = true;
        } else if code == KeyCode::Enter {
            reload = self.difftool(terminal);
        }

        if reload {
            self.reload();
        } else if moved {
            self.rescope();
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

/// Parse `git status --porcelain -z` into entries. `-z` NUL-separates records
/// (so paths with spaces/newlines are safe) and, for renames/copies, follows the
/// record with an extra NUL-terminated original path, which we skip.
fn load_status(root: &str) -> Vec<Entry> {
    // `git_capture_raw`, not `git_capture`: the porcelain's first column is a
    // SPACE when a file has no staged change, and trimming would eat it on the
    // first record - shifting the status codes and the path by one char.
    let raw = match git_capture_raw(root, &["status", "--porcelain", "-z"]) {
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

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());

    // ── top: file list ──
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| status_item(&app.entries[i]))
        .collect();
    let top_title = if app.visible.is_empty() {
        format!(" {}  clean   {X_MOVE} scope · q quit ", app.scope().label())
    } else {
        format!(
            " {}  {}/{}   {Y_MOVE} · {X_MOVE} scope · s/u/space stage · q quit ",
            app.scope().label(),
            app.sel + 1,
            app.visible.len()
        )
    };
    let list = List::new(items)
        .block(pane_block(top_title, true))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[0], &mut app.state);
    list_scrollbar(frame, areas[0], app.visible.len(), app.state.offset());

    // ── bottom: diff of the selected file ──
    let (path, tag) = match app.current() {
        Some(e) => (e.path.as_str(), diff_tag(e, app.scope())),
        None => ("", ""),
    };
    let title = match &app.msg {
        Some(m) => format!(" {path}  ⚠ {m} "),
        None if path.is_empty() => " (nothing to show) ".to_string(),
        None => format!(" {path} {tag}  enter difftool · {CTRL_Y_MOVE} scroll · {CTRL_X_MOVE} pan "),
    };
    let diff = Paragraph::new(app.diff.clone())
        .block(pane_block(title, true))
        .scroll((app.diff_scroll, 0));
    frame.render_widget(diff, areas[1]);
    diff_scrollbar(frame, areas[1], app.diff.lines.len(), app.diff_scroll);
    let cell = app.prepared.cell_width(areas[1].width.saturating_sub(2));
    diff_hscrollbar(frame, areas[1], app.prepared.max_line(), cell, app.diff_hscroll);
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
