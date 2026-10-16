//! The remotes level: every remote of one repo, where the checked-out branch
//! stands against each, and the branches each one holds.

use super::delete::{Confirm, Target, git_words};
use super::{App, Level};
use crate::git::git_run;
use crate::git::load::{self, Remote};
use crate::tui::widgets::{Field, Form, Modal};

/// `c` or `e` while its form is up. `was` is the remote an edit started from,
/// None for a new one.
pub(super) struct RemoteForm {
    pub(super) form: Form,
    was: Option<Remote>,
}

/// Everything about a remote a filter can match: its name and where it lives.
fn haystack(r: &Remote) -> String {
    format!("{} {} {}", r.name, r.url, r.push_url)
}

impl App {
    pub(super) fn load_remotes(&mut self) {
        self.head = load::head_branch(&self.repo);
        self.remotes = load::load_remotes(&self.repo, self.head.as_deref());
    }

    pub(super) fn rescope_remotes(&mut self) {
        let query = &self.msel.query;
        let visible = (0..self.remotes.len())
            .filter(|&i| query.keeps(&haystack(&self.remotes[i])))
            .collect();
        self.msel.show(visible);
        if let Some(want) = self.msel.restore.take()
            && let Some(at) = self
                .msel
                .visible
                .iter()
                .position(|&i| self.remotes[i].name == want)
        {
            self.msel.restored_at(at);
        }
    }

    /// Show the selected remote's branches in the pane below. They are already
    /// loaded with every other branch, so this only narrows the list.
    pub(super) fn enter_remote(&mut self) {
        self.branch_remote = self.remote().map_or(String::new(), |r| r.name.clone());
        self.rescope_branches();
    }

    pub(super) fn remote(&self) -> Option<&Remote> {
        self.msel.idx().map(|i| &self.remotes[i])
    }

    /// Whether `p` has anything to send: a branch is checked out and the remote
    /// either lacks it or is behind it.
    pub(super) fn can_push(&self) -> bool {
        self.head.is_some()
            && self
                .remote()
                .is_some_and(|r| r.ahead_behind.is_none_or(|(a, _)| a > 0))
    }

    /// `p`: say what is about to happen, and push once that frame is drawn.
    pub(super) fn ask_push(&mut self) {
        let Some(r) = self.remote() else {
            return;
        };
        let name = r.name.clone();
        let Some(branch) = self.head.clone() else {
            self.modal = Some(Modal::new(
                "not on a branch",
                "HEAD is detached, so there is no branch to push. Check one out, then `p` again.",
            ));
            return;
        };
        if !self.can_push() {
            self.set_status(format!("nothing to push: {name}/{branch} has it all"));
            return;
        }
        self.set_status(format!("pushing {branch} to {name}…"));
        self.push_next = Some(name);
    }

    /// The push `p` asked for, then the remotes and branches read again so the
    /// counts show it.
    pub(super) fn push(&mut self, remote: &str) {
        let branch = self.head.clone().unwrap_or_default();
        if let Err(words) = load::push_branch(&self.repo, remote) {
            self.note = None;
            let words = git_words(&words);
            self.modal = Some(Modal::new(
                format!("could not push {branch} to {remote}"),
                words,
            ));
            return;
        }
        self.set_status(format!("pushed {branch} to {remote}"));
        self.reread_remotes(Some(remote.to_string()));
    }

    /// `s` and `S` on a remote: fetch and prune that one, and with `pull`
    /// fast-forward the checked-out branch to its copy there, which for a fork
    /// is how `upstream`'s new commits come in.
    pub(super) fn sync_remote(&mut self, pull: bool) {
        let Some(name) = self.remote().map(|r| r.name.clone()) else {
            return;
        };
        if let Err(words) = load::fetch_remote(&self.repo, &name) {
            self.note = None;
            let words = git_words(&words);
            self.modal = Some(Modal::new(format!("could not sync {name}"), words));
            return;
        }
        let note = match (&self.head, pull) {
            (_, false) => Ok(format!("synced {name}")),
            (None, true) => Ok(format!("synced {name}; not on a branch to pull into")),
            (Some(branch), true) => {
                let rev = format!("refs/remotes/{name}/{branch}");
                if crate::git::git_capture(&self.repo, &["rev-parse", "--verify", "-q", &rev])
                    .is_none()
                {
                    Ok(format!("synced {name}; it has no {branch} to pull"))
                } else {
                    match load::fast_forward_to(&self.repo, &rev) {
                        Ok(0) => Ok(format!("synced {name}; {branch} already up to date")),
                        Ok(1) => Ok(format!("synced {name}; pulled 1 commit")),
                        Ok(n) => Ok(format!("synced {name}; pulled {n} commits")),
                        Err(words) => Err(words),
                    }
                }
            }
        };
        match note {
            Ok(text) => self.set_status(text),
            Err(words) => self.set_failed(format!(
                "fetched, but not pulled: {}",
                crate::git::first_line(&words)
            )),
        }
        self.reread_remotes(Some(name));
    }

