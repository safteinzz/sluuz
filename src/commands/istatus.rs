//! `slu istatus` - interactive `git status` (TUI) for the current repo.
//!
//! Top pane: the changed files. Bottom pane: the selected file's diff
//! (syntax-highlighted, via the shared `tui` renderer). `h`/`l` (or `←`/`→`)
//! move between the **all**, **staged** and **unstaged** tabs; `s`/`u`/Space
//! stage / unstage / toggle the selected file, `S`/`U` every file listed, `d`
//! puts it back as HEAD has it behind a typed gate;
//! Ctrl-↑/↓ (or Ctrl-j/k) scroll the diff. `j`/`k` move the file list and `/` filters it by path. `r` reads
//! the working tree again, for when a build or another terminal has touched it
//! since this one opened. `q` / `Esc` / `Ctrl-C` quit.
//!
//! This is the interactive counterpart to `slu repos` (which is cross-repo).

use crate::git::load::load_blob;
use crate::git::{GIT_WORDS, git_capture, git_capture_raw, git_run};
use crate::tui::FRAME;
use crate::tui::difffeed::DiffFeed;
use crate::tui::difftool::{DiffTool, run_difftool};
use crate::tui::highlight::{Blob, DiffContext, RenderedDiff, render_prepared};
use crate::tui::input::{
    Accel, CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE, is_down, is_left, is_right, is_up, norm_esc,
    stepped,
};
use crate::tui::widgets::{
    Command, CommandLine, Modal, NOTE, Query, Typed, diff_hscrollbar, diff_scrollbar, key_footer,
    list_scrollbar, pane_block, scope_tabs, typed_popup,
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
#[derive(Clone)]
struct Entry {
    x: char,
    y: char,
    path: String,
    /// Where a rename or copy came from, on whichever side `x` or `y` says.
    orig: Option<String>,
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
    /// The flag that makes `git diff` pair a move, and the path it moved from,
    /// when the index (`cached`) or the working tree holds one.
    fn moved_from(&self, cached: bool) -> Option<(&'static str, &str)> {
        let flag = match if cached { self.x } else { self.y } {
            'R' => "-M",
            // git only looks at unmodified files as a copy's source when asked
            'C' => "--find-copies-harder",
            _ => return None,
        };
        self.orig.as_deref().map(|orig| (flag, orig))
    }
    /// What `git add` takes to stage this row: a rename in the working tree
    /// also needs its old path, or the deletion is left behind unstaged.
    fn to_stage(&self) -> Vec<&str> {
        self.with_orig(self.y == 'R')
    }
    /// What `git restore --staged` takes to unstage this row: a staged rename
    /// also needs its old path, or the deletion stays staged. A copy does not,
    /// since its source is a file of its own.
    fn to_unstage(&self) -> Vec<&str> {
        self.with_orig(self.x == 'R')
    }
    fn with_orig(&self, moved: bool) -> Vec<&str> {
        match self.orig.as_deref() {
            Some(orig) if moved => vec![orig, &self.path],
            _ => vec![&self.path],
        }
    }
    /// The file or directory name alone, which is what the gate in front of
    /// `d` wants typed: a whole path is too long to ask for.
    fn base_name(&self) -> &str {
        let path = self.path.trim_end_matches('/');
        path.rsplit('/').next().unwrap_or(path)
    }
    /// The row's name, a move written as `git diff --stat` writes it with
    /// everything but the new part grayed.
    fn label(&self) -> Vec<Span<'static>> {
        match &self.orig {
            Some(orig) => moved_parts(orig, &self.path)
                .into_iter()
                .map(|(text, part)| {
                    let style = match part {
                        Part::Shared | Part::New => Style::default(),
                        Part::Mark | Part::Old => Style::default().fg(Color::DarkGray),
                    };
                    Span::styled(text.to_string(), style)
                })
                .collect(),
            None => vec![Span::raw(self.path.clone())],
        }
    }
}

