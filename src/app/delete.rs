//! `d`: deleting the branch or tag under the cursor, behind a gate.
//!
//! What is finished or cheap to recreate - a branch whose upstream is gone, a
//! tag no remote has - gets the Yes/No gate. What cannot be taken back and that
//! somebody else depends on or that exists nowhere else - anything on a remote,
//! commits no remote has - gets the typed gate, which only the thing's name
//! opens. Anything else is refused with a box that says why, so `d` never
//! silently does nothing.

use super::{App, Sel};
use crate::git::load::{self, RemoteTags, TagState};
use crate::git::{git_capture, git_run};
use crate::tui::widgets::Modal;
use ratatui::style::Color;

/// How much of git's own complaint a box shows, so a long one cannot fill the
/// screen.
const GIT_WORDS: usize = 8;

/// What `u` puts back: the last ref `d` deleted from this clone, exactly as it
/// was - the object it named and, for a branch, the upstream `git branch -D`
/// forgets. Only for this clone: a delete on a remote has already reached
/// everyone who fetched since.
pub(super) struct Undo {
    pub(super) branch: bool,
    name: String,
    refname: String,
    sha: String,
    upstream: Option<(String, String)>,
}

/// A box the drill is waiting on an answer from.
pub(super) struct Confirm {
    pub(super) target: Target,
    /// Which button is lit. A delete opens on No, the offer to track on Yes.
    pub(super) yes: bool,
    /// What has been typed, for a target that has to be named to go ahead.
    pub(super) typed: String,
}

impl Confirm {
    pub(super) fn new(target: Target) -> Confirm {
        Confirm {
            yes: matches!(target, Target::Track(_)),
            typed: String::new(),
            target,
        }
    }
}

pub(super) enum Target {
    /// A local branch. `lost` counts its commits no remote has; `landed` is
    /// whether their changes are in the remote's main line anyway (`trunk`),
    /// as a squash or rebase merge leaves them. Commits whose changes are not
    /// are work that exists nowhere else, and put it behind the typed gate.
    Branch {
        name: String,
        lost: usize,
        landed: bool,
        trunk: String,
    },
    /// A tag no remote has this version of.
    Tag {
        name: String,
        differs: bool,
    },
    /// A tag deleted on the remote, for everyone, and here too when `here`.
    RemoteTag {
        remote: String,
        name: String,
        here: bool,
    },
    RemoteBranch {
        remote: String,
        branch: String,
    },
    /// `t`: add the fetch rule that has git keep a copy of this remote's tags.
    Track(String),
}

impl Target {
    /// Whether it takes the typed gate: something that cannot be taken back
    /// and that other people depend on or that exists nowhere else.
    pub(super) fn typed(&self) -> bool {
        match self {
            Target::Branch { lost, landed, .. } => *lost > 0 && !landed,
            Target::RemoteTag { .. } | Target::RemoteBranch { .. } => true,
            Target::Tag { .. } | Target::Track(_) => false,
        }
    }

    pub(super) fn title(&self) -> String {
        match self {
            Target::Branch {
                lost,
                landed: false,
                name,
                ..
            } if *lost > 0 => format!("delete {name} here, with its commits?"),
            Target::Branch { .. } => "delete this branch?".to_string(),
            Target::Tag { .. } => "delete this tag?".to_string(),
            Target::RemoteTag {
                remote,
                name,
                here: true,
            } => format!("delete {name} on {remote} and here?"),
            Target::RemoteTag { remote, name, .. } => format!("delete {name} on {remote}?"),
            Target::RemoteBranch { remote, branch } => format!("delete {branch} on {remote}?"),
            Target::Track(_) => "show push marks instantly?".to_string(),
        }
    }

    /// What the box names, which is also what a typed gate wants typed.
    pub(super) fn name(&self) -> String {
        match self {
            Target::Branch { name, .. }
            | Target::Tag { name, .. }
            | Target::RemoteTag { name, .. } => name.clone(),
            Target::RemoteBranch { branch, .. } => branch.clone(),
            Target::Track(remote) => {
                format!("git keeps a copy of {remote}'s tags, like it does its branches")
            }
        }
    }