    /// Read the remotes and their branches again, the cursor on `keep`.
    pub(super) fn reread_remotes(&mut self, keep: Option<String>) {
        self.unpushed_for.clear();
        self.load_remotes();
        self.msel.restore = keep;
        self.rescope_remotes();
        self.bsel.restore = self.bsel.idx().map(|i| self.branches[i].name.clone());
        self.request_branches();
        self.pending = true;
    }

    /// `c`: an empty form for a new remote.
    pub(super) fn create_remote(&mut self) {
        self.form = Some(RemoteForm {
            form: Form::new("new remote", fields("", "")),
            was: None,
        });
    }

    /// `e`: the selected remote's name and URL, to change either.
    pub(super) fn edit_remote(&mut self) {
        let Some(r) = self.remote() else {
            return;
        };
        let form = Form::new(format!("edit {}", r.name), fields(&r.name, &r.url));
        self.form = Some(RemoteForm {
            form,
            was: Some(r.clone()),
        });
    }

    /// The form's Enter on its last field. A refusal leaves the form up under
    /// git's words, so a typo costs one fix rather than the whole form again.
    pub(super) fn submit_remote(&mut self) {
        let Some(f) = &self.form else {
            return;
        };
        let (name, url) = (f.form.value(0).to_string(), f.form.value(1).to_string());
        let was = f.was.as_ref().map(|w| (w.name.clone(), w.url.clone()));
        let steps: Vec<Vec<&str>> = match &was {
            None => vec![vec!["remote", "add", &name, &url]],
            Some((was_name, was_url)) => {
                let mut steps = Vec::new();
                if *was_name != name {
                    steps.push(vec!["remote", "rename", was_name, &name]);
                }
                if *was_url != url {
                    steps.push(vec!["remote", "set-url", &name, &url]);
                }
                steps
            }
        };
        let done = if was.is_some() { "saved" } else { "added" };
        for args in &steps {
            let (ok, out) = git_run(&self.repo, args);
            if !ok {
                let words = git_words(&out);
                self.modal = Some(Modal::new(format!("could not save `{name}`"), words));
                return;
            }
        }
        self.form = None;
        self.set_status(format!("{done} {name}"));
        self.reread_remotes(Some(name));
    }

    /// `d` on a remote: the plain gate, since only this clone's config and
    /// tracking refs go and `c` with the same URL brings them back.
    pub(super) fn ask_delete_remote(&mut self) {
        let Some(r) = self.remote() else {
            return;
        };
        self.confirm = Some(Confirm::new(Target::Remote {
            name: r.name.clone(),
            url: r.url.clone(),
        }));
    }

    /// The gate's Yes.
    pub(super) fn delete_remote(&mut self, target: &Target) {
        let Target::Remote { name, .. } = target else {
            return;
        };
        let (ok, out) = git_run(&self.repo, &["remote", "remove", name]);
        if !ok {
            let words = git_words(&out);
            self.modal = Some(Modal::new(format!("could not remove `{name}`"), words));
            return;
        }
        self.set_status(format!("removed {name}"));
        let at = self.msel.cur;
        let next = self
            .msel
            .visible
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|b| self.msel.visible.get(b)))
            .map(|&i| self.remotes[i].name.clone());
        self.reread_remotes(next);
    }

    /// The local branches that track a branch of `remote`.
    pub(super) fn tracking(&self, remote: &str) -> usize {
        let prefix = format!("{remote}/");
        self.branches
            .iter()
            .filter(|b| !b.remote && b.upstream.starts_with(&prefix))
            .count()
    }

    /// Whether the branch pane is a remote's: `iremote` keeps it to the branches
    /// of the remote under the cursor.
    pub(super) fn remote_branches(&self) -> bool {
        self.start == Level::Remotes
    }
}

fn fields(name: &str, url: &str) -> Vec<Field> {
    vec![
        Field::new("name", "such as upstream", true).with(name),
        Field::new("url", "git@example.com:user/app.git, or a path", true).with(url),
    ]
}
