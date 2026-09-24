//! `istash`: the commits level walking the stash instead of a branch.
//!
//! A stash is a merge commit whose first parent is the HEAD it was made on, so
//! the files pane and the diff, which read a merge against its first parent,
//! show exactly what it holds. What differs is that its rows are named by
//! position (`stash@{2}`), which is also what `git stash` takes.

use super::App;
use super::delete::{Confirm, GIT_WORDS, Target};
use crate::git::{git_capture, git_run};
use crate::tui::widgets::Modal;

/// How many of git's lines a failed apply or pop shows.
const STASH_WORDS: usize = 5;

/// git's words for a failed apply or pop, cut to `STASH_WORDS` lines, with the
/// files it indents with a tab listed as `- path` and never cut, since a list
/// of them is what the user has to go and fix.
fn failure_lines(words: &str) -> String {
    let mut said = 0;
    let mut out = Vec::new();
    for line in words.lines() {
        // git's "kept" and "Aborting" say nothing a title does not.
        if line.starts_with("The stash entry is kept") || line == "Aborting" {
            continue;
        }
        if let Some(path) = line.strip_prefix('\t') {
            out.push(format!("- {path}"));
        } else if said < STASH_WORDS {
            said += 1;
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

/// What `git log` walks for the stash list: its reflog, newest first, which is
/// the order `stash@{n}` counts in.
pub(super) fn stash_log() -> Vec<String> {
    vec!["-g".to_string(), "refs/stash".to_string()]
}

impl App {
    /// Name newly streamed stash rows by their place in the reflog, in place of
    /// the short hash `git stash` never asks for.
    pub(super) fn name_stashes(&mut self, from: usize) {
        for (i, c) in self.commits.iter_mut().enumerate().skip(from) {
            c.short = format!("stash@{{{i}}}");
        }
    }

    /// The stash under the cursor as `(stash@{n}, hash)`, provided `stash@{n}`
    /// still names it. A stash made or dropped in another terminal shifts every
    /// number under it, and acting on a shifted one would apply or drop the
    /// wrong stash.
    fn stash_here(&mut self) -> Option<(String, String)> {
        let i = self.csel.idx()?;
        let (name, hash) = (self.commits[i].short.clone(), self.commits[i].hash.clone());
        let now = git_capture(&self.repo, &["rev-parse", "--verify", "-q", &name]);
        if now.as_deref() != Some(hash.as_str()) {
            self.set_failed("the stash list changed since it was read · r reads it again");
            return None;
        }
        Some((name, hash))
    }

    /// `a` and `p`: put the stash's changes back in the working tree, and with
    /// `pop` drop it once they are. git keeps a stash whose pop conflicted, so a
    /// failed pop has lost nothing.
    pub(super) fn apply_stash(&mut self, pop: bool) {
        let Some((name, _)) = self.stash_here() else {
            return;
        };
        let verb = if pop { "pop" } else { "apply" };
        // `-q`, or git follows its error with a whole `git status` on stdout,
        // which `git_run` puts ahead of the error itself.
        let (ok, out) = git_run(&self.repo, &["stash", verb, "-q", &name]);
        if !ok {
            // A conflict is the one failure `-q` says nothing about beyond the
            // stash being kept, and it still puts the changes in the tree,
            // markers and all, so the unmerged files are what to say. Anything
            // else git said is a refusal that changed nothing, even with files
            // left unmerged from before.
            let unmerged = git_capture(&self.repo, &["diff", "--name-only", "--diff-filter=U"])
                .unwrap_or_default();
            let refused = !failure_lines(&out).is_empty();
            let (title, words) = if !refused && !unmerged.is_empty() {
                let kept = if pop { "\nthe stash is kept" } else { "" };
                (
                    format!("`{name}` applied with conflicts"),
                    format!(
                        "resolve them in:\n\t{}{kept}",
                        unmerged.replace('\n', "\n\t")
                    ),
                )
            } else {
                (format!("could not {verb} `{name}`"), out)
            };
            self.modal = Some(Modal::new(title, failure_lines(&words)));
            return;
        }
        if pop {
            self.set_status(format!("popped {name} into the working tree"));
            self.reread_stashes();
        } else {
            self.set_status(format!("applied {name}; it is still stashed"));
        }
    }

    /// `d`: the typed gate, since a dropped stash's changes exist nowhere else.
    pub(super) fn ask_drop_stash(&mut self) {
        let Some(i) = self.csel.idx() else {
            return;
        };
        let c = &self.commits[i];
        self.confirm = Some(Confirm::new(Target::Stash {
            name: c.short.clone(),
            subject: c.subject.clone(),
            hash: c.hash.clone(),
        }));
    }

    /// The drop the typed gate let through: the stash it named, whatever the
    /// cursor has moved to since, and only while `name` still means it.
    pub(super) fn drop_stash(&mut self, name: &str, hash: &str) {
        let now = git_capture(&self.repo, &["rev-parse", "--verify", "-q", name]);
        if now.as_deref() != Some(hash) {
            self.set_failed("the stash list changed since it was read · r reads it again");
            return;
        }
        let (ok, out) = git_run(&self.repo, &["stash", "drop", "-q", name]);
        if !ok {
            let words: Vec<&str> = out.lines().take(GIT_WORDS).collect();
            self.modal = Some(Modal::new(
                format!("could not drop `{name}`"),
                words.join("\n"),
            ));
            return;
        }
        let short: String = hash.chars().take(7).collect();
        self.set_status(format!("dropped {name} (was {short})"));
        self.reread_stashes();
    }

    /// Read the stash again after one left it, the cursor on the row that was
    /// next to it. Rows are kept by hash, since every number below it moved up.
    fn reread_stashes(&mut self) {
        let at = self.csel.cur;
        let next = self
            .csel
            .visible
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|b| self.csel.visible.get(b)));
        self.csel.restore = next.map(|&i| self.commits[i].hash.clone());
        self.request_commits();
        self.pending = true;
    }
}
