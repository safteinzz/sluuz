//! The repos level: every repo under a path, and picking one to drill into.

use super::App;
use crate::git::{find_repos, repo_status, RepoStatus};
use rayon::prelude::*;
use std::path::Path;

/// Which slice of the repos the top pane shows. Left is the most local view
/// (uncommitted work), right the most remote (commits no remote has), matching
/// the way the branch and commit sliders are laid out.
#[derive(Clone, Copy, PartialEq)]
pub enum Scope {
    Dirty,
    All,
    Unpushed,
}

pub const SCOPES: [Scope; 3] = [Scope::Dirty, Scope::All, Scope::Unpushed];
/// `slu irepos` opens on the middle stop, showing everything.
pub const DEFAULT_SCOPE: usize = 1;

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Dirty => "dirty",
            Scope::All => "all",
            Scope::Unpushed => "unpushed",
        }
    }

    pub(super) fn keeps(self, r: &RepoStatus) -> bool {
        match self {
            Scope::Dirty => r.is_dirty(),
            Scope::All => true,
            Scope::Unpushed => r.is_unpushed(),
        }
    }
}

impl App {
    /// Find every repo under `base` and read its state. Statuses are read in
    /// parallel: it is one `git status` per repo, and a tree of them is slow
    /// enough serially to feel like a hang.
    pub(super) fn load_repos(&mut self, base: &Path, depth: usize) {
        let found = find_repos(base, depth);
        let mut repos: Vec<RepoStatus> = found.par_iter().map(|r| repo_status(r)).collect();
        repos.sort_by(|a, b| a.name.cmp(&b.name));
        self.repos = repos;
    }

    pub(super) fn rescope_repos(&mut self) {
        let scope = SCOPES[self.rsel.scope];
        let visible = (0..self.repos.len())
            .filter(|&i| scope.keeps(&self.repos[i]))
            .collect();
        self.rsel.show(visible);
    }

    /// Point the app at the selected repo and load its branches, which are what
    /// the pane below the repo list previews.
    pub(super) fn enter_repo(&mut self) {
        let Some(i) = self.rsel.idx() else {
            self.branches.clear();
            self.bsel.show(Vec::new());
            return;
        };
        self.set_repo(self.repos[i].path.clone());
        self.rescope_branches();
    }

}
