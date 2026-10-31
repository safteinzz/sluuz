//! The repos level: every repo under a path, and picking one to drill into.

use super::{App, Level};
use crate::git::load::{self, Batch};
use crate::git::{
    Progress, RepoStatus, Stage, find_repos_counting, first_line, repo_status, with_remote_limit,
};
use crate::tui::widgets::{Modal, SPINNER};
use rayon::prelude::*;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

/// Which slice of the repos the top pane shows.
#[derive(Clone, Copy, PartialEq)]
pub enum Scope {
    Dirty,
    All,
    Unpushed,
}

/// Tab order: the default first, as in every view, then local towards remote.
pub const SCOPES: [Scope; 3] = [Scope::All, Scope::Dirty, Scope::Unpushed];
pub const DEFAULT_SCOPE: usize = 0;

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

/// Everything about a repo a `/` filter can match: what it is called, what it
/// is on, and where it came from.
fn haystack(r: &RepoStatus) -> String {
    format!("{} {} {}", r.name, r.branch, r.origin)
}

/// A repo's path relative to the directory being scanned (`crates/widget`), or
/// its plain name when it sits directly under it.
fn relative_name(repo: &Path, base: &Path) -> String {
    repo.strip_prefix(base)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        // The repo a scan started inside of sits above the scan root: it gets
        // its directory name rather than a full path.
        .unwrap_or_else(|| {
            repo.file_name()
                .unwrap_or(repo.as_os_str())
                .to_string_lossy()
                .into_owned()
        })
}