    pub(super) fn note(&self) -> Option<String> {
        match self {
            Target::Branch { lost: 0, .. } => {
                Some(
                "no commits of its own: everything in it is already on the remote, so only the name goes"
                    .to_string(),
            )
            }
            Target::Branch {
                landed: true,
                trunk,
                ..
            } => Some(format!(
                "its changes are all in {trunk} already, so only the name goes"
            )),
            Target::Branch { lost, .. } => Some(if *lost == 1 {
                "about to remove the branch and the commit only it holds".to_string()
            } else {
                format!("about to remove the branch and the {lost} commits only it holds")
            }),
            Target::Tag { differs: true, .. } => {
                Some("only yours goes: the remote's comes back with `git fetch --tags`".to_string())
            }
            Target::Tag { .. } => Some("the commit stays; only the tag goes".to_string()),
            Target::RemoteTag { remote, .. } => Some(format!(
                "everyone loses it on {remote}; clones that fetched it keep a copy"
            )),
            Target::RemoteBranch { remote, .. } => Some(format!(
                "everyone loses it on {remote}; your own branches are untouched"
            )),
            Target::Track(remote) => Some(format!(
                "adds to .git/config: fetch = {}\nundo: git config --unset remote.{remote}.fetch remote-tags",
                load::tag_rule(remote)
            )),
        }
    }

    pub(super) fn colour(&self) -> Color {
        match self {
            Target::Track(_) => Color::Cyan,
            _ => Color::Red,
        }
    }
}

impl App {
    /// `d` on a branch: the gate when its upstream is gone, which is the one
    /// sign a branch is finished; the typed gate when it takes commits nobody
    /// else has, or is the remote's; a refusal saying why for anything else.
    pub(super) fn ask_delete_branch(&mut self) {
        let Some(i) = self.bsel.idx() else {
            return;
        };
        let b = &self.branches[i];
        let name = b.name.clone();
        let (remote, branch) = if b.remote {
            b.name.split_once('/').unwrap_or(("origin", &b.name))
        } else {
            ("", b.name.as_str())
        };
        let target = if is_trunk(&self.repo, remote, branch) {
            let body = if matches!(branch, "main" | "master") {
                "You cannot delete main or master from here."
            } else {
                "You cannot delete the remote's default branch from here."
            };
            Err((
                format!("`{name}` is not something `d` deletes"),
                body.to_string(),
            ))
        } else if b.remote {
            Ok(Target::RemoteBranch {
                remote: remote.to_string(),
                branch: branch.to_string(),
            })
        } else if b.is_head {
            Err((
                format!("you are on `{name}`"),
                "git will not delete the branch that is checked out. Switch to another one, then `d` again."
                    .to_string(),
            ))
        } else if b.track.contains("gone") || !b.has_upstream || b.track.contains("ahead") {
            let lost = load::unique_commits(&self.repo, &name);
            let origin = b.upstream.split_once('/').map_or("origin", |(r, _)| r);
            let trunk = load::trunk(&self.repo, origin);
            let landed = lost > 0
                && trunk
                    .as_deref()
                    .is_some_and(|t| load::landed(&self.repo, &name, t));
            let trunk = trunk
                .map(|t| t.trim_start_matches("refs/remotes/").to_string())
                .unwrap_or_default();
            Ok(Target::Branch {
                name,
                lost,
                landed,
                trunk,
            })
        } else {
            let (remote, branch) = b.upstream.split_once('/').unwrap_or(("origin", &b.name));
            Err((
                format!("`{name}` is still on {remote}"),
                format!(
                    "Its upstream is alive, so it is not finished. Delete it on the remote first - \
                     `d` on `{remote}/{branch}` in the remote tab - then `s` to sync, and it shows as gone."
                ),
            ))
        };
        self.gate(target);
    }

    /// `d` on a tag: the gate when no remote has this version of it; the typed
    /// gate when it is on the remote, since deleting it only here would last
    /// until the next fetch brought it back.
    pub(super) fn ask_delete_tag(&mut self) {
        let Some(i) = self.tsel.idx() else {
            return;
        };
        let t = &self.tags[i];
        let name = t.name.clone();
        let remote = match &self.remote {
            Some(RemoteTags::Answered { remote, .. } | RemoteTags::Unreachable(remote)) => {
                remote.clone()
            }
            _ => "origin".to_string(),
        };
        let target = match (t.state, &self.remote) {
            (TagState::Local | TagState::Differs, _) => Ok(Target::Tag {
                differs: t.state == TagState::Differs,
                name,
            }),
            (TagState::Pushed | TagState::Remote, _) => Ok(Target::RemoteTag {
                here: t.state == TagState::Pushed,
                remote,
                name,
            }),
            (TagState::Unknown, Some(RemoteTags::Unreachable(_))) => Err((
                format!("{remote} could not be reached"),
                format!(
                    "There is no telling whether `{name}` exists anywhere else, so deleting it \
                     here could lose it. If you are sure: `git tag -d {name}`"
                ),
            )),
            (TagState::Unknown, _) => Err((
                format!("{remote} has not answered yet"),
                format!(
                    "Whether `{name}` is pushed is not known until it does. Try again in a moment."
                ),
            )),
        };
        self.gate(target);
    }

