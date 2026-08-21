//! The branches level: one repo's branches, and picking one to read.

use super::App;
use crate::git::{SEP, git_capture};

pub struct Branch {
    pub is_head: bool,
    pub remote: bool,
    pub name: String,
    pub rel: String,
    pub author: String,
    pub has_upstream: bool,
    /// Raw `%(upstream:track)`: "", "[gone]", "[ahead 2, behind 1]", …
    pub track: String,
}

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
    /// Load every branch (local + remote-tracking) with its push state, newest
    /// first.
    pub(super) fn load_branches(&mut self) {
        let fmt = format!(
            "--format=%(HEAD){SEP}%(refname){SEP}%(refname:short){SEP}%(committerdate:relative){SEP}%(authorname){SEP}%(upstream){SEP}%(upstream:track)"
        );
        self.branches = git_capture(
            &self.repo,
            &[
                "for-each-ref",
                "--sort=-committerdate",
                &fmt,
                "refs/heads",
                "refs/remotes",
            ],
        )
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    let head = f.next()?;
                    let refname = f.next()?;
                    let short = f.next()?;
                    let rel = f.next().unwrap_or("").to_string();
                    let author = f.next().unwrap_or("").to_string();
                    let upstream = f.next().unwrap_or("");
                    let track = f.next().unwrap_or("").to_string();
                    // Skip the symbolic `refs/remotes/*/HEAD` alias - it's noise.
                    if refname.ends_with("/HEAD") {
                        return None;
                    }
                    Some(Branch {
                        is_head: head.trim() == "*",
                        remote: refname.starts_with("refs/remotes/"),
                        name: short.to_string(),
                        rel,
                        author,
                        has_upstream: !upstream.is_empty(),
                        track,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    }

    pub(super) fn rescope_branches(&mut self) {
        let scope = SCOPES[self.bsel.scope];
        let visible = (0..self.branches.len())
            .filter(|&i| scope.keeps(&self.branches[i]))
            .collect();
        self.bsel.show(visible);
    }

    /// Load the selected branch's commits, which the pane below the branch list
    /// previews and the commits level then takes over.
    pub(super) fn enter_branch(&mut self) {
        let Some(i) = self.bsel.idx() else {
            self.commits.clear();
            self.csel.show(Vec::new());
            self.files.clear();
            self.fsel.show(Vec::new());
            return;
        };
        self.ensure_unpushed();
        self.log_args = vec![self.branches[i].name.clone()];
        self.limit = super::COMMITS_PER_BRANCH;
        self.load_commits();
    }
}

/// Pull the number after `key` out of a `%(upstream:track)` string like
/// `[ahead 2, behind 1]`.
fn count(track: &str, key: &str) -> Option<u32> {
    let rest = &track[track.find(key)? + key.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .find(|t| !t.is_empty())
        .and_then(|t| t.parse().ok())
}
