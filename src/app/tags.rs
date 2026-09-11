//! The tags level: one repo's tags marked against its remote, and the commits
//! each one added since the tag before it.

use super::delete::{Confirm, Target};
use super::{App, COMMITS_PER_BRANCH, Level};
use crate::git::git_run;
use crate::git::load::{self, RemoteTags, Tag, TagState};
use crate::tui::widgets::Modal;
use std::collections::HashSet;

/// Which slice of the tags the top pane shows.
#[derive(Clone, Copy, PartialEq)]
pub enum Scope {
    Local,
    Remote,
}

/// The same two tabs `ibranch` opens with, in the same places. No `all`:
/// `local` is already every tag in this clone.
pub const SCOPES: [Scope; 2] = [Scope::Local, Scope::Remote];
pub const DEFAULT_SCOPE: usize = 0;

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::Remote => "remote",
        }
    }

    /// `local` is the tags in this clone and `remote` the ones on the remote,
    /// as the same tabs mean in `ibranch`; the marks say which are only on one
    /// side.
    fn keeps(self, t: &Tag) -> bool {
        match self {
            Scope::Local => t.state != TagState::Remote,
            Scope::Remote => matches!(
                t.state,
                TagState::Pushed | TagState::Remote | TagState::Differs
            ),
        }
    }
}

impl TagState {
    pub fn label(self) -> &'static str {
        match self {
            TagState::Unknown => "",
            TagState::Local => "not pushed",
            TagState::Pushed => "pushed",
            TagState::Differs => "differs",
            TagState::Remote => "remote only",
        }
    }
}

/// What the commits under a tag are counted from.
pub(super) enum Since {
    /// `describe` has not answered yet.
    Asking,
    /// The tag before it on the same line of history.
    Tag(String),
    /// Nothing before it, so the pane holds its whole history.
    Start,
}

/// Everything about a tag a filter can match: its name, where it stands, when
/// it was made and what it says.
fn haystack(t: &Tag) -> String {
    format!("{} {} {} {}", t.name, t.state.label(), t.date, t.subject)
}

impl App {
    pub(super) fn load_tags(&mut self) {
        self.tags = load::load_tags(&self.repo);
    }

    /// Ask the remote for its tags. The list is up without them and gains its
    /// marks when the answer lands - at once, in a repo that tracks its tags,
    /// from git's copy, which `show_copy` draws while the remote is asked again.
    /// Not straight after `t`: the copy is still empty then, and would mark
    /// every tag as not pushed until the first fetch fills it.
    pub(super) fn request_remote_tags(&mut self, show_copy: bool) {
        let seq = self.rfeed.issue();
        self.tracked = self
            .tag_remote
            .as_deref()
            .is_some_and(|r| load::tracks_tags(&self.repo, r));
        match (&self.tag_remote, self.tracked && show_copy) {
            (Some(remote), true) => {
                let tags = load::tag_copy(&self.repo, remote);
                let remote = remote.clone();
                let answer = RemoteTags::Answered {
                    remote,
                    tags,
                    fresh: false,
                };
                self.apply_remote(answer);
            }
            _ => self.remote = None,
        }
        load::stream_remote_tags(
            self.repo.clone(),
            self.tag_remote.clone(),
            self.tracked,
            seq,
            self.tx.clone(),
        );
    }

    /// Whether `t` has anything to offer: a remote whose tags git is not yet
    /// keeping a copy of.
    pub(super) fn can_track(&self) -> bool {
        self.tag_remote.is_some() && !self.tracked
    }

    /// `t`: the offer to keep a copy of the remote's tags, opening on Yes since
    /// nothing is lost either way. A repo that already has the copy, or has no
    /// remote to copy from, has nothing to offer.
    pub(super) fn offer_tracking(&mut self) {
        if let (Some(remote), true) = (&self.tag_remote, self.can_track()) {
            self.confirm = Some(Confirm::new(Target::Track(remote.clone())));
        }
    }

