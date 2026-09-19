//! The drill: repos → branches → commits → files → diff.
//!
//! One app with one level stack, entered at whichever level the command asked
//! for. `slu irepos` starts at Repos, `slu ibranch` at Branches, `slu ilog` at
//! Commits; Esc walks back up and quits at the level it was entered on. `slu
//! itag` starts at Tags, which stands where Branches would: a tag's commits
//! step back to it.
//!
//! Every screen is two panes: the current level's list on top, the level below
//! it underneath, so the bottom pane always previews where Enter goes. Plain
//! keys drive the top pane, Ctrl drives the bottom one.

mod branches;
mod commits;
mod delete;
mod diff;
mod inspect;
mod keys;
mod repos;
mod tags;
mod ui;

pub use branches::Branch;

use crate::git::RepoStatus;
use crate::git::load::{self, Batch, Commit, FileEntry, RemoteTags, Tag};
use crate::tui::difffeed::DiffFeed;
use crate::tui::highlight::RenderedDiff;
use crate::tui::input::{Accel, stepped};
use crate::tui::widgets::{CommandLine, Modal, NOTE, Query};
use crate::tui::{pane_width, pop_keyboard_enhancement, push_keyboard_enhancement};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::text::Text;
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Ceiling on a branch's log. Nothing about the view needs a limit now that
/// rows stream in, so this is only there to bound memory on a history no one
/// is going to scroll to the end of anyway.
const COMMITS_PER_BRANCH: usize = 10_000;

/// How long a frame waits for a key while rows are still arriving. With nothing
/// in flight the loop blocks on the key instead, so an idle TUI costs nothing.
use crate::tui::FRAME;

/// How long a pane may be empty before it says it is still loading. A load
/// quick enough not to be noticed says nothing at all: a word that appears and
/// vanishes on every keypress is more noise than the blank it replaced.
const SLOW: Duration = Duration::from_millis(120);

/// Which list the top pane is showing. The bottom pane shows the next one down.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Repos,
    Branches,
    Tags,
    Commits,
    Diff,
}

impl Level {
    /// The level Esc steps back to from here, in an app entered at `start`, or
    /// None at the top of the stack. Only `itag` enters at Tags, and nothing
    /// sits above them.
    fn back(self, start: Level) -> Option<Level> {
        match self {
            Level::Repos | Level::Tags => None,
            Level::Branches => Some(Level::Repos),
            Level::Commits if start == Level::Tags => Some(Level::Tags),
            Level::Commits => Some(Level::Branches),
            Level::Diff => Some(Level::Commits),
        }
    }
}

/// A selection over one list: which of its entries the current scope keeps,
/// where the cursor sits in that subset, and the widget state that scrolls it.
#[derive(Default)]
pub struct Sel {
    /// Indices into the owning Vec, in display order.
    pub visible: Vec<usize>,
    pub cur: usize,
    pub state: ListState,
    /// Index into the level's scope slider, moved with h/l.
    pub scope: usize,
    /// What the cursor was on when `r` was pressed, named rather than numbered:
    /// a refresh exists because the list may have changed underneath, so the
    /// row that was selected can come back at a different position, or not at
    /// all - in which case the cursor simply stays at the top.
    pub restore: Option<String>,
    /// What `/` (or `?` on the pane below) narrowed this list to.
    pub query: Query,
}

impl Sel {
    /// A fresh selection parked on a level's default scope. Every level starts
    /// on its own stop, whichever level the app was entered at.
    fn scoped(scope: usize) -> Sel {
        Sel {
            scope,
            ..Sel::default()
        }
    }

