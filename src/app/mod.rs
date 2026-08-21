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
use crate::git::load::{Commit, FileEntry};
use crate::tui::highlight::RenderedDiff;
use crate::tui::{pane_width, pop_keyboard_enhancement, push_keyboard_enhancement};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::text::Text;
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::io;
use std::path::Path;

/// How many commits to load for a branch picked at the branches level.
const COMMITS_PER_BRANCH: usize = 200;

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

    /// Show every entry of a list of `len`, unfiltered.
    fn show_all(&mut self, len: usize) {
        self.show((0..len).collect());
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

pub struct App {
    level: Level,
    /// Where the app was entered: Esc here quits instead of stepping back.
    start: Level,
    enhanced: bool,
    width: u16,
    /// Transient status line (a difftool complaint, mostly).
    msg: Option<String>,

    // ── repos level ─────────────────────────────────────────────────────────
    repos: Vec<RepoStatus>,
    rsel: Sel,

    /// The repo every git call runs against. `"."` when the app was entered
    /// inside one repo rather than over a tree of them.
    repo: String,

    // ── branches level ──────────────────────────────────────────────────────
    branches: Vec<Branch>,
    bsel: Sel,

    // ── commits level ───────────────────────────────────────────────────────
    commits: Vec<Commit>,
    csel: Sel,
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

    // ── diff level ──────────────────────────────────────────────────────────
    prepared: RenderedDiff,
    diff: Text<'static>,
    diff_scroll: u16,
    diff_hscroll: u16,
}

impl App {
    fn new(start: Level, repo: String) -> App {
        App {
            level: start,
            start,
            enhanced: false,
            width: 120,
            msg: None,
            repos: Vec::new(),
            rsel: Sel::scoped(repos::DEFAULT_SCOPE),
            repo,
            branches: Vec::new(),
            bsel: Sel::scoped(branches::DEFAULT_SCOPE),
            commits: Vec::new(),
            csel: Sel::scoped(commits::DEFAULT_SCOPE),
            unpushed: HashSet::new(),
            unpushed_for: String::new(),
            log_args: Vec::new(),
            limit: COMMITS_PER_BRANCH,
            paths: Vec::new(),
            files: Vec::new(),
            fsel: Sel::default(),
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
        app.load_commits();
        if app.commits.is_empty() {
            return None;
        }
        app.enter_commit();
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
            terminal.draw(|frame| ui::draw(frame, self))?;

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
    use super::{Level, Sel};
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
