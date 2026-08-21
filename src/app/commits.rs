//! The commits level: a branch's (or the log's) commits, and the files each one
//! touched in the pane below.

use super::App;
use crate::git::load;

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
}

impl App {
    /// Point every git call at `repo` and load the branches that go with it.
    pub(super) fn set_repo(&mut self, repo: String) {
        self.repo = repo;
        self.load_branches();
    }

    /// Read which commits are on no remote, once per repo. Only the commit
    /// panes need it, so walking the repo list never pays for it.
    pub(super) fn ensure_unpushed(&mut self) {
        if self.unpushed_for != self.repo {
            self.unpushed = load::load_unpushed(&self.repo);
            self.unpushed_for = self.repo.clone();
        }
    }

    /// Run the log for whatever `log_args` currently says: the entry command's
    /// flags, or the branch picked at the level above.
    pub(super) fn load_commits(&mut self) {
        let args: Vec<&str> = self.log_args.iter().map(String::as_str).collect();
        self.commits = load::load_commits(&self.repo, &args, self.limit);
        self.rescope_commits();
    }

    pub(super) fn rescope_commits(&mut self) {
        let scope = SCOPES[self.csel.scope];
        let visible = (0..self.commits.len())
            .filter(|&i| match scope {
                Scope::All => true,
                Scope::Local => self.unpushed.contains(&self.commits[i].hash),
                Scope::Pushed => !self.unpushed.contains(&self.commits[i].hash),
            })
            .collect();
        self.csel.show(visible);
    }

    /// Load the selected commit's files, which the pane below the commit list
    /// shows and Enter opens into a diff.
    pub(super) fn enter_commit(&mut self) {
        let Some(i) = self.csel.idx() else {
            self.files.clear();
            self.fsel.show(Vec::new());
            return;
        };
        let paths: Vec<&str> = self.paths.iter().map(String::as_str).collect();
        self.files = load::load_files(&self.repo, &self.commits[i].hash, &paths);
        self.fsel.show_all(self.files.len());
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