/// What a piece of a move's name is: shared by both paths, the brace and arrow
/// notation, or the part only the old or only the new path has.
enum Part {
    Shared,
    Mark,
    Old,
    New,
}

/// `from` and `to` as one name with the parts they share written once, the way
/// git's `--stat` does: `templates/{ => components}/table.html`.
fn moved_parts<'a>(from: &'a str, to: &'a str) -> Vec<(&'a str, Part)> {
    let (a, b) = (from.as_bytes(), to.as_bytes());
    let common = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let pfx = a[..common]
        .iter()
        .rposition(|&c| c == b'/')
        .map_or(0, |i| i + 1);
    // the suffix may start on the slash that ends the prefix, as git's does
    let sfx = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(a.len().min(b.len()) - pfx.saturating_sub(1))
        .take_while(|(x, y)| x == y)
        .enumerate()
        .filter(|(_, (c, _))| **c == b'/')
        .last()
        .map_or(0, |(i, _)| i + 1);
    if pfx == 0 && sfx == 0 {
        return vec![(from, Part::Old), (" => ", Part::Mark), (to, Part::New)];
    }
    let middle = |s: &'a str| &s[pfx..(s.len() - sfx).max(pfx)];
    vec![
        (&from[..pfx], Part::Shared),
        ("{", Part::Mark),
        (middle(from), Part::Old),
        (" => ", Part::Mark),
        (middle(to), Part::New),
        ("}", Part::Mark),
        (&from[from.len() - sfx..], Part::Shared),
    ]
}

/// Everything the view holds. `root` is on it because every git call needs it,
/// which is what used to make the helpers below take four arguments each.
struct App {
    root: String,
    enhanced: bool,
    width: u16,
    entries: Vec<Entry>,
    /// Indices into `entries` that the current scope and `query` keep.
    visible: Vec<usize>,
    sel: usize,
    scope_idx: usize,
    state: ListState,
    /// What `/` narrowed the file list to, matched against each path.
    query: Query,
    /// The filter is being typed into, so it owns every key until Enter keeps
    /// it or Esc clears it.
    editing: bool,
    /// A difftool result, in the footer in place of the keys until `NOTE` is up.
    note: Option<(bool, String)>,
    command: Option<CommandLine>,
    note_at: Instant,
    /// Rows the diff pane showed on the last frame, which a scroll is clamped to.
    diff_rows: u16,
    /// A message that has to be read before anything else happens: it owns
    /// every key until it is dismissed.
    modal: Option<Modal>,
    /// The typed gate `d` opens, owning every key until it is answered.
    discard: Option<Discard>,
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
        query: Query::default(),
        editing: false,
        note: None,
        command: None,
        note_at: Instant::now(),
        diff_rows: 1,
        modal: None,
        discard: None,
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

