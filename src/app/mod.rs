//! The drill: repos → branches → commits → files → diff.
//!
//! One app with one level stack, entered at whichever level the command asked
//! for. `slu irepos` starts at Repos, `slu ibranch` at Branches, `slu ilog` at
//! Commits; Esc walks back up and quits at the level it was entered on.
//!
//! Every screen is two panes: the current level's list on top, the level below
//! it underneath, so the bottom pane always previews where Enter goes. Plain
//! keys drive the top pane, Ctrl drives the bottom one.

mod branches;
mod commits;
mod diff;
mod keys;
mod repos;
mod ui;

pub use branches::Branch;

use crate::git::RepoStatus;
use crate::git::load::{Batch, Commit, FileEntry};
use crate::tui::highlight::RenderedDiff;
use crate::tui::input::char_to_byte;
use crate::tui::widgets::Modal;
use crate::tui::{pane_width, pop_keyboard_enhancement, push_keyboard_enhancement};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::text::Text;
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::io;
use std::path::Path;
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
const FRAME: Duration = Duration::from_millis(33);

/// How long a pane may be empty before it says it is still loading. A load
/// quick enough not to be noticed says nothing at all: a word that appears and
/// vanishes on every keypress is more noise than the blank it replaced.
const SLOW: Duration = Duration::from_millis(120);

/// Which list the top pane is showing. The bottom pane shows the next one down.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Repos,
    Branches,
    Commits,
    Diff,
}

impl Level {
    /// The level Esc steps back to, or None at the top of the stack.
    fn back(self) -> Option<Level> {
        match self {
            Level::Repos => None,
            Level::Branches => Some(Level::Repos),
            Level::Commits => Some(Level::Branches),
            Level::Diff => Some(Level::Commits),
        }
    }
}

/// A pane's live filter: what has been typed into it, and where the caret sits
/// in that text. Empty means the pane shows everything its scope keeps.
#[derive(Default)]
pub struct Query {
    pub text: String,
    pub caret: usize,
}

impl Query {
    /// Does a row survive this filter? Terms are whitespace-separated and every
    /// one of them has to appear somewhere in the row, so `pablo fix` narrows
    /// to what both words are in rather than to either.
    pub fn keeps(&self, row: &str) -> bool {
        if self.text.trim().is_empty() {
            return true;
        }
        let row = row.to_lowercase();
        self.text
            .split_whitespace()
            .all(|term| row.contains(&term.to_lowercase()))
    }

    fn insert(&mut self, c: char) {
        self.text.insert(char_to_byte(&self.text, self.caret), c);
        self.caret += 1;
    }

    fn backspace(&mut self) {
        if self.caret > 0 {
            self.text.remove(char_to_byte(&self.text, self.caret - 1));
            self.caret -= 1;
        }
    }

    fn delete(&mut self) {
        if self.caret < self.text.chars().count() {
            self.text.remove(char_to_byte(&self.text, self.caret));
        }
    }

    fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
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