    /// Show these entries, cursor back at the top.
    fn show(&mut self, visible: Vec<usize>) {
        self.visible = visible;
        self.cur = 0;
        self.state.select((!self.visible.is_empty()).then_some(0));
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Index into the owning Vec of whatever the cursor is on.
    pub fn idx(&self) -> Option<usize> {
        self.visible.get(self.cur).copied()
    }

    /// Put the cursor on the row a refresh was standing on, now that it has
    /// turned up among the rows that arrived.
    fn restored_at(&mut self, at: usize) {
        self.cur = at;
        self.state.select(Some(at));
        self.restore = None;
    }

    /// Add entries a background load produced. The cursor stays where the user
    /// put it: rows landing under it must never move the selection, which is
    /// the whole point of loading them in the background.
    fn append(&mut self, more: Vec<usize>) {
        let was_empty = self.visible.is_empty();
        self.visible.extend(more);
        if was_empty && !self.visible.is_empty() {
            self.cur = 0;
            self.state.select(Some(0));
        }
    }

    /// Move the cursor `rows` at a time, stopping at either end. Returns
    /// whether it actually moved, so the caller reloads only when it did.
    fn step(&mut self, down: bool, rows: usize) -> bool {
        if self.visible.is_empty() {
            return false;
        }
        let to = stepped(self.cur, self.visible.len(), down, rows);
        if to == self.cur {
            return false;
        }
        self.cur = to;
        self.state.select(Some(to));
        true
    }

    /// Slide the scope one notch within `len` stops, returning whether it moved.
    fn slide(&mut self, right: bool, len: usize) -> bool {
        if right && self.scope + 1 < len {
            self.scope += 1;
            true
        } else if !right && self.scope > 0 {
            self.scope -= 1;
            true
        } else {
            false
        }
    }
}

/// One pane's background load: the request number in flight, shared with the
/// worker so a superseded one stops itself, and what the pane on screen is
/// currently showing.
#[derive(Default)]
struct Feed {
    /// The newest request issued. The worker reads it to find out it has been
    /// abandoned.
    latest: Arc<AtomicU64>,
    /// The request whose rows the pane holds; a batch from any other is stale.
    shown: u64,
    /// Rows are still coming, so the frame timer stays on and the pane says so.
    loading: bool,
    /// When the request went out, which is what `slow` measures against.
    since: Option<Instant>,
}

impl Feed {
    /// Claim the next request number, superseding whatever was in flight.
    fn issue(&mut self) -> u64 {
        let seq = self.latest.fetch_add(1, Ordering::Relaxed) + 1;
        self.shown = seq;
        self.loading = true;
        self.since = Some(Instant::now());
        seq
    }

    /// Has this load been going long enough to be worth mentioning? Only then
    /// does an empty pane say so, which is what keeps a fast repo silent.
    fn slow(&self) -> bool {
        self.loading && self.since.is_some_and(|t| t.elapsed() >= SLOW)
    }

    /// Whether a batch belongs to the request the pane is showing.
    fn accepts(&self, seq: u64) -> bool {
        seq == self.shown
    }
}

/// Which of a level's two lists a filter key opens: `/` the one plain keys
/// drive, `?` the pane below it. Naming them by position rather than by list
/// keeps one pair of keys meaning the same thing at every level.
#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Top,
    Bottom,
}

impl Pane {
    /// The key that opens this pane's filter, which is also how the filter is
    /// written on its title: a query typed with `?` reading back as `/` is the
    /// pane telling you it went somewhere else.
    pub fn sigil(self) -> char {
        match self {
            Pane::Top => '/',
            Pane::Bottom => '?',
        }
    }
}

pub struct App {
    level: Level,
    /// Where the app was entered: Esc here quits instead of stepping back.
    start: Level,
    enhanced: bool,
    width: u16,
    /// What the last action came to, green when it worked and yellow when it
    /// did not, in the footer in place of the key hints until `NOTE` is up.
    note: Option<(bool, String)>,
    note_at: Instant,
    /// Rows the diff pane showed on the last frame, which is what a scroll is
    /// clamped to.
    diff_rows: u16,
    /// A message that has to be read before anything else happens: it owns
    /// every key until it is dismissed.
    modal: Option<Modal>,
    /// A `d` waiting on Yes or No, which owns every key the same way.
    confirm: Option<delete::Confirm>,
    /// `s` or `S` was pressed (`Some(pull)`). The sync runs after the frame
    /// that says so is drawn, or the screen would freeze for a network round
    /// trip with no word why.
    sync_next: Option<bool>,
    /// A delete on a remote waiting for the same frame, for the same reason.
    delete_next: Option<delete::Target>,
    /// What `u` would put back.
    undo: Option<delete::Undo>,
    /// The `:` line while one is open. It owns every key until it is run or
    /// dropped.
    command: Option<CommandLine>,
    /// The pane below the cursor is out of date. A cursor move only sets this,
    /// so a held `j` scrolls at the speed of the terminal and the git call it
    /// would have made is asked for once, when the keys stop coming.
    pending: bool,
    /// The pane whose filter is being typed into, if any. It owns every key
    /// until Enter keeps it or Esc clears it.
    editing: Option<Pane>,
    accel: Accel,
    /// Rows from the background loads land here; the drawing loop drains it
    /// once a frame.
    tx: Sender<Batch>,
    rx: Receiver<Batch>,