    /// Re-filter for the current scope and query, keep the cursor in range, and
    /// refresh the diff pane under it.
    fn rescope(&mut self) {
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.in_scope(self.scope())
                    && (self.query.keeps(&e.path)
                        || e.orig.as_deref().is_some_and(|o| self.query.keeps(o)))
            })
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
        let (root, scope, entry) = (self.root.clone(), self.scope(), entry.clone());
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

    /// Run a git command against the paths `paths` gives for the selected
    /// file, reporting whether the working tree changed.
    fn on_current(&self, args: &[&str], paths: fn(&Entry) -> Vec<&str>) -> bool {
        match self.current() {
            Some(e) => {
                let mut argv = args.to_vec();
                argv.push("--");
                argv.extend(paths(e));
                git_run(&self.root, &argv).0
            }
            None => false,
        }
    }

    /// Run a git command against the paths `paths` gives for every file the
    /// list shows that `wanted` picks, reporting whether the working tree
    /// changed. A path git has nothing to do with fails the whole command,
    /// which is why `wanted` narrows it first.
    fn on_visible(
        &self,
        args: &[&str],
        wanted: fn(&Entry) -> bool,
        paths: fn(&Entry) -> Vec<&str>,
    ) -> bool {
        let paths: Vec<&str> = self
            .visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|e| wanted(e))
            .flat_map(paths)
            .collect();
        if paths.is_empty() {
            return false;
        }
        let mut argv = args.to_vec();
        argv.push("--");
        argv.extend(paths);
        git_run(&self.root, &argv).0
    }

    /// Space: stage a file that has unstaged changes, else unstage it.
    fn toggle(&self) -> bool {
        match self.current() {
            Some(e) if e.unstaged() => self.on_current(&["add"], Entry::to_stage),
            Some(_) => self.on_current(&["restore", "--staged"], Entry::to_unstage),
            None => false,
        }
    }

    /// Open the selected file in the user's difftool, matching the comparison
    /// the pane shows. Returns whether the tree may have changed under it.
    fn difftool(&mut self, terminal: &mut DefaultTerminal) -> bool {
        let Some(e) = self.current().cloned() else {
            return false;
        };
        if e.untracked() {
            self.set_failed("untracked - nothing to compare");
            return false;
        }
        let argv = diff_args(&e, cached(&e, self.scope()));
        let outcome = run_difftool(terminal, self.enhanced, &self.root, &argv);
        self.width = pane_width(terminal);
        match outcome {
            DiffTool::Quiet => {}
            DiffTool::Note(m) => self.set_failed(m),
            DiffTool::Failed(modal) => self.modal = Some(modal),
        }
        true // a difftool edit may have changed the file
    }

    /// `d` let through: put the entry back as HEAD has it, or delete it when
    /// git has no copy, then read the tree again.
    fn discard(&mut self, entry: Entry) {
        let (ok, out) = discard(&self.root, &entry);
        if ok {
            self.set_status(match entry.untracked() {
                true => format!("deleted {}", entry.base_name()),
                false => format!("deleted the changes to {}", entry.base_name()),
            });
        } else {
            let words: Vec<&str> = out.lines().take(GIT_WORDS).collect();
            self.modal = Some(Modal::new(
                format!("could not delete `{}`", entry.base_name()),
                words.join("\n"),
            ));
        }
        self.reload();
    }

    /// Something worked: green in the footer.
    fn set_status(&mut self, text: impl Into<String>) {
        self.note = Some((true, text.into()));
        self.note_at = Instant::now();
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
                    if let Some(gate) = &mut self.discard {
                        match code {
                            KeyCode::Enter if gate.typed == gate.entry.base_name() => {
                                if let Some(gate) = self.discard.take() {
                                    self.discard(gate.entry);
                                }
                            }
                            KeyCode::Esc => self.discard = None,
                            KeyCode::Backspace => {
                                gate.typed.pop();
                            }
                            KeyCode::Char(c) => gate.typed.push(c),
                            _ => {}
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
                    // So does an open filter - `q` types a letter there, it does not quit.
                    if self.editing {
                        self.filter_key(code);
                        continue;
                    }
                    if code == KeyCode::Char('/') {
                        self.query.open();
                        self.editing = true;
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

    /// A key typed into the open filter, narrowing the list as it goes. Enter
    /// keeps what it found and Esc clears it.
    fn filter_key(&mut self, code: KeyCode) {
        let changed = match code {
            KeyCode::Enter => {
                self.editing = false;
                false
            }
            KeyCode::Esc => {
                self.editing = false;
                self.query.clear();
                true
            }
            _ => self.query.on_key(code),
        };
        if changed {
            self.sel = 0;
            self.rescope();
        }
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
            reload = self.on_current(&["add"], Entry::to_stage);
        } else if code == KeyCode::Char('u') {
            reload = self.on_current(&["restore", "--staged"], Entry::to_unstage);
        } else if code == KeyCode::Char('S') {
            reload = self.on_visible(&["add"], Entry::unstaged, Entry::to_stage);
        } else if code == KeyCode::Char('U') {
            reload = self.on_visible(&["restore", "--staged"], Entry::staged, Entry::to_unstage);
        } else if code == KeyCode::Char(' ') {
            reload = self.toggle();
        } else if code == KeyCode::Char('d') {
            self.discard = self.current().map(|e| Discard {
                entry: e.clone(),
                typed: String::new(),
            });
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

/// What `d` is asking about: the entry as it was when the gate opened, so a
/// reload under it cannot change which file goes.
struct Discard {
    entry: Entry,
    typed: String,
}

impl Discard {
    fn title(&self) -> &'static str {
        match (self.entry.untracked(), self.entry.path.ends_with('/')) {
            (true, true) => "delete this directory?",
            (true, false) => "delete this file?",
            (false, _) => "delete its changes?",
        }
    }

    /// The whole name, then what is about to go.
    fn note(&self) -> String {
        let e = &self.entry;
        let name: String = e.label().iter().map(|s| s.content.as_ref()).collect();
        let lost = if e.untracked() {
            "untracked, so git has no copy: it goes from the disk".to_string()
        } else if let Some(orig) = e.orig.as_deref().filter(|_| e.x == 'R' || e.y == 'R') {
            format!("every change goes and it moves back to {orig}")
        } else if matches!(e.x, 'A' | 'C') || matches!(e.y, 'A' | 'C') {
            "added since the last commit, so the file itself goes".to_string()
        } else {
            "every change since the last commit goes, staged or not".to_string()
        };
        format!("{name}\n{lost}")
    }
}

/// Put an entry back as HEAD has it, reporting success and git's words.
/// `git restore` removes a path HEAD has no copy of, which is what takes back
/// an added file. A rename takes its old path back with it; a copy leaves its
/// source alone, since that is a file of its own.
fn discard(root: &str, entry: &Entry) -> (bool, String) {
    if entry.untracked() {
        return git_run(root, &["clean", "-fdq", "--", &entry.path]);
    }
    let mut args = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
    args.extend(entry.with_orig(entry.x == 'R' || entry.y == 'R'));
    git_run(root, &args)
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
    let mut args = vec!["diff"];
    args.extend(diff_args(entry, cached(entry, scope)));
    git_capture(root, &args).unwrap_or_default()
}

/// What follows `git diff` or `git difftool` to compare one side of an entry.
/// A move is given both its paths and the flag that pairs them: with only the
/// new path in the pathspec, git has nothing to pair it with and shows a file
/// wholly added.
fn diff_args(entry: &Entry, cached: bool) -> Vec<&str> {
    let mut args = Vec::new();
    if cached {
        args.push("--cached");
    }
    let moved = entry.moved_from(cached);
    if let Some((flag, _)) = moved {
        args.push(flag);
    }
    args.push("--");
    if let Some((_, orig)) = moved {
        args.push(orig);
    }
    args.push(&entry.path);
    args
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
    let rev = |rev: &str, path: &str| -> Blob {
        let (root, rev, path) = (root.to_string(), rev.to_string(), path.to_string());
        Box::new(move || load_blob(&root, &rev, &path))
    };
    if entry.untracked() {
        return DiffContext {
            old: None,
            new: Some(worktree()),
        };
    }
    let cached = cached(entry, scope);
    let old = entry
        .moved_from(cached)
        .map_or(&*entry.path, |(_, orig)| orig);
    if cached {
        DiffContext {
            old: Some(rev("HEAD", old)),
            new: Some(rev("", &entry.path)),
        }
    } else {
        DiffContext {
            old: Some(rev("", old)),
            new: Some(worktree()),
        }
    }
}

/// Read `git status --porcelain -z` into entries, untracked directories
/// expanded.
fn load_status(root: &str) -> Vec<Entry> {
    // `git_capture_raw`, not `git_capture`: the porcelain's first column is a
    // SPACE when a file has no staged change, and trimming would eat it on the
    // first record - shifting the status codes and the path by one char.
    let raw = match git_capture_raw(root, &["status", "--porcelain", "-z"]) {
        Some(r) => r,
        None => return Vec::new(),
    };
    expand_untracked_dirs(root, parse_status(&raw))
}

/// Parse `git status --porcelain -z`. `-z` NUL-separates records, so paths with
/// spaces or newlines are safe, and follows a rename or copy in either column
/// with one more NUL-terminated record holding the path it came from.
fn parse_status(raw: &str) -> Vec<Entry> {
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());
    let mut entries = Vec::new();
    while let Some(tok) = tokens.next() {
        let bytes = tok.as_bytes();
        if bytes.len() < 4 {
            continue;
        }
        let (x, y) = (bytes[0] as char, bytes[1] as char);
        let moved = matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C');
        entries.push(Entry {
            x,
            y,
            path: tok[3..].to_string(),
            orig: if moved {
                tokens.next().map(str::to_string)
            } else {
                None
            },
            hidden: 0,
        });
    }
    entries
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
    match git_capture_raw(root, &["status", "--porcelain", "-z", "-uall", "--", dir]) {
        Some(raw) => parse_status(&raw),
        None => Vec::new(),
    }
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
    let count = match (app.visible.is_empty(), app.query.text.is_empty()) {
        (true, true) => "clean".to_string(),
        (true, false) => "(none)".to_string(),
        (false, _) => format!("{}/{}", app.sel + 1, app.visible.len()),
    };
    let labels: Vec<&str> = SCOPES.iter().map(|s| s.label()).collect();
    let mut spans = scope_tabs(&labels, app.scope_idx);
    spans.push(Span::raw(format!(" {count}")));
    spans.extend(app.query.title_span('/', app.editing, true));
    spans.push(Span::raw(" "));
    let top_title = Line::from(spans);
    let list = List::new(items)
        .block(pane_block(top_title, true))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[0], &mut app.state);
    list_scrollbar(frame, areas[0], app.visible.len(), app.state.offset());

    // ── bottom: diff of the selected file ──
    let title = match app.current() {
        Some(e) => {
            // a move is named only on the side whose diff shows it
            let mut spans = vec![Span::raw(" ")];
            match e.moved_from(cached(e, app.scope())) {
                Some(_) => spans.extend(e.label()),
                None => spans.push(Span::raw(e.path.clone())),
            }
            spans.push(Span::raw(format!(" {} ", diff_tag(e, app.scope()))));
            Line::from(spans)
        }
        None => Line::from(" (nothing to show) "),
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
        "d del".to_string(),
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

    if let Some(gate) = &app.discard {
        typed_popup(
            frame,
            gate.title(),
            Some(&gate.note()),
            gate.entry.base_name(),
            &gate.typed,
            "delete",
        );
    }
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
        row("/", "filter the files by path"),
        row("s", "stage the file"),
        row("u", "unstage it"),
        row("S", "stage every file the list shows"),
        row("U", "unstage every file the list shows"),
        row("space", "flip it between staged and not"),
        row(CTRL_Y_MOVE, "scroll the diff"),
        row(CTRL_X_MOVE, "pan it sideways"),
        row("enter", "open the file in your git difftool"),
        row("d", "delete its changes, or the file when git has no copy"),
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

    let mut spans = vec![
        Span::styled(xc.to_string(), xs),
        Span::styled(yc.to_string(), ys),
        Span::raw("  "),
    ];
    spans.extend(e.label());
    spans.push(Span::styled(
        match e.hidden {
            0 => String::new(),
            n => format!("  {n} files"),
        },
        Style::default().fg(Color::DarkGray),
    ));
    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A throwaway repo under the system temp dir, deleted when it goes out of
    /// scope, including when an assertion panics, so a failing test leaves the
    /// machine as it found it.
    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new(tag: &str) -> TempRepo {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("sluuz-{tag}-{stamp}"));
            fs::create_dir_all(&dir).expect("temp dir");
            let repo = TempRepo(dir);
            repo.git(&["init", "-q"]);
            // set here so the machine's own git config cannot hide the rename
            repo.git(&["config", "status.renames", "true"]);
            repo
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("utf-8 temp path")
        }

        fn git(&self, args: &[&str]) {
            git_capture(self.path(), args).unwrap_or_else(|| panic!("git {args:?}"));
        }

        fn write(&self, file: &str, text: &str) {
            let path = self.0.join(file);
            fs::create_dir_all(path.parent().expect("a parent")).expect("fixture dir");
            fs::write(path, text).expect("write fixture");
        }

        fn read(&self, file: &str) -> Option<String> {
            fs::read_to_string(self.0.join(file)).ok()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn deleting_every_row_leaves_the_tree_as_head_has_it() {
        let repo = TempRepo::new("discard");
        let lines = "one\ntwo\nthree\nfour\nfive\n";
        repo.write("edited.txt", lines);
        repo.write("moved.txt", lines);
        repo.git(&["add", "."]);
        repo.git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "base",
        ]);

        // MM: a staged edit and an unstaged one on top
        repo.write("edited.txt", "staged\n");
        repo.git(&["add", "edited.txt"]);
        repo.write("edited.txt", "unstaged\n");
        // A: a file the last commit never had
        repo.write("added.txt", "new\n");
        repo.git(&["add", "added.txt"]);
        // RM: a staged rename edited after the move
        fs::create_dir_all(repo.0.join("sub")).expect("fixture dir");
        repo.git(&["mv", "moved.txt", "sub/moved.txt"]);
        repo.write("sub/moved.txt", &format!("{lines}six\n"));
        // ??: an untracked folder holding a file git ignores
        repo.write("junk/a.txt", "junk\n");
        repo.write("junk/keep.log", "keep\n");
        repo.write(".git/info/exclude", "*.log\n");

        let rows = load_status(repo.path());
        let codes: Vec<String> = rows.iter().map(|e| format!("{}{}", e.x, e.y)).collect();
        for code in ["MM", "A ", "RM", "??"] {
            assert!(
                codes.iter().any(|c| c == code),
                "the fixture has no `{code}` row to delete: {codes:?}"
            );
        }
        for row in &rows {
            let (ok, out) = discard(repo.path(), row);
            assert!(ok, "deleting `{}` failed: {out}", row.path);
        }

        let left: Vec<String> = load_status(repo.path())
            .iter()
            .map(|e| format!("{}{} {}", e.x, e.y, e.path))
            .collect();
        assert!(left.is_empty(), "rows survived their delete: {left:?}");
        assert_eq!(repo.read("edited.txt").as_deref(), Some(lines));
        assert_eq!(
            repo.read("moved.txt").as_deref(),
            Some(lines),
            "the rename went back to its old path as committed"
        );
        assert_eq!(
            repo.read("junk/keep.log").as_deref(),
            Some("keep\n"),
            "an ignored file inside the untracked folder was deleted"
        );
    }

    #[test]
    fn a_rename_in_either_column_takes_its_source_path_with_it() {
        let raw = "R  new.txt\0old.txt\0 R moved.txt\0gone.txt\0 M kept.txt\0";
        let rows: Vec<(String, Option<String>)> = parse_status(raw)
            .into_iter()
            .map(|e| (e.path, e.orig))
            .collect();
        assert_eq!(
            rows,
            [
                ("new.txt".into(), Some("old.txt".into())),
                ("moved.txt".into(), Some("gone.txt".into())),
                ("kept.txt".into(), None),
            ],
            "a source path was read as a row of its own"
        );
    }
}
