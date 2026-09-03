//! The branches level: one repo's branches, and picking one to read.

use super::App;
use crate::git::load;

pub use crate::git::load::Branch;

impl Branch {
    /// A local branch that isn't fully on a remote: never pushed (no upstream),
    /// its upstream was deleted (`[gone]`), or it's ahead of its upstream.
    pub fn unpushed(&self) -> bool {
        !self.remote
            && (!self.has_upstream || self.track.contains("gone") || self.track.contains("ahead"))
    }

    /// Short push-state text: "no remote", "gone", "↑N ↓M", or "synced".
    pub fn status(&self) -> String {
        if self.remote {
            return String::new();
        }
        if !self.has_upstream {
            return "no remote".to_string();
        }
        if self.track.contains("gone") {
            return "gone".to_string();
        }
        let ahead = count(&self.track, "ahead");
        let behind = count(&self.track, "behind");
        match (ahead, behind) {
            (Some(a), Some(b)) => format!("↑{a} ↓{b}"),
            (Some(a), None) => format!("↑{a}"),
            (None, Some(b)) => format!("↓{b}"),
            (None, None) => "synced".to_string(),
        }
    }
}

/// Which slice of branches the top pane shows.
#[derive(Clone, Copy, PartialEq)]
pub enum Scope {
    Local,
    All,
    Remote,
}

/// Left→right order for the `h`/`l` slider; `All` in the middle.
pub const SCOPES: [Scope; 3] = [Scope::Local, Scope::All, Scope::Remote];
pub const DEFAULT_SCOPE: usize = 0;

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::All => "all",
            Scope::Remote => "remote",
        }
    }

    pub(super) fn keeps(self, b: &Branch) -> bool {
        match self {
            Scope::Local => !b.remote,
            Scope::Remote => b.remote,
            Scope::All => true,
        }
    }
}

impl App {
    /// Read this repo's branches and wait for them. Only the entry point uses
    /// it: `slu ibranch` has to know before the screen is up whether there is
    /// anything to show.
    pub(super) fn load_branches(&mut self) {
        self.branches = load::load_branches(&self.repo);
    }

    /// Start the same read in the background. Nothing waits for it - the pane
    /// empties now and fills as rows arrive, so walking a list of repos never
    /// stops on one of them.
    pub(super) fn request_branches(&mut self) {
        let seq = self.bfeed.issue();
        self.branches.clear();
        self.bsel.show(Vec::new());
        load::stream_branches(
            self.repo.clone(),
            seq,
            self.bfeed.latest.clone(),
            self.tx.clone(),
        );
    }

    /// Fold newly streamed branches into what is on screen, honouring both the
    /// scope and the filter, so rows that arrive during a search are held to
    /// the same test as the ones already there.
    pub(super) fn extend_branches(&mut self, from: usize) {
        // Rows arriving can put a different branch under the cursor - the first
        // batch after a refresh always does, since the list was empty a moment
        // ago - and the commits pane has to follow it.
        let before = self.bsel.idx();
        let scope = SCOPES[self.bsel.scope];
        let query = &self.bsel.query;
        let more: Vec<usize> = (from..self.branches.len())
            .filter(|&i| {
                scope.keeps(&self.branches[i]) && query.keeps(&haystack(&self.branches[i]))
            })
            .collect();
        self.bsel.append(more);
        if let Some(want) = self.bsel.restore.clone()
            && let Some(at) = self
                .bsel
                .visible
                .iter()
                .position(|&i| self.branches[i].name == want)
        {
            self.bsel.restored_at(at);
        }
        // Only when this level's plain keys drive the branch list: at the repos
        // level it is the preview pane, and marking the pane below it stale
        // re-runs the repo's own load, which requests these branches again.
        if self.level == super::Level::Branches {
            self.pending |= self.bsel.idx() != before;
        }
    }

    pub(super) fn rescope_branches(&mut self) {
        let scope = SCOPES[self.bsel.scope];
        let query = &self.bsel.query;
        let visible = (0..self.branches.len())
            .filter(|&i| {
                scope.keeps(&self.branches[i]) && query.keeps(&haystack(&self.branches[i]))
            })
            .collect();
        self.bsel.show(visible);
    }

    /// Load the selected branch's commits, which the pane below the branch list
    /// previews and the commits level then takes over.
    pub(super) fn enter_branch(&mut self) {
        let Some(i) = self.bsel.idx() else {
            self.commits.clear();
            self.csel.show(Vec::new());
            return;
        };
        self.ensure_unpushed();
        self.log_args = vec![self.branches[i].name.clone()];
        self.limit = super::COMMITS_PER_BRANCH;
        self.request_commits();
    }
}

/// Everything about a branch a filter can match: its name, its push state and
/// who last touched it.
fn haystack(b: &Branch) -> String {
    format!("{} {} {} {}", b.name, b.status(), b.rel, b.author)
}

/// Pull the number after `key` out of a `%(upstream:track)` string like
/// `[ahead 2, behind 1]`.
fn count(track: &str, key: &str) -> Option<u32> {
    let rest = &track[track.find(key)? + key.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .find(|t| !t.is_empty())
        .and_then(|t| t.parse().ok())
}