    // ── repos level ─────────────────────────────────────────────────────────
    repos: Vec<RepoStatus>,
    rsel: Sel,

    /// The repo every git call runs against. `"."` when the app was entered
    /// inside one repo rather than over a tree of them.
    repo: String,
    /// What `slu irepos` was pointed at, kept so `r` can scan it again.
    base: PathBuf,
    depth: usize,

    // ── branches level ──────────────────────────────────────────────────────
    branches: Vec<Branch>,
    bsel: Sel,
    bfeed: Feed,

    // ── tags level ──────────────────────────────────────────────────────────
    tags: Vec<Tag>,
    tsel: Sel,
    /// The remote's answer about its tags, None while it is being asked.
    remote: Option<RemoteTags>,
    /// The remote tags are compared against, and whether this repo has git
    /// keep a copy of its tags.
    tag_remote: Option<String>,
    tracked: bool,
    rfeed: Feed,
    /// The tag whose commits the commits pane holds, and what they are
    /// counted from.
    range_tag: String,
    since: tags::Since,

    // ── commits level ───────────────────────────────────────────────────────
    commits: Vec<Commit>,
    csel: Sel,
    cfeed: Feed,
    unpushed: HashSet<String>,
    /// Which repo `unpushed` was read from, so walking a tree of them reloads
    /// it exactly once per repo instead of once per keypress.
    unpushed_for: String,
    /// What to pass `git log`: the entry command's flags, or a branch name once
    /// one is picked at the branches level.
    log_args: Vec<String>,
    limit: usize,
    /// Pathspec from `slu ilog <path…>`: filters both the log and each commit's
    /// file list, so a path-filtered log shows only that file's change.
    paths: Vec<String>,

    // ── files: the bottom pane of the commits level ─────────────────────────
    files: Vec<FileEntry>,
    fsel: Sel,
    ffeed: Feed,
    /// The commit the files pane belongs to, so a list that streams a new
    /// commit in under the cursor is noticed and the pane follows it.
    files_for: String,

    // ── diff level ──────────────────────────────────────────────────────────
    dfeed: DiffFeed,
    prepared: RenderedDiff,
    diff: Text<'static>,
    diff_scroll: u16,
    diff_hscroll: u16,
}

impl App {
    fn new(start: Level, repo: String) -> App {
        let (tx, rx) = mpsc::channel();
        App {
            level: start,
            start,
            enhanced: false,
            width: 120,
            note: None,
            note_at: Instant::now(),
            diff_rows: 1,
            modal: None,
            confirm: None,
            sync_next: None,
            delete_next: None,
            undo: None,
            command: None,
            pending: false,
            editing: None,
            accel: Accel::default(),
            tx,
            rx,
            repos: Vec::new(),
            rsel: Sel::scoped(repos::DEFAULT_SCOPE),
            repo,
            base: PathBuf::from("."),
            depth: 0,
            branches: Vec::new(),
            bsel: Sel::scoped(branches::DEFAULT_SCOPE),
            bfeed: Feed::default(),
            tags: Vec::new(),
            tsel: Sel::scoped(tags::DEFAULT_SCOPE),
            remote: None,
            tag_remote: None,
            tracked: false,
            rfeed: Feed::default(),
            range_tag: String::new(),
            since: tags::Since::Asking,
            commits: Vec::new(),
            csel: Sel::scoped(commits::DEFAULT_SCOPE),
            cfeed: Feed::default(),
            unpushed: HashSet::new(),
            unpushed_for: String::new(),
            log_args: Vec::new(),
            limit: COMMITS_PER_BRANCH,
            paths: Vec::new(),
            files: Vec::new(),
            fsel: Sel::default(),
            ffeed: Feed::default(),
            files_for: String::new(),
            dfeed: DiffFeed::default(),
            prepared: RenderedDiff::default(),
            diff: Text::default(),
            diff_scroll: 0,
            diff_hscroll: 0,
        }
    }

