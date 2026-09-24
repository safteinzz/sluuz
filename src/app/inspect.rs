//! `K`: everything about the row under the cursor that its line has no room
//! for, in a reader box over the panes. What that is depends on the level: a
//! repo, a branch, a tag, or at the commits and the diff, the commit.

use super::{App, Branch, Level};
use crate::git::load::{self, TagState};
use crate::tui::widgets::Modal;

type Rows = Vec<(String, String)>;

fn row(key: &str, value: impl Into<String>) -> (String, String) {
    (key.to_string(), value.into())
}

impl App {
    pub(super) fn inspect(&mut self) {
        let found = match self.level {
            Level::Repos => self.inspect_repo(),
            Level::Branches => self.inspect_branch(),
            Level::Tags => self.inspect_tag(),
            Level::Commits | Level::Diff => self.inspect_commit(),
        };
        if let Some((title, rows, text)) = found {
            self.modal = Some(Modal::reader(title, &rows).with_text(&text));
        }
    }

    fn inspect_repo(&self) -> Option<(String, Rows, String)> {
        let r = &self.repos[self.rsel.idx()?];
        let mut state = Vec::new();
        if r.dirty > 0 {
            state.push(count(r.dirty, "changed file"));
        }
        if r.ahead > 0 {
            state.push(format!("{} to push", count(r.ahead, "commit")));
        }
        if r.behind > 0 {
            state.push(format!("{} to pull", count(r.behind, "commit")));
        }
        if state.is_empty() {
            state.push("clean".to_string());
        }
        if !r.has_upstream {
            state.push(format!("`{}` tracks no upstream", r.branch));
        }
        let path =
            std::fs::canonicalize(&r.path).map_or(r.path.clone(), |p| p.display().to_string());
        let mut rows = vec![
            row("path", path),
            row("branch", r.branch.as_str()),
            row("state", state.join(", ")),
        ];
        let remotes = load::remote_urls(&r.path);
        if remotes.is_empty() {
            rows.push(row("remotes", "none"));
        }
        rows.extend(remotes);
        Some((format!("repo {}", r.name), rows, String::new()))
    }

    fn inspect_branch(&self) -> Option<(String, Rows, String)> {
        let b = &self.branches[self.bsel.idx()?];
        let mut rows = Vec::new();
        if b.remote {
            rows.push(row("kind", "remote branch"));
        } else {
            if b.is_head {
                rows.push(row("checked out", "yes"));
            }
            rows.push(row("upstream", push_state(b)));
            let own = load::unique_commits(&self.repo, &b.name);
            if own > 0 {
                rows.push(row("on no remote", count(own, "commit")));
            }
        }
        let mut text = String::new();
        if let Some(c) = load::commit_details(&self.repo, &b.refname) {
            rows.push(row("last commit", c.authored));
            rows.push(row("by", c.author));
            text = c.message.lines().next().unwrap_or("").to_string();
        }
        Some((format!("branch {}", b.name), rows, text))
    }

    fn inspect_tag(&self) -> Option<(String, Rows, String)> {
        let t = &self.tags[self.tsel.idx()?];
        let mut rows = vec![row(
            "kind",
            if t.annotated {
                "annotated"
            } else {
                "lightweight"
            },
        )];
        let remote = self.tag_remote.as_deref().unwrap_or("the remote");
        let state = match t.state {
            TagState::Unknown => None,
            TagState::Local => Some(format!("not on {remote}")),
            TagState::Pushed => Some(format!("pushed to {remote}")),
            TagState::Differs => Some(format!("{remote} has a different tag by this name")),
            TagState::Remote => Some(format!("only on {remote}, not fetched here")),
        };
        if let Some(state) = state {
            rows.push(row("remote", state));
        }
        if !t.commit.is_empty() {
            let short: String = t.commit.chars().take(8).collect();
            rows.push(row("commit", short));
        }
        rows.push(row("date", t.date.as_str()));
        let mut text = String::new();
        if let Some((tagger, message)) = load::tag_details(&self.repo, &t.name) {
            rows.push(row("tagger", tagger));
            text = message;
        } else if !t.subject.is_empty() {
            rows.push(row("subject", t.subject.as_str()));
        }
        Some((format!("tag {}", t.name), rows, text))
    }

    fn inspect_commit(&self) -> Option<(String, Rows, String)> {
        let c = &self.commits[self.csel.idx()?];
        let d = load::commit_details(&self.repo, &c.hash)?;
        let mut rows = vec![row("hash", c.hash.as_str())];
        if !c.refs.is_empty() {
            let refs: Vec<String> = c.refs.iter().map(|r| r.text()).collect();
            rows.push(row("refs", refs.join(", ")));
        }
        // A stash is never pushed, and saying so on every one says nothing.
        if !self.stashes {
            rows.push(row(
                "remote",
                if self.unpushed.contains(&c.hash) {
                    "on no remote yet"
                } else {
                    "pushed"
                },
            ));
        }
        rows.push(row("author", d.author.as_str()));
        rows.push(row("date", d.authored.as_str()));
        if d.committer != d.author {
            rows.push(row("committer", d.committer.as_str()));
        }
        if d.committed != d.authored {
            rows.push(row("committed", d.committed.as_str()));
        }
        // A stash's other parents hold its index and untracked files, which
        // is how git stores one rather than a merge of anything.
        if self.stashes {
            let base = d.parents.first().map_or("", String::as_str);
            rows.push(row("made on", base.chars().take(8).collect::<String>()));
            return Some((c.short.clone(), rows, d.message));
        }
        let parents = match d.parents.len() {
            0 => "none, the first commit".to_string(),
            1 => d.parents[0].clone(),
            _ => format!("{} (a merge)", d.parents.join(" ")),
        };
        rows.push(row("parents", parents));
        let title = if self.stashes {
            c.short.clone()
        } else {
            format!("commit {}", c.short)
        };
        Some((title, rows, d.message))
    }
}

/// Where a local branch stands against its upstream, in words.
fn push_state(b: &Branch) -> String {
    if !b.has_upstream {
        return "none: never pushed, or pushed without -u".to_string();
    }
    if b.track.contains("gone") {
        return format!("{}, deleted on the remote", b.upstream);
    }
    match b.status().as_str() {
        "synced" => format!("{}, in sync", b.upstream),
        state => format!("{}, {state}", b.upstream),
    }
}

/// `1 commit`, `3 commits`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
