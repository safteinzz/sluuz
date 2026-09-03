//! The commits level: a branch's (or the log's) commits, and the files each one
//! touched in the pane below.

use super::App;
use crate::git::load::{self, Commit, FileEntry};
use std::collections::HashSet;

/// Everything about a commit a filter can match: its sha (so a short one is a
/// prefix of the long one), when it landed, who made it and what it says.
fn haystack(c: &Commit) -> String {
    format!("{} {} {} {}", c.hash, c.date, c.committer, c.subject)
}

/// A changed file matches on its path and on its status letter.
fn file_haystack(f: &FileEntry) -> String {
    format!("{} {}", f.status, f.path)
}

/// Push-state filter over the loaded commits.
#[derive(Clone, Copy, PartialEq)]
pub enum Scope {
    /// Only commits that are on no remote yet.
    Local,
    All,
    /// Only commits that are on some remote.
    Pushed,
}

/// Left→right order for the `h`/`l` slider; `All` in the middle.
pub const SCOPES: [Scope; 3] = [Scope::Local, Scope::All, Scope::Pushed];
pub const DEFAULT_SCOPE: usize = 1;

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::All => "all",
            Scope::Pushed => "pushed",
        }
    }

    fn keeps(self, unpushed: &HashSet<String>, c: &Commit) -> bool {
        match self {
            Scope::All => true,
            Scope::Local => unpushed.contains(&c.hash),
            Scope::Pushed => !unpushed.contains(&c.hash),
        }
    }
}

impl App {
    /// Point every git call at `repo` and load the branches that go with it.
    pub(super) fn set_repo(&mut self, repo: String) {
        self.repo = repo;
        self.request_branches();
    }

    /// Read which commits are on no remote, once per repo. Only the commit
    /// panes need it, so walking the repo list never pays for it.
    pub(super) fn ensure_unpushed(&mut self) {
        if self.unpushed_for != self.repo {
            self.unpushed = load::load_unpushed(&self.repo);
            self.unpushed_for = self.repo.clone();
        }
    }

    /// Is there anything in this log at all? One commit is enough to answer it,
    /// and a command that would otherwise open an empty screen asks first.
    pub(super) fn has_any_commit(&self) -> bool {
        let args: Vec<&str> = self.log_args.iter().map(String::as_str).collect();
        !load::first_commit(&self.repo, &args).is_empty()
    }

    /// Start the log for whatever `log_args` currently says: the entry command's
    /// flags, or the branch picked at the level above. Nothing waits for it -
    /// the pane empties now and fills as rows arrive.
    pub(super) fn request_commits(&mut self) {
        let seq = self.cfeed.issue();
        self.commits.clear();
        self.csel.show(Vec::new());
        load::stream_commits(
            self.repo.clone(),
            self.log_args.clone(),
            self.limit,
            seq,
            self.cfeed.latest.clone(),
            self.tx.clone(),
        );
    }

    /// Fold newly streamed commits into what is on screen, honouring both the
    /// scope and the filter, so rows that arrive during a search are held to
    /// the same test as the ones already there.
    pub(super) fn extend_commits(&mut self, from: usize) {
        let scope = SCOPES[self.csel.scope];
        let query = &self.csel.query;
        let unpushed = &self.unpushed;
        let more: Vec<usize> = (from..self.commits.len())
            .filter(|&i| {
                scope.keeps(unpushed, &self.commits[i]) && query.keeps(&haystack(&self.commits[i]))
            })
            .collect();
        self.csel.append(more);
        if let Some(want) = self.csel.restore.clone()
            && let Some(at) = self
                .csel
                .visible
                .iter()
                .position(|&i| self.commits[i].hash == want)
        {
            self.csel.restored_at(at);
        }
    }

    pub(super) fn rescope_commits(&mut self) {
        let scope = SCOPES[self.csel.scope];
        let query = &self.csel.query;
        let unpushed = &self.unpushed;
        let visible = (0..self.commits.len())
            .filter(|&i| {
                scope.keeps(unpushed, &self.commits[i]) && query.keeps(&haystack(&self.commits[i]))
            })
            .collect();
        self.csel.show(visible);
    }

    /// The files pane has no scope of its own, only the filter `?` typed into
    /// it - a hundred-file commit is exactly where that earns its keep.
    pub(super) fn extend_files(&mut self, from: usize) {
        let query = &self.fsel.query;
        let more: Vec<usize> = (from..self.files.len())
            .filter(|&i| query.keeps(&file_haystack(&self.files[i])))
            .collect();
        self.fsel.append(more);
    }

    pub(super) fn rescope_files(&mut self) {
        let query = &self.fsel.query;
        let visible = (0..self.files.len())
            .filter(|&i| query.keeps(&file_haystack(&self.files[i])))
            .collect();
        self.fsel.show(visible);
    }

    /// Point the files pane at whatever commit the cursor is on, asking git for
    /// its file list in the background. A no-op while it already shows that
    /// commit, so it is safe to call on every frame.
    pub(super) fn sync_files(&mut self) {
        // The pane only exists from the commits level down; previewing a branch
        // never needs it, and asking would cost a `git show` per branch moved.
        if self.level < super::Level::Commits {
            return;
        }
        let want = self.commit_hash().unwrap_or("").to_string();
        if want == self.files_for {
            return;
        }
        self.files_for = want.clone();
        self.files.clear();
        self.fsel.show(Vec::new());
        if want.is_empty() {
            self.ffeed.loading = false;
            return;
        }
        let seq = self.ffeed.issue();
        load::stream_files(
            self.repo.clone(),
            want,
            self.paths.clone(),
            seq,
            self.tx.clone(),
        );
    }

    /// The pane below the commit list follows the cursor.
    pub(super) fn enter_commit(&mut self) {
        self.sync_files();
    }

    /// Hash of the commit the cursor is on.
    pub(super) fn commit_hash(&self) -> Option<&str> {
        self.csel.idx().map(|i| self.commits[i].hash.as_str())
    }

    /// Path of the file the cursor is on in the files pane.
    pub(super) fn file_path(&self) -> Option<&str> {
        self.fsel.idx().map(|i| self.files[i].path.as_str())
    }
}