    /// `slu irepos`: every repo under `base`, previewing the selected one's
    /// branches. Returns None when the tree holds no repos at all.
    pub fn at_repos(base: &Path, depth: usize, scope: usize) -> Option<App> {
        let mut app = App::new(Level::Repos, ".".to_string());
        app.rsel.scope = scope;
        app.base = base.to_path_buf();
        app.depth = depth;
        app.load_repos(base, depth);
        if app.repos.is_empty() {
            return None;
        }
        app.rescope_repos();
        app.enter_repo();
        Some(app)
    }

    /// `slu ibranch`: the branches of the repo we are standing in.
    pub fn at_branches(repo: String, scope: usize) -> Option<App> {
        let mut app = App::new(Level::Branches, repo);
        app.bsel.scope = scope;
        app.load_branches();
        if app.branches.is_empty() {
            return None;
        }
        app.rescope_branches();
        app.enter_branch();
        Some(app)
    }

    /// `slu itag`: the tags of the repo we are standing in, marked against its
    /// remote once that answers. Returns None when it has no tags.
    pub fn at_tags(repo: String) -> Option<App> {
        let mut app = App::new(Level::Tags, repo);
        app.load_tags();
        if app.tags.is_empty() {
            return None;
        }
        app.tag_remote = load::tag_remote(&app.repo);
        app.rescope_tags();
        app.request_remote_tags(true);
        app.enter_tag();
        Some(app)
    }

    /// `slu ilog`: one repo's commits, filtered by whatever the flags asked for.
    pub fn at_commits(
        repo: String,
        log_args: Vec<String>,
        limit: usize,
        paths: Vec<String>,
    ) -> Option<App> {
        let mut app = App::new(Level::Commits, repo);
        app.log_args = log_args;
        app.limit = limit;
        app.paths = paths;
        app.ensure_unpushed();
        // The log itself streams in once the screen is up, so the only thing
        // asked for here is whether there is anything to show at all: `ilog` in
        // an empty repo has to say so on the command line, not open a blank TUI.
        if !app.has_any_commit() {
            return None;
        }
        app.request_commits();
        Some(app)
    }