    /// `t`, answered Yes: add the rule, then fetch once to fill the copy.
    pub(super) fn start_tracking(&mut self, remote: &str) {
        let key = format!("remote.{remote}.fetch");
        let (ok, out) = git_run(
            &self.repo,
            &["config", "--add", &key, &load::tag_rule(remote)],
        );
        if !ok {
            self.modal = Some(Modal::new("could not add the rule", out));
            return;
        }
        self.set_status(format!("tracking {remote}'s tags"));
        self.request_remote_tags(false);
    }

    /// Take the remote's answer, keeping the cursor on the tag it was on.
    pub(super) fn apply_remote(&mut self, answer: RemoteTags) {
        let keep = self.tag_name().map(str::to_string);
        self.remote = Some(answer);
        self.tsel.restore = keep.clone();
        self.mark_tags();
        if self.level == Level::Tags && self.tag_name() != keep.as_deref() {
            self.pending = true;
        }
    }

    /// Mark every local tag against what the remote said and add the ones only
    /// the remote has. With no remote at all nothing is pushed, which is what
    /// lets `d` treat those tags like any other that exists only here.
    pub(super) fn mark_tags(&mut self) {
        self.tags.retain(|t| t.state != TagState::Remote);
        let remote = self.remote.take();
        if let Some(RemoteTags::NoRemote) = &remote {
            for t in &mut self.tags {
                t.state = TagState::Local;
            }
        }
        if let Some(RemoteTags::Answered { tags, .. }) = &remote {
            for t in &mut self.tags {
                t.state = match tags.get(&t.name) {
                    None => TagState::Local,
                    Some(r) if r.object == t.object => TagState::Pushed,
                    Some(_) => TagState::Differs,
                };
            }
            let here: HashSet<&str> = self.tags.iter().map(|t| t.name.as_str()).collect();
            let mut only: Vec<Tag> = tags
                .iter()
                .filter(|(name, _)| !here.contains(name.as_str()))
                .map(|(name, r)| Tag {
                    name: name.clone(),
                    date: String::new(),
                    annotated: r.object != r.commit,
                    object: r.object.clone(),
                    commit: r.commit.clone(),
                    subject: String::new(),
                    state: TagState::Remote,
                })
                .collect();
            // Without their objects there is no date to sort by, so by name.
            only.sort_by(|a, b| b.name.cmp(&a.name));
            self.tags.extend(only);
        }
        self.remote = remote;
        self.rescope_tags();
    }

    pub(super) fn rescope_tags(&mut self) {
        let scope = SCOPES[self.tsel.scope];
        let query = &self.tsel.query;
        let visible = (0..self.tags.len())
            .filter(|&i| scope.keeps(&self.tags[i]) && query.keeps(&haystack(&self.tags[i])))
            .collect();
        self.tsel.show(visible);
        if let Some(want) = self.tsel.restore.take()
            && let Some(at) = self
                .tsel
                .visible
                .iter()
                .position(|&i| self.tags[i].name == want)
        {
            self.tsel.restored_at(at);
        }
    }

    /// Load the selected tag's own commits, which the pane below the tag list
    /// previews and the commits level then takes over.
    pub(super) fn enter_tag(&mut self) {
        let Some(i) = self.tsel.idx() else {
            self.commits.clear();
            self.csel.show(Vec::new());
            return;
        };
        self.ensure_unpushed();
        let t = &self.tags[i];
        // A tag only the remote has has no ref here, only the commit it names,
        // and that only if this clone already has it.
        let rev = if t.state == TagState::Remote {
            t.commit.clone()
        } else {
            format!("refs/tags/{}", t.name)
        };
        self.range_tag = t.name.clone();
        self.since = Since::Asking;
        self.log_args = vec![rev];
        self.limit = COMMITS_PER_BRANCH;
        self.request_commits();
    }

    /// Which commits a tag's list holds, in words: `new in v0.6.5 since
    /// v0.6.4`, or `everything up to v0.1.0` for the oldest tag.
    pub(super) fn range_label(&self) -> String {
        let tag = &self.range_tag;
        match &self.since {
            Since::Asking => format!("new in {tag}"),
            Since::Tag(prev) => format!("new in {tag} since {prev}"),
            Since::Start => format!("everything up to {tag}"),
        }
    }

    pub(super) fn tag_name(&self) -> Option<&str> {
        self.tsel.idx().map(|i| self.tags[i].name.as_str())
    }
}