    /// Move the cursor one row, returning whether it actually moved (so the
    /// caller only reloads the pane below when something changed).
    fn step(&mut self, down: bool) -> bool {
        let moved = if down {
            self.cur + 1 < self.visible.len() && {
                self.cur += 1;
                true
            }
        } else {
            self.cur > 0 && {
                self.cur -= 1;
                true
            }
        };
        if moved {
            self.state.select(Some(self.cur));
        }
        moved
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
    fn jump(&mut self, down: bool, rows: usize) -> bool {
        if self.visible.is_empty() {
            return false;
        }
        let last = self.visible.len() - 1;
        let to = if down {
            (self.cur + rows).min(last)
        } else {
            self.cur.saturating_sub(rows)
        };
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
    /// Transient status line (a difftool complaint, mostly).
    msg: Option<String>,
    /// A message that has to be read before anything else happens: it owns
    /// every key until it is dismissed.
    modal: Option<Modal>,
    /// The pane below the cursor is out of date. A cursor move only sets this,
    /// so a held `j` scrolls at the speed of the terminal and the git call it
    /// would have made is asked for once, when the keys stop coming.
    pending: bool,
    /// The pane whose filter is being typed into, if any. It owns every key
    /// until Enter keeps it or Esc clears it.
    editing: Option<Pane>,
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

    // ── branches level ──────────────────────────────────────────────────────
    branches: Vec<Branch>,
    bsel: Sel,
    bfeed: Feed,

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
            msg: None,
            modal: None,
            pending: false,
            editing: None,
            tx,
            rx,
            repos: Vec::new(),
            rsel: Sel::scoped(repos::DEFAULT_SCOPE),
            repo,
            branches: Vec::new(),
            bsel: Sel::scoped(branches::DEFAULT_SCOPE),
            bfeed: Feed::default(),
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

            // While rows are still arriving, come back on a frame timer to show
            // them. With nothing in flight there is nothing to wake for, so the
            // loop blocks on the key and leaves the CPU alone.
            if self.filling() && !event::poll(FRAME)? {
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

    /// Is any pane still being streamed into? While one is, the loop comes back
    /// on a frame timer to show what has arrived.
    fn filling(&self) -> bool {
        self.bfeed.loading || self.cfeed.loading || self.ffeed.loading
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
            }
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
            (Level::Branches, Pane::Bottom) | (Level::Commits, Pane::Top) => &mut self.csel,
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
        sel.query.caret = sel.query.text.chars().count();
        self.editing = Some(pane);
    }

    /// Re-apply a pane's scope and filter to everything already loaded, which
    /// is what a keystroke in the query bar changes.
    fn refilter(&mut self, pane: Pane) {
        match (self.level, pane) {
            (Level::Repos, Pane::Top) => self.rescope_repos(),
            (Level::Repos, Pane::Bottom) | (Level::Branches, Pane::Top) => self.rescope_branches(),
            (Level::Branches, Pane::Bottom) | (Level::Commits, Pane::Top) => self.rescope_commits(),
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

    /// Esc: step back one level, or quit if this is where we came in.
    fn back(&mut self) -> bool {
        match self.level.back() {
            Some(prev) if self.level > self.start => {
                self.level = prev;
                false
            }
            _ => true,
        }
    }
}

/// Starting stop on the branch scope slider for `slu ibranch`'s `-a`/`-r`.
pub fn branch_scope(all: bool, remotes: bool) -> usize {
    if remotes && !all {
        2
    } else if all {
        1
    } else {
        branches::DEFAULT_SCOPE
    }
}

/// Starting stop on the repo scope slider for `slu irepos --dirty`.
pub fn repo_scope(dirty: bool) -> usize {
    if dirty { 0 } else { repos::DEFAULT_SCOPE }
}

#[cfg(test)]
mod tests {
    use super::branches::{Branch, Scope as BranchScope};
    use super::repos::Scope as RepoScope;
    use super::{Level, Query, Sel};
    use crate::git::RepoStatus;

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

    fn query(text: &str) -> Query {
        Query {
            text: text.to_string(),
            caret: 0,
        }
    }

    fn branch(remote: bool, upstream: bool, track: &str) -> Branch {
        Branch {
            is_head: false,
            remote,
            name: "b".into(),
            rel: String::new(),
            author: String::new(),
            has_upstream: upstream,
            track: track.into(),
        }
    }

    #[test]
    fn esc_steps_back_one_level_at_a_time() {
        assert_eq!(Level::Diff.back(), Some(Level::Commits));
        assert_eq!(Level::Commits.back(), Some(Level::Branches));
        assert_eq!(Level::Branches.back(), Some(Level::Repos));
        // Nothing above the repos level, so Esc there quits.
        assert_eq!(Level::Repos.back(), None);
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
        assert!(!sel.step(false), "already at the top");
        assert!(sel.step(true));
        assert!(!sel.step(true), "already at the bottom");
        assert_eq!(sel.cur, 1);
    }

    #[test]
    fn an_empty_list_selects_nothing() {
        let mut sel = Sel::default();
        sel.show(Vec::new());
        assert!(sel.is_empty());
        assert_eq!(sel.idx(), None);
        assert!(!sel.step(true));
    }

    #[test]
    fn the_cursor_indexes_the_underlying_list_not_the_filtered_one() {
        let mut sel = Sel::default();
        sel.show(vec![3, 7]); // a scope that kept entries 3 and 7
        sel.step(true);
        assert_eq!(sel.idx(), Some(7));
    }

    #[test]
    fn paging_stops_at_both_ends() {
        // `PageUp`/`PageDown` and `Ctrl-d`/`Ctrl-u` move by a pane's worth, and
        // a jump past either end has to land on the end rather than wrap or
        // saturate the cursor out of the list.
        let mut sel = Sel::default();
        sel.show((0..10).collect());
        assert!(sel.jump(true, 4));
        assert_eq!(sel.cur, 4);
        assert!(sel.jump(true, 99), "a jump past the bottom still moves");
        assert_eq!(sel.cur, 9);
        assert!(!sel.jump(true, 4), "already on the last row");
        assert!(sel.jump(false, 99));
        assert_eq!(sel.cur, 0);
        assert!(!sel.jump(false, 1), "already on the first row");
    }

    #[test]
    fn an_empty_list_cannot_be_paged() {
        let mut sel = Sel::default();
        sel.show(Vec::new());
        assert!(!sel.jump(true, 5));
        assert_eq!(sel.idx(), None);
    }

    #[test]
    fn rows_arriving_in_the_background_leave_the_cursor_alone() {
        // The whole point of loading in the background: a list that grows under
        // the cursor must not move it, or scrolling a long log would keep
        // yanking the selection back as batches land.
        let mut sel = Sel::default();
        sel.show(vec![0, 1, 2]);
        sel.step(true);
        sel.step(true);
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
    fn a_filter_keeps_only_rows_every_term_is_in() {
        // `/` and `?` split on whitespace and require all of them, which is what
        // lets `pablo fix` mean both words rather than either.
        assert!(
            query("").keeps("anything at all"),
            "an empty filter keeps everything"
        );

        let q = query("pablo fix");
        assert!(q.keeps("a1b2c3 2026-09-03 pablo fix: the thing"));
        assert!(!q.keeps("a1b2c3 2026-09-03 pablo feat: the thing"));
        assert!(!q.keeps("a1b2c3 2026-09-03 marta fix: the thing"));
    }

    #[test]
    fn a_filter_ignores_case_on_both_sides() {
        assert!(query("FIX Pablo").keeps("pablo fix: lowercase row"));
        assert!(query("fix").keeps("PABLO FIX: UPPERCASE ROW"));
    }

    #[test]
    fn a_filter_of_only_spaces_is_no_filter() {
        // Typing a space and deleting the word must not leave a query that
        // matches nothing at all.
        assert!(query("   ").keeps("anything"));
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
        assert!(BranchScope::All.keeps(&branch(true, false, "")));
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