    /// Set up the terminal, run until the user quits, then put it back exactly
    /// as it was. `cmd` only names the command in an error line.
    pub fn run(mut self, cmd: &str) {
        let mut terminal = ratatui::init();
        self.enhanced = push_keyboard_enhancement();
        self.width = pane_width(&terminal);
        let result = self.event_loop(&mut terminal);
        if self.enhanced {
            pop_keyboard_enhancement();
        }
        ratatui::restore();

        if let Err(e) = result {
            eprintln!("slu {cmd}: {e}");
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            self.drain();

            // An empty input queue means the cursor has come to rest, so the
            // panes under it are worth asking for; mid-burst we skip it, which
            // is what stops a held `j` from starting a load per row.
            if self.pending && !event::poll(Duration::ZERO)? {
                self.settle();
            }
            terminal.draw(|frame| ui::draw(frame, self))?;
            if let Some(pull) = self.sync_next.take() {
                if self.level == Level::Repos {
                    self.sync_all(pull);
                } else {
                    self.sync(pull);
                }
                continue;
            }
            if let Some(target) = self.delete_next.take() {
                self.delete_on_remote(target);
                continue;
            }

            // While rows are still arriving, come back on a frame timer to show
            // them, and while a note is up, come back when it is due to go. With
            // neither there is nothing to wake for, so the loop blocks on the key
            // and leaves the CPU alone.
            let wait = match (self.filling(), self.note_left()) {
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
                        if self.level == Level::Diff {
                            self.relayout_diff();
                        }
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if keys::on_key(self, key, terminal) {
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Something worked: green in the footer.
    fn set_status(&mut self, text: impl Into<String>) {
        self.note = Some((true, text.into()));
        self.note_at = Instant::now();
    }

    /// Something did not, with nothing more to read than one line: yellow.
    fn set_failed(&mut self, text: impl Into<String>) {
        self.note = Some((false, text.into()));
        self.note_at = Instant::now();
    }

    fn note_left(&self) -> Option<Duration> {
        self.note
            .as_ref()
            .map(|_| NOTE.saturating_sub(self.note_at.elapsed()))
    }

    /// Is any pane still being streamed into? While one is, the loop comes back
    /// on a frame timer to show what has arrived.
    fn filling(&self) -> bool {
        self.bfeed.loading
            || self.rfeed.loading
            || self.cfeed.loading
            || self.ffeed.loading
            || self.dfeed.loading()
    }

    /// Take everything the background loads have produced since the last frame.
    /// A batch whose request the pane has moved on from is dropped: those rows
    /// belong to a branch or a commit that is no longer under the cursor.
    fn drain(&mut self) {
        while let Ok(batch) = self.rx.try_recv() {
            match batch {
                Batch::Branches { seq, rows, done } => {
                    if !self.bfeed.accepts(seq) {
                        continue;
                    }
                    let from = self.branches.len();
                    self.branches.extend(rows);
                    self.extend_branches(from);
                    if done {
                        self.bfeed.loading = false;
                    }
                }
                Batch::Commits { seq, rows, done } => {
                    if !self.cfeed.accepts(seq) {
                        continue;
                    }
                    let from = self.commits.len();
                    self.commits.extend(rows);
                    self.extend_commits(from);
                    if done {
                        self.cfeed.loading = false;
                    }
                }
                Batch::Files { seq, rows, done } => {
                    if !self.ffeed.accepts(seq) {
                        continue;
                    }
                    let from = self.files.len();
                    self.files.extend(rows);
                    self.extend_files(from);
                    if done {
                        self.ffeed.loading = false;
                    }
                }
                Batch::TagBase { seq, prev } => {
                    if !self.cfeed.accepts(seq) {
                        continue;
                    }
                    self.since = prev.map_or(tags::Since::Start, tags::Since::Tag);
                }
                Batch::RemoteTags { seq, answer } => {
                    if !self.rfeed.accepts(seq) {
                        continue;
                    }
                    self.rfeed.loading = false;
                    self.apply_remote(answer);
                }
            }
        }
        if let Some(prepared) = self.dfeed.take() {
            self.prepared = prepared;
            self.relayout_diff();
        }
        // Streaming can put a commit under the cursor that was not there when
        // the pane was last asked for, so the files pane follows it here - but
        // not mid-burst, or a held key starts a `git show` per row it passes.
        if !self.pending {
            self.sync_files();
        }
    }

    /// Run the load a cursor move deferred: every level asks for the pane below.
    fn settle(&mut self) {
        self.pending = false;
        match self.level {
            Level::Repos => self.enter_repo(),
            Level::Branches => self.enter_branch(),
            Level::Tags => self.enter_tag(),
            Level::Commits => self.enter_commit(),
            Level::Diff => {}
        }
    }

    /// The `Sel` a filter key acts on at this level. The diff has no list of
    /// its own below the commits, so only its top pane answers.
    fn pane_sel(&mut self, pane: Pane) -> Option<&mut Sel> {
        Some(match (self.level, pane) {
            (Level::Repos, Pane::Top) => &mut self.rsel,
            (Level::Repos, Pane::Bottom) | (Level::Branches, Pane::Top) => &mut self.bsel,
            (Level::Tags, Pane::Top) => &mut self.tsel,
            (Level::Branches | Level::Tags, Pane::Bottom) | (Level::Commits, Pane::Top) => {
                &mut self.csel
            }
            (Level::Commits, Pane::Bottom) => &mut self.fsel,
            (Level::Diff, _) => return None,
        })
    }

    /// Open a pane's filter for typing. A pane with no list behind it is not
    /// one you can narrow, so the key does nothing there.
    fn open_query(&mut self, pane: Pane) {
        let Some(sel) = self.pane_sel(pane) else {
            return;
        };
        sel.query.open();
        self.editing = Some(pane);
    }

    /// Re-apply a pane's scope and filter to everything already loaded, which
    /// is what a keystroke in the query bar changes.
    fn refilter(&mut self, pane: Pane) {
        match (self.level, pane) {
            (Level::Repos, Pane::Top) => self.rescope_repos(),
            (Level::Repos, Pane::Bottom) | (Level::Branches, Pane::Top) => self.rescope_branches(),
            (Level::Tags, Pane::Top) => self.rescope_tags(),
            (Level::Branches | Level::Tags, Pane::Bottom) | (Level::Commits, Pane::Top) => {
                self.rescope_commits()
            }
            (Level::Commits, Pane::Bottom) => self.rescope_files(),
            (Level::Diff, _) => {}
        }
        // Narrowing the top pane puts a different row under the cursor, so the
        // pane below it has to follow. Narrowing the bottom pane has nothing
        // under it to reload, and marking it would restart the very load whose
        // rows are being filtered.
        if pane == Pane::Top {
            self.pending = true;
        }
    }

    /// `r`: read this level again from git. It is the answer to a second
    /// terminal having committed something while this one was open - without it
    /// the only way to see the new commit is to quit and start over.
    ///
    /// The push-state set is dropped with it: that is cached once per repo, so
    /// a commit made a moment ago would otherwise arrive wearing the wrong mark.
    fn refresh(&mut self) {
        self.unpushed_for.clear();
        self.files_for.clear();
        match self.level {
            Level::Repos => {
                let keep = self.rsel.idx().map(|i| self.repos[i].path.clone());
                let (base, depth) = (self.base.clone(), self.depth);
                self.load_repos(&base, depth);
                self.rsel.restore = keep;
                self.rescope_repos();
                self.pending = true;
            }
            Level::Branches => {
                self.bsel.restore = self.bsel.idx().map(|i| self.branches[i].name.clone());
                self.request_branches();
                self.pending = true;
            }
            // The remote is asked again too: a tag pushed from another terminal
            // is exactly what `r` is pressed to see.
            Level::Tags => {
                self.tsel.restore = self.tag_name().map(str::to_string);
                self.load_tags();
                self.rescope_tags();
                self.request_remote_tags(true);
                self.pending = true;
            }
            Level::Commits => {
                // Read the push state again *before* the log: the commits about
                // to arrive are tested against this set, and a commit made since
                // the view opened is in neither the old set nor on a remote, so
                // a stale one silently reports it as pushed.
                self.ensure_unpushed();
                self.csel.restore = self.commit_hash().map(str::to_string);
                self.request_commits();
                self.pending = true;
            }
            // The diff is one file of one commit, so there is nothing to look up
            // again except that file.
            Level::Diff => self.open_diff(),
        }
    }

    /// Esc: step back one level, or quit if this is where we came in.
    fn back(&mut self) -> bool {
        match self.level.back(self.start) {
            Some(prev) if self.level > self.start => {
                self.level = prev;
                false
            }
            _ => true,
        }
    }
}

/// Starting tab for `slu ibranch`'s `-g`/`-r`.
pub fn branch_scope(gone: bool, remotes: bool) -> usize {
    let wanted = if gone {
        branches::Scope::Gone
    } else if remotes {
        branches::Scope::Remote
    } else {
        branches::Scope::Local
    };
    branches::SCOPES
        .iter()
        .position(|&s| s == wanted)
        .unwrap_or(branches::DEFAULT_SCOPE)
}

/// Starting stop on the repo scope slider for `slu irepos --dirty`.
pub fn repo_scope(dirty: bool) -> usize {
    let wanted = if dirty {
        repos::Scope::Dirty
    } else {
        repos::Scope::All
    };
    repos::SCOPES
        .iter()
        .position(|&s| s == wanted)
        .unwrap_or(repos::DEFAULT_SCOPE)
}

#[cfg(test)]
mod tests {
    use super::branches::{Branch, Scope as BranchScope};
    use super::repos::Scope as RepoScope;
    use super::{App, Level, Sel};
    use crate::git::{RepoStatus, git_capture};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A throwaway repo with a real remote of its own, deleted when it goes out
    /// of scope - including when an assertion panics, so a failing test leaves
    /// the machine as it found it. Everything lives under the system temp dir.
    struct Stage {
        dir: PathBuf,
    }

    impl Stage {
        /// A repo whose only commit is pushed, so nothing is unpushed yet.
        fn new() -> Stage {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("sluuz-refresh-{stamp}"));
            fs::create_dir_all(dir.join("work")).expect("temp dir");
            let stage = Stage { dir };
            let (work, remote) = (stage.work(), stage.remote());
            git_capture(".", &["init", "-q", "--bare", &remote]).expect("bare remote");
            git_capture(".", &["init", "-q", &work]).expect("work tree");
            stage.git(&["remote", "add", "origin", &remote]);
            stage.commit("base");
            stage.git(&["push", "-q", "-u", "origin", "HEAD"]);
            stage
        }

        fn work(&self) -> String {
            self.dir
                .join("work")
                .to_str()
                .expect("utf-8 temp path")
                .to_string()
        }

        fn remote(&self) -> String {
            self.dir
                .join("remote.git")
                .to_str()
                .expect("utf-8 temp path")
                .to_string()
        }

        fn git(&self, args: &[&str]) {
            git_capture(&self.work(), args).unwrap_or_else(|| panic!("git {args:?}"));
        }

        fn commit(&self, msg: &str) {
            self.git(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                msg,
            ]);
        }

        /// The hash of whatever HEAD is on now.
        fn head(&self) -> String {
            git_capture(&self.work(), &["rev-parse", "HEAD"]).expect("rev-parse")
        }
    }

    impl Drop for Stage {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_refreshed_log_sees_a_commit_made_after_it_opened_as_unpushed() {
        // `r` exists for exactly this: another terminal commits while this view
        // is open. The push-state set is read once per repo and cached, so a
        // refresh that reloads the log without reloading that set reports the
        // new commit as already pushed, and it arrives without its `↑`.
        let stage = Stage::new();
        let app = App::at_commits(stage.work(), Vec::new(), 200, Vec::new());
        let mut app = app.expect("the repo has a commit, so the view opens");
        assert!(
            app.unpushed.is_empty(),
            "everything was pushed when the view opened"
        );

        stage.commit("made while the view was open");
        let fresh = stage.head();

        app.refresh();
        assert!(
            app.unpushed.contains(&fresh),
            "a refresh has to read the push state again, not trust the cached one"
        );
    }

    fn repo(dirty: usize, ahead: usize) -> RepoStatus {
        RepoStatus {
            name: "r".into(),
            path: "/r".into(),
            branch: "main".into(),
            has_upstream: true,
            dirty,
            ahead,
            behind: 0,
            origin: "github.com:o/r".into(),
        }
    }

    fn branch(remote: bool, upstream: bool, track: &str) -> Branch {
        Branch {
            is_head: false,
            remote,
            name: "b".into(),
            refname: "refs/heads/b".into(),
            rel: String::new(),
            author: String::new(),
            has_upstream: upstream,
            upstream: if upstream {
                "origin/b".into()
            } else {
                String::new()
            },
            track: track.into(),
        }
    }

    #[test]
    fn esc_steps_back_one_level_at_a_time() {
        let start = Level::Repos;
        assert_eq!(Level::Diff.back(start), Some(Level::Commits));
        assert_eq!(Level::Commits.back(start), Some(Level::Branches));
        assert_eq!(Level::Branches.back(start), Some(Level::Repos));
        // Nothing above the repos level, so Esc there quits.
        assert_eq!(Level::Repos.back(start), None);
    }

    #[test]
    fn the_variant_order_is_the_drill_order() {
        // `App::back` steps back only while the current level is deeper than the
        // one the app was entered at, which is this ordering. Reorder the
        // variants and Esc starts quitting from the wrong place.
        assert!(Level::Repos < Level::Branches);
        assert!(Level::Branches < Level::Commits);
        assert!(Level::Commits < Level::Diff);
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut sel = Sel::default();
        sel.show(vec![0, 1]);
        assert!(!sel.step(false, 1), "already at the top");
        assert!(sel.step(true, 1));
        assert!(!sel.step(true, 1), "already at the bottom");
        assert_eq!(sel.cur, 1);
    }

    #[test]
    fn an_empty_list_selects_nothing() {
        let mut sel = Sel::default();
        sel.show(Vec::new());
        assert!(sel.is_empty());
        assert_eq!(sel.idx(), None);
        assert!(!sel.step(true, 1));
    }

    #[test]
    fn the_cursor_indexes_the_underlying_list_not_the_filtered_one() {
        let mut sel = Sel::default();
        sel.show(vec![3, 7]); // a scope that kept entries 3 and 7
        sel.step(true, 1);
        assert_eq!(sel.idx(), Some(7));
    }

    #[test]
    fn a_held_key_stops_at_both_ends() {
        // A held key moves several rows a press, and one that would carry past
        // either end has to land on the end rather than wrap or saturate the
        // cursor out of the list.
        let mut sel = Sel::default();
        sel.show((0..10).collect());
        assert!(sel.step(true, 4));
        assert_eq!(sel.cur, 4);
        assert!(sel.step(true, 99), "a step past the bottom still moves");
        assert_eq!(sel.cur, 9);
        assert!(!sel.step(true, 4), "already on the last row");
        assert!(sel.step(false, 99));
        assert_eq!(sel.cur, 0);
        assert!(!sel.step(false, 1), "already on the first row");
    }

    #[test]
    fn an_empty_list_cannot_be_stepped() {
        let mut sel = Sel::default();
        sel.show(Vec::new());
        assert!(!sel.step(true, 5));
        assert_eq!(sel.idx(), None);
    }

    #[test]
    fn rows_arriving_in_the_background_leave_the_cursor_alone() {
        // The whole point of loading in the background: a list that grows under
        // the cursor must not move it, or scrolling a long log would keep
        // yanking the selection back as batches land.
        let mut sel = Sel::default();
        sel.show(vec![0, 1, 2]);
        sel.step(true, 1);
        sel.step(true, 1);
        assert_eq!(sel.idx(), Some(2));
        sel.append(vec![3, 4, 5]);
        assert_eq!(sel.idx(), Some(2), "the cursor stayed where it was put");
        assert_eq!(sel.len(), 6);
    }

    #[test]
    fn the_first_rows_to_arrive_are_what_gets_selected() {
        // Nothing is selected while a pane is still empty, so the first batch
        // has to be what puts the cursor on screen.
        let mut sel = Sel::default();
        sel.show(Vec::new());
        assert_eq!(sel.idx(), None);
        sel.append(vec![7, 8]);
        assert_eq!(sel.idx(), Some(7));
        assert_eq!(sel.state.selected(), Some(0));
    }

    #[test]
    fn the_scope_slider_stops_at_its_ends() {
        let mut sel = Sel::scoped(1);
        assert!(sel.slide(true, 3));
        assert!(!sel.slide(true, 3), "already on the last stop");
        assert_eq!(sel.scope, 2);
        assert!(sel.slide(false, 3));
    }

    #[test]
    fn repo_scopes_split_dirty_from_unpushed() {
        assert!(RepoScope::Dirty.keeps(&repo(2, 0)));
        assert!(!RepoScope::Dirty.keeps(&repo(0, 2)));
        assert!(RepoScope::Unpushed.keeps(&repo(0, 2)));
        assert!(!RepoScope::Unpushed.keeps(&repo(2, 0)));
        assert!(RepoScope::All.keeps(&repo(0, 0)));
    }

    #[test]
    fn branch_scopes_split_local_from_remote() {
        assert!(BranchScope::Local.keeps(&branch(false, true, "")));
        assert!(!BranchScope::Local.keeps(&branch(true, false, "")));
        assert!(BranchScope::Remote.keeps(&branch(true, false, "")));
    }

    #[test]
    fn a_branch_counts_as_unpushed_when_no_remote_has_it() {
        assert!(branch(false, false, "").unpushed(), "never pushed");
        assert!(branch(false, true, "[gone]").unpushed(), "upstream deleted");
        assert!(
            branch(false, true, "[ahead 2]").unpushed(),
            "ahead of upstream"
        );
        assert!(!branch(false, true, "").unpushed(), "in sync");
        assert!(
            !branch(true, false, "").unpushed(),
            "a remote branch is on a remote"
        );
    }

    #[test]
    fn push_state_reads_the_track_field() {
        assert_eq!(branch(false, false, "").status(), "no remote");
        assert_eq!(branch(false, true, "[gone]").status(), "gone");
        assert_eq!(branch(false, true, "[ahead 2]").status(), "↑2");
        assert_eq!(branch(false, true, "[behind 3]").status(), "↓3");
        assert_eq!(branch(false, true, "[ahead 2, behind 1]").status(), "↑2 ↓1");
        assert_eq!(branch(false, true, "").status(), "synced");
    }
}
