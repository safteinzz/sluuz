//! `slu istatus` - interactive `git status` (TUI) for the current repo.
//!
//! Top pane: the changed files. Bottom pane: the selected file's diff
//! (syntax-highlighted, via the shared `tui` renderer). `h`/`l` (or `←`/`→`)
//! move between the **all**, **staged** and **unstaged** tabs; `s`/`u`/Space
//! stage / unstage / toggle the selected file; Ctrl-↑/↓ (or Ctrl-j/k) scroll
//! the diff. `j`/`k` move the file list. `r` reads the working
//! tree again, for when a build or another terminal has touched it since this
//! one opened. `q` / `Esc` / `Ctrl-C` quit.
//!
//! This is the interactive counterpart to `slu repos` (which is cross-repo).

use crate::git::load::load_blob;
use crate::git::{git_capture, git_capture_raw, git_run};
use crate::tui::FRAME;
use crate::tui::difffeed::DiffFeed;
use crate::tui::difftool::{DiffTool, run_difftool};
use crate::tui::highlight::{Blob, DiffContext, RenderedDiff, render_prepared};
use crate::tui::input::{
    Accel, CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE, is_down, is_left, is_right, is_up, norm_esc,
    stepped,
};
use crate::tui::widgets::{
    Command, CommandLine, Modal, NOTE, Typed, diff_hscrollbar, diff_scrollbar, key_footer,
    list_scrollbar, pane_block, scope_tabs,
};
use crate::tui::{
    clamp_hscroll, clamp_scroll, pane_width, pop_keyboard_enhancement, push_keyboard_enhancement,
};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::time::{Duration, Instant};

/// Rows a Ctrl-j/k moves the diff and columns a Ctrl-h/l pans it, before a held
/// key multiplies them.
const SCROLL_STEP: u16 = 3;
const PAN_STEP: u16 = 8;

#[derive(clap::Args)]
pub struct Args {}

/// Which slice of the working tree the file list is showing.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Staged,
    All,
    Unstaged,
}