    fn gate(&mut self, target: Result<Target, (String, String)>) {
        match target {
            Ok(target) => self.confirm = Some(Confirm::new(target)),
            Err((title, body)) => self.modal = Some(Modal::new(title, body)),
        }
    }

    /// Run what the box was answered with. A delete on a remote goes over the
    /// network, so it runs after the frame that says so; the rest runs now.
    pub(super) fn delete(&mut self, target: Target) {
        match target {
            Target::Track(remote) => self.start_tracking(&remote),
            Target::RemoteTag { .. } | Target::RemoteBranch { .. } => {
                self.set_status(format!("deleting {} on the remote…", target.name()));
                self.delete_next = Some(target);
            }
            Target::Branch { .. } | Target::Tag { .. } => self.delete_here(target),
        }
    }

    /// A branch goes with `-D`: one whose upstream is gone often looks
    /// unmerged here after a squash or rebase merge, which `-d` refuses, and one
    /// with commits of its own was named by hand to go. What it named is kept
    /// first, so `u` can put it back.
    fn delete_here(&mut self, target: Target) {
        let (branch, name) = match &target {
            Target::Branch { name, .. } => (true, name.clone()),
            Target::Tag { name, .. } => (false, name.clone()),
            _ => return,
        };
        let refname = if branch {
            format!("refs/heads/{name}")
        } else {
            format!("refs/tags/{name}")
        };
        let Some(sha) = git_capture(&self.repo, &["rev-parse", &refname]) else {
            self.fail(&target, &format!("`{refname}` does not resolve any more"));
            return;
        };
        let upstream = branch.then(|| upstream_of(&self.repo, &name)).flatten();
        let args = if branch {
            ["branch", "-D", name.as_str()]
        } else {
            ["tag", "-d", name.as_str()]
        };
        let (ok, out) = git_run(&self.repo, &args);
        if !ok {
            self.fail(&target, &out);
            return;
        }
        let short: String = sha.chars().take(7).collect();
        self.set_status(format!("deleted {name} (was {short})"));
        self.undo = Some(Undo {
            branch,
            name,
            refname,
            sha,
            upstream,
        });
        self.reread(&target);
    }