/// Find every repo under `base` and read its state. Statuses are read in
/// parallel: it is one `git status` per repo, and a tree of them is slow enough
/// serially to feel like a hang.
pub(super) fn scan(base: &Path, depth: usize, progress: &Progress) -> Vec<RepoStatus> {
    let found = find_repos_counting(base, depth, progress);
    progress.counted(found.len());
    let mut repos: Vec<RepoStatus> = found
        .par_iter()
        .map(|r| {
            let status = repo_status(r);
            progress.one_done();
            status
        })
        .collect();
    // Label each repo by where it sits under the scan root, not by its bare
    // directory name: a tree of them routinely holds two `work`s, and a
    // basename alone gives no way to tell which one you are about to open.
    for (repo, path) in repos.iter_mut().zip(&found) {
        repo.name = relative_name(path, base);
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos
}

/// A job over the whole repo list running off the input thread, so the list
/// stays usable and the footer can say how far it has got.
pub(super) struct RepoJob {
    verb: &'static str,
    progress: Arc<Progress>,
    since: Instant,
}

impl RepoJob {
    fn new(verb: &'static str, progress: Arc<Progress>) -> RepoJob {
        RepoJob {
            verb,
            progress,
            since: Instant::now(),
        }
    }

    /// The job as one footer entry. The mark keeps moving while no repo is
    /// finishing, which is what tells a slow server from a hung app.
    pub(super) fn line(&self) -> String {
        let at = self.since.elapsed().as_millis() / 80;
        let mark = SPINNER[at as usize % SPINNER.len()];
        match self.progress.stage() {
            Stage::Finding(found) => format!("{mark} finding repos {found}"),
            Stage::Working(done, total) => format!("{mark} {} {done}/{total}", self.verb),
        }
    }
}

impl App {
    pub(super) fn rescope_repos(&mut self) {
        let scope = SCOPES[self.rsel.scope];
        let query = &self.rsel.query;
        let visible = (0..self.repos.len())
            .filter(|&i| scope.keeps(&self.repos[i]) && query.keeps(&haystack(&self.repos[i])))
            .collect();
        self.rsel.show(visible);
        if let Some(want) = self.rsel.restore.clone()
            && let Some(at) = self
                .rsel
                .visible
                .iter()
                .position(|&i| self.repos[i].path == want)
        {
            self.rsel.restored_at(at);
        }
    }

    /// Read the repo list again in the background; `take_scan` lands the result.
    /// Does nothing while a job is already running, since one ends in this read.
    pub(super) fn read_repos(&mut self) {
        if self.repo_job.is_some() {
            return;
        }
        let progress = Arc::new(Progress::default());
        self.repo_job = Some(RepoJob::new("reading", progress.clone()));
        let (base, depth, tx) = (self.base.clone(), self.depth, self.tx.clone());
        thread::spawn(move || {
            let repos = scan(&base, depth, &progress);
            let _ = tx.send(Batch::RepoScan { repos });
        });
    }

    /// Swap in a finished read, with the cursor on the repo it is on now rather
    /// than the one it was on when the read started.
    pub(super) fn take_scan(&mut self, repos: Vec<RepoStatus>) {
        self.repo_job = None;
        let keep = self.rsel.idx().map(|i| self.repos[i].path.clone());
        self.repos = repos;
        self.rsel.restore = keep;
        self.rescope_repos();
        if self.level == Level::Repos {
            self.pending = true;
        }
    }

    /// `s`/`S` on the repos level: what `slu sync [--pull]` does, for every repo
    /// the list is showing, in the background. `finish_sync` lands the result.
    /// Does nothing while a job is already running.
    pub(super) fn sync_all(&mut self, pull: bool) {
        if self.repo_job.is_some() {
            return;
        }
        let repos: Vec<(String, String)> = self
            .rsel
            .visible
            .iter()
            .map(|&i| (self.repos[i].name.clone(), self.repos[i].path.clone()))
            .collect();
        let progress = Arc::new(Progress::of(repos.len()));
        let verb = if pull { "pulling" } else { "syncing" };
        self.repo_job = Some(RepoJob::new(verb, progress.clone()));
        self.note = None;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcomes = with_remote_limit(|| {
                repos
                    .par_iter()
                    .map(|(name, path)| {
                        let done = load::fetch_prune(path).and_then(|()| {
                            if pull {
                                load::fast_forward(path).map(|n| n.unwrap_or(0))
                            } else {
                                Ok(0)
                            }
                        });
                        progress.one_done();
                        (name.clone(), done)
                    })
                    .collect()
            });
            let _ = tx.send(Batch::Synced { pull, outcomes });
        });
    }

    /// Say how a sync went, then read the list again to show what it changed.
    pub(super) fn finish_sync(
        &mut self,
        pull: bool,
        outcomes: Vec<(String, Result<usize, String>)>,
    ) {
        let failed: Vec<String> = outcomes
            .iter()
            .filter_map(|(name, r)| {
                r.as_ref()
                    .err()
                    .map(|e| format!("{name}: {}", first_line(e)))
            })
            .collect();
        let pulled: usize = outcomes.iter().filter_map(|(_, r)| r.as_ref().ok()).sum();
        let into = outcomes
            .iter()
            .filter(|(_, r)| r.as_ref().is_ok_and(|&n| n > 0))
            .count();
        let synced = outcomes.len() - failed.len();
        let s = if synced == 1 { "" } else { "s" };
        let mut text = format!("synced {synced} repo{s}");
        if pull && pulled > 0 {
            let c = if pulled == 1 { "" } else { "s" };
            let r = if into == 1 { "" } else { "s" };
            text.push_str(&format!("; pulled {pulled} commit{c} into {into} repo{r}"));
        }
        if failed.is_empty() {
            self.set_status(text);
        } else {
            self.note = None;
            self.held_alert = Some(Modal::new(
                format!("{} of {} repos did not sync", failed.len(), outcomes.len()),
                failed.join("\n"),
            ));
        }
        self.repo_job = None;
        self.read_repos();
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
    }
}

#[cfg(test)]
mod tests {
    use super::relative_name;
    use std::path::Path;

    #[test]
    fn a_repo_under_the_root_keeps_its_plain_name() {
        assert_eq!(
            relative_name(Path::new("pg/hscroll"), Path::new("pg")),
            "hscroll"
        );
    }

    #[test]
    fn a_nested_repo_is_named_by_its_path() {
        // Two `work` repos in one tree are only telling apart by where they sit.
        assert_eq!(
            relative_name(Path::new("pg/itidy/work"), Path::new("pg")),
            "itidy/work"
        );
        assert_eq!(
            relative_name(Path::new("pg/pushstate/work"), Path::new("pg")),
            "pushstate/work"
        );
    }

    #[test]
    fn scanning_the_current_directory_drops_the_dot() {
        assert_eq!(
            relative_name(Path::new("./crates/widget"), Path::new(".")),
            "crates/widget"
        );
        assert_eq!(
            relative_name(Path::new("./hscroll"), Path::new(".")),
            "hscroll"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_root_changes_nothing() {
        assert_eq!(
            relative_name(Path::new("/pg/crates/widget"), Path::new("/pg/")),
            "crates/widget"
        );
    }

    #[test]
    fn a_path_outside_the_root_falls_back_to_its_name() {
        assert_eq!(
            relative_name(Path::new("/elsewhere/widget"), Path::new("/pg")),
            "widget"
        );
    }
}