/// Tab order: the default first, as in every view.
const SCOPES: [Scope; 3] = [Scope::All, Scope::Staged, Scope::Unstaged];

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
    /// How many files a collapsed untracked directory holds, when it held too
    /// many to list.
    hidden: usize,
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
    /// A difftool result, in the footer in place of the keys until `NOTE` is up.
    note: Option<(bool, String)>,
    command: Option<CommandLine>,
    note_at: Instant,
    /// Rows the diff pane showed on the last frame, which a scroll is clamped to.
    diff_rows: u16,
    /// A message that has to be read before anything else happens: it owns
    /// every key until it is dismissed.
    modal: Option<Modal>,
    accel: Accel,
    prepared: RenderedDiff,
    diff: Text<'static>,
    diff_scroll: u16,
    diff_hscroll: u16,
    dfeed: DiffFeed,
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
        scope_idx: 0,
        state: ListState::default(),
        note: None,
        command: None,
        note_at: Instant::now(),
        diff_rows: 1,
        modal: None,
        accel: Accel::default(),
        prepared: RenderedDiff::default(),
        diff: Text::default(),
        diff_scroll: 0,
        diff_hscroll: 0,
        dfeed: DiffFeed::default(),
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

    /// Re-read the working tree, then re-filter and re-diff. The cursor is kept
    /// on the file it was on by path, not by index: staging a file can drop it
    /// out of the scope above the cursor, and a reload is exactly when the rows
    /// underneath shift.
    fn reload(&mut self) {
        let keep = self.current().map(|e| e.path.clone());
        self.entries = load_status(&self.root);
        self.rescope();
        if let Some(path) = keep
            && let Some(i) = self
                .visible
                .iter()
                .position(|&i| self.entries[i].path == path)
            && i != self.sel
        {
            self.sel = i;
            self.state.select(Some(self.sel));
            self.diff_scroll = 0;
            self.diff_hscroll = 0;
            self.refresh_diff();
        }
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
        self.state
            .select((!self.visible.is_empty()).then_some(self.sel));
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.refresh_diff();
    }

    /// Ask for the selected file's diff. The read and the syntect pass happen
    /// on a worker, so moving the cursor never waits for either; the pane is
    /// cleared now and filled when the answer for this row arrives.
    fn refresh_diff(&mut self) {
        self.prepared = RenderedDiff::default();
        self.diff = Text::default();
        let Some(entry) = self.current() else {
            self.dfeed.idle();
            return;
        };
        let (root, scope) = (self.root.clone(), self.scope());
        let entry = Entry {
            x: entry.x,
            y: entry.y,
            path: entry.path.clone(),
            hidden: entry.hidden,
        };
        self.dfeed.request(move || {
            (
                diff_for(&root, &entry, scope),
                blobs_for(&root, &entry, scope),
            )
        });
    }

    /// Take a diff that has arrived, unless the cursor has moved on since.
    fn drain_diff(&mut self) {
        if let Some(prepared) = self.dfeed.take() {
            self.prepared = prepared;
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
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
            self.set_failed("untracked - nothing to compare");
            return false;
        }
        let args: &[&str] = if cached { &["--cached", "--"] } else { &["--"] };
        let mut argv = args.to_vec();
        argv.push(&path);
        let outcome = run_difftool(terminal, self.enhanced, &self.root, &argv);
        self.width = pane_width(terminal);
        match outcome {
            DiffTool::Quiet => {}
            DiffTool::Note(m) => self.set_failed(m),
            DiffTool::Failed(modal) => self.modal = Some(modal),
        }
        true // a difftool edit may have changed the file
    }

    /// Something did not work, with nothing more to read than one line.
    fn set_failed(&mut self, text: impl Into<String>) {
        self.note = Some((false, text.into()));
        self.note_at = Instant::now();
    }

    fn note_left(&self) -> Option<Duration> {
        self.note
            .as_ref()
            .map(|_| NOTE.saturating_sub(self.note_at.elapsed()))
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            self.drain_diff();
            terminal.draw(|frame| draw(frame, self))?;

            // While a diff is still being prepared, come back on a frame timer
            // to show it, and while a note is up, when it is due to go;
            // otherwise block on the key and leave the CPU alone.
            let wait = match (self.dfeed.loading(), self.note_left()) {
                (true, Some(left)) => Some(left.min(FRAME)),
                (true, None) => Some(FRAME),
                (false, left) => left,
            };
            if let Some(wait) = wait
                && !event::poll(wait)?
            {
                if self.note_left() == Some(Duration::ZERO) {
                    self.note = None;
                }
                continue;
            }
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

                    // Ctrl-C quits from anywhere, even out from under a modal.
                    if ctrl && code == KeyCode::Char('c') {
                        break;
                    }
                    // A modal owns every key until it is dismissed.
                    if let Some(modal) = &mut self.modal {
                        if modal.on_key(code) {
                            self.modal = None;
                        }
                        continue;
                    }

                    self.note = None;
                    // An open `:` line owns every key until it is run or dropped.
                    if let Some(cmd) = &mut self.command {
                        match cmd.on_key(code) {
                            Typed::Open => {}
                            Typed::Cancel => self.command = None,
                            Typed::Run(run) => {
                                self.command = None;
                                match run {
                                    Command::Help => {
                                        self.modal = Some(Modal::reader("keys · status", &help()))
                                    }
                                    Command::Quit => break,
                                    Command::Unknown(what) => self.set_failed(format!(
                                        "unknown command `{what}` · :help lists the keys"
                                    )),
                                }
                            }
                        }
                        continue;
                    }
                    if code == KeyCode::Char(':') {
                        self.command = Some(CommandLine::default());
                        continue;
                    }
                    if matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
                        break;
                    }
                    self.on_key(code, ctrl, terminal);
                    self.diff_scroll =
                        clamp_scroll(self.diff_scroll, self.diff.lines.len(), self.diff_rows);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn on_key(&mut self, code: KeyCode, ctrl: bool, terminal: &mut DefaultTerminal) {
        let steps = self.accel.steps(code, ctrl);
        let mut moved = false; // selection or scope changed → re-diff
        let mut reload = false; // working tree changed → re-read status

        if ctrl && is_down(code) {
            let by = SCROLL_STEP.saturating_mul(steps as u16);
            self.diff_scroll = self.diff_scroll.saturating_add(by);
        } else if ctrl && is_up(code) {
            let by = SCROLL_STEP.saturating_mul(steps as u16);
            self.diff_scroll = self.diff_scroll.saturating_sub(by);
        } else if ctrl && is_right(code) {
            self.diff_hscroll = clamp_hscroll(
                self.diff_hscroll
                    .saturating_add(PAN_STEP.saturating_mul(steps as u16)),
                self.prepared.max_line(),
                self.prepared.cell_width(self.width),
            );
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
        } else if ctrl && is_left(code) {
            self.diff_hscroll = self
                .diff_hscroll
                .saturating_sub(PAN_STEP.saturating_mul(steps as u16));
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
        } else if (is_down(code) || is_up(code))
            && stepped(self.sel, self.visible.len(), is_down(code), steps) != self.sel
        {
            self.sel = stepped(self.sel, self.visible.len(), is_down(code), steps);
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
    let args: &[&str] = if cached(entry, scope) {
        &["diff", "--cached", "--", &entry.path]
    } else {
        &["diff", "--", &entry.path]
    };
    git_capture(root, args).unwrap_or_default()
}

/// Which pair `diff_for` compares: the index against HEAD, or the working tree
/// against the index.
fn cached(entry: &Entry, scope: Scope) -> bool {
    match scope {
        Scope::Staged => true,
        Scope::Unstaged => false,
        Scope::All => !entry.unstaged(),
    }
}

/// The two sides of that same diff as whole files, which is what lets a hunk
/// starting mid-file be highlighted with the state of the lines above it. The
/// working tree is read from disk because git has no revision name for it.
fn blobs_for(root: &str, entry: &Entry, scope: Scope) -> DiffContext {
    let worktree = || -> Blob {
        let path = Path::new(root).join(&entry.path);
        Box::new(move || fs::read_to_string(path).ok())
    };
    // an empty revision names the index copy: `git show :<path>`
    let rev = |rev: &str| -> Blob {
        let (root, rev, path) = (root.to_string(), rev.to_string(), entry.path.clone());
        Box::new(move || load_blob(&root, &rev, &path))
    };
    if entry.untracked() {
        return DiffContext {
            old: None,
            new: Some(worktree()),
        };
    }
    if cached(entry, scope) {
        DiffContext {
            old: Some(rev("HEAD")),
            new: Some(rev("")),
        }
    } else {
        DiffContext {
            old: Some(rev("")),
            new: Some(worktree()),
        }
    }
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
            hidden: 0,
        });
    }
    expand_untracked_dirs(root, entries)
}