    /// `u`: put back the last ref `d` deleted from this clone, on the object it
    /// named, and a branch's upstream with it. Refused when something has taken
    /// the name since, rather than moved onto it.
    pub(super) fn undo(&mut self) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        // An empty old value makes git refuse if the ref exists again.
        let (ok, out) = git_run(&self.repo, &["update-ref", &undo.refname, &undo.sha, ""]);
        if !ok {
            let words: Vec<&str> = out.lines().take(GIT_WORDS).collect();
            self.modal = Some(Modal::new(
                format!("could not put back `{}`", undo.name),
                words.join("\n"),
            ));
            return;
        }
        if let Some((remote, merge)) = &undo.upstream {
            let key = |k: &str| format!("branch.{}.{k}", undo.name);
            git_run(&self.repo, &["config", &key("remote"), remote]);
            git_run(&self.repo, &["config", &key("merge"), merge]);
        }
        self.set_status(format!("put back {}", undo.name));
        if undo.branch {
            self.bsel.restore = Some(undo.name);
            self.request_branches();
        } else {
            self.tsel.restore = Some(undo.name);
            self.load_tags();
            self.mark_tags();
        }
        self.pending = true;
    }

    /// The remote delete a typed gate let through, run once its frame is up.
    /// A tag that was also here goes here too, or the next fetch is all it would
    /// take to see it again.
    pub(super) fn delete_on_remote(&mut self, target: Target) {
        let (remote, refname) = match &target {
            Target::RemoteTag { remote, name, .. } => (remote, format!("refs/tags/{name}")),
            Target::RemoteBranch { remote, branch } => (remote, format!("refs/heads/{branch}")),
            _ => return,
        };
        if let Err(words) = load::push_delete(&self.repo, remote, &refname) {
            self.note = None;
            self.fail(&target, &words);
            return;
        }
        if let Target::RemoteTag {
            name, here: true, ..
        } = &target
        {
            let (ok, out) = git_run(&self.repo, &["tag", "-d", name]);
            if !ok {
                self.fail(&target, &out);
                return;
            }
        }
        self.set_status(format!("deleted {} on {remote}", target.name()));
        self.reread(&target);
    }

    fn fail(&mut self, target: &Target, words: &str) {
        let words: Vec<&str> = words.lines().take(GIT_WORDS).collect();
        self.modal = Some(Modal::new(
            format!("could not delete `{}`", target.name()),
            words.join("\n"),
        ));
    }

    /// Read the list the delete came from again, the cursor on the row that was
    /// next to the one that went.
    fn reread(&mut self, target: &Target) {
        match target {
            Target::Branch { .. } | Target::RemoteBranch { .. } => {
                self.bsel.restore = neighbour(&self.bsel, |i| self.branches[i].name.clone());
                self.request_branches();
            }
            Target::Tag { .. } => {
                self.tsel.restore = neighbour(&self.tsel, |i| self.tags[i].name.clone());
                self.load_tags();
                self.mark_tags();
            }
            Target::RemoteTag { name, .. } => {
                // The answer on screen still lists it, and would bring it back as
                // remote only until the remote is asked again.
                if let Some(RemoteTags::Answered { tags, .. }) = &mut self.remote {
                    tags.remove(name);
                }
                self.tsel.restore = neighbour(&self.tsel, |i| self.tags[i].name.clone());
                self.load_tags();
                self.mark_tags();
            }
            Target::Track(_) => {}
        }
        self.pending = true;
    }

    /// `s` and `S`: fetch and prune every remote, and with `pull` fast-forward
    /// the checked-out branch too, then read the branches again. Nothing here
    /// can lose work, so it asks nothing first.
    pub(super) fn sync(&mut self, pull: bool) {
        if let Err(words) = load::fetch_prune(&self.repo) {
            self.note = None;
            let words: Vec<&str> = words.lines().take(GIT_WORDS).collect();
            self.modal = Some(Modal::new("sync failed", words.join("\n")));
            return;
        }
        let note = if pull {
            match load::fast_forward(&self.repo) {
                Ok(Some(0)) => Ok("synced; already up to date".to_string()),
                Ok(Some(1)) => Ok("synced; pulled 1 commit".to_string()),
                Ok(Some(n)) => Ok(format!("synced; pulled {n} commits")),
                Ok(None) => Ok("synced; this branch tracks nothing to pull".to_string()),
                Err(words) => Err(words),
            }
        } else {
            Ok("synced".to_string())
        };
        match note {
            Ok(text) => self.set_status(text),
            // A branch with commits of its own cannot fast-forward: git's own
            // line says so, and the fetch above still happened.
            Err(words) => self.set_failed(format!(
                "fetched, but not pulled: {}",
                crate::git::first_line(&words)
            )),
        }
        // A pull can move the push state of every commit on screen.
        self.unpushed_for.clear();
        self.bsel.restore = self.bsel.idx().map(|i| self.branches[i].name.clone());
        self.request_branches();
        self.pending = true;
    }
}

/// The `branch.<name>.remote` and `.merge` that `git branch -D` removes with
/// the branch, so putting it back leaves it tracking what it tracked.
fn upstream_of(repo: &str, name: &str) -> Option<(String, String)> {
    let get = |k: &str| git_capture(repo, &["config", "--get", &format!("branch.{name}.{k}")]);
    Some((get("remote")?, get("merge")?))
}

/// `main`, `master`, or whatever the remote says its default branch is: the
/// branches `d` never deletes, here or on the remote. `remote` is empty for a
/// local branch, which is checked against `origin`'s default.
fn is_trunk(repo: &str, remote: &str, branch: &str) -> bool {
    if matches!(branch, "main" | "master") {
        return true;
    }
    let remote = if remote.is_empty() { "origin" } else { remote };
    load::default_branch(repo, remote).is_some_and(|d| d == branch)
}

/// The row a deleted one leaves the cursor on: the one after it, or the one
/// before when it was last.
fn neighbour(sel: &Sel, name: impl Fn(usize) -> String) -> Option<String> {
    let at = sel.cur;
    sel.visible
        .get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|b| sel.visible.get(b)))
        .map(|&i| name(i))
}