/// How many files an untracked directory may contribute before it stays one
/// row. A directory listed as `development/` cannot be read or staged
/// selectively, which is most of what this view is for; a `node_modules/`
/// listed file by file is worse.
const UNTRACKED_CAP: usize = 25;

/// Replace each untracked directory with the files inside it, unless there are
/// more than `UNTRACKED_CAP` of them. git collapses a wholly untracked
/// directory into one row (`?? development/`) unless it is asked for `-uall`,
/// and that row says nothing about what is in it.
fn expand_untracked_dirs(root: &str, entries: Vec<Entry>) -> Vec<Entry> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        if !(entry.untracked() && entry.path.ends_with('/')) {
            out.push(entry);
            continue;
        }
        let inside = untracked_inside(root, &entry.path);
        match inside.len() {
            0 => out.push(entry),
            n if n > UNTRACKED_CAP => out.push(Entry { hidden: n, ..entry }),
            _ => out.extend(inside),
        }
    }
    out
}

/// The untracked files under one directory, as their own entries.
fn untracked_inside(root: &str, dir: &str) -> Vec<Entry> {
    let raw = match git_capture_raw(root, &["status", "--porcelain", "-z", "-uall", "--", dir]) {
        Some(r) => r,
        None => return Vec::new(),
    };
    raw.split('\0')
        .filter(|t| !t.is_empty() && t.len() > 3)
        .map(|t| Entry {
            x: t.as_bytes()[0] as char,
            y: t.as_bytes()[1] as char,
            path: t[3..].to_string(),
            hidden: 0,
        })
        .collect()
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let [panes, footer_row] =
        Layout::vertical([Constraint::Min(2), Constraint::Length(1)]).areas(frame.area());
    let areas =
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).split(panes);
    app.diff_rows = areas[1].height.saturating_sub(2).max(1);

    // ── top: file list ──
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| status_item(&app.entries[i]))
        .collect();
    let tail = if app.visible.is_empty() {
        " clean ".to_string()
    } else {
        format!(" {}/{} ", app.sel + 1, app.visible.len())
    };
    let labels: Vec<&str> = SCOPES.iter().map(|s| s.label()).collect();
    let mut spans = scope_tabs(&labels, app.scope_idx);
    spans.push(Span::raw(tail));
    let top_title = Line::from(spans);
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
    let title = if path.is_empty() {
        " (nothing to show) ".to_string()
    } else {
        format!(" {path} {tag} ")
    };
    // An empty diff and one still being prepared look the same, so say which.
    let body = if app.diff.lines.is_empty() && app.dfeed.slow() {
        Text::from(Line::from(Span::styled(
            "  loading…",
            Style::default().add_modifier(Modifier::DIM),
        )))
    } else {
        app.diff.clone()
    };
    let diff = Paragraph::new(body)
        .block(pane_block(title, true))
        .scroll((app.diff_scroll, 0));
    frame.render_widget(diff, areas[1]);
    diff_scrollbar(frame, areas[1], app.diff.lines.len(), app.diff_scroll);
    let cell = app.prepared.cell_width(areas[1].width.saturating_sub(2));
    diff_hscrollbar(
        frame,
        areas[1],
        app.prepared.max_line(),
        cell,
        app.diff_hscroll,
    );

    let actions = [
        "s stage".to_string(),
        "u unstage".to_string(),
        "space flip".to_string(),
        "enter difftool".to_string(),
        "r refresh".to_string(),
    ];
    frame.render_widget(
        key_footer(
            &actions,
            app.note.as_ref(),
            app.command.as_ref(),
            true,
            footer_row.width,
        ),
        footer_row,
    );

    if let Some(modal) = &mut app.modal {
        modal.draw(frame);
    }
}

/// Every key the view answers to, for the box `:help` opens.
fn help() -> Vec<(String, String)> {
    let row = |k: &str, d: &str| (k.to_string(), d.to_string());
    vec![
        row(Y_MOVE, "move (hold to speed up)"),
        row(X_MOVE, "switch tab"),
        row("s", "stage the file"),
        row("u", "unstage it"),
        row("space", "flip it between staged and not"),
        row(CTRL_Y_MOVE, "scroll the diff"),
        row(CTRL_X_MOVE, "pan it sideways"),
        row("enter", "open the file in your git difftool"),
        row("r", "read it again from git"),
        row("q", "quit"),
    ]
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

    let (xc, xs) = if e.staged() {
        (e.x, staged)
    } else {
        (' ', none)
    };
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
        Span::styled(
            match e.hidden {
                0 => String::new(),
                n => format!("  {n} files"),
            },
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}
