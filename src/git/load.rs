//! Git loaders behind the interactive views: commits, the files a commit
//! touched, one file's raw diff, and which commits are unpushed.
//!
//! Every loader takes the repo to read, so one view can walk several repos in
//! the same session. `"."` is the current one.

use crate::git::{SEP, git_capture, git_capture_raw};
use crate::tui::highlight::{Blob, DiffContext};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

/// How many rows a streaming load hands over at a time: small enough that the
/// first screenful is up almost at once, large enough not to wake the drawing
/// loop once per line of a long history.
const BATCH: usize = 64;

/// A chunk of rows a background load produced. `seq` names the request it
/// answers, so rows for a branch the cursor has already left are dropped rather
/// than pasted in under the wrong one.
pub enum Batch {
    Branches {
        seq: u64,
        rows: Vec<Branch>,
        done: bool,
    },
    Commits {
        seq: u64,
        rows: Vec<Commit>,
        done: bool,
    },
    Files {
        seq: u64,
        rows: Vec<FileEntry>,
        done: bool,
    },
    RemoteTags {
        seq: u64,
        answer: RemoteTags,
    },
    /// The tag a tag's commits are counted from, None for the oldest tag.
    /// Arrives ahead of those commits, under the same `seq`.
    TagBase {
        seq: u64,
        prev: Option<String>,
    },
}

pub struct Branch {
    pub is_head: bool,
    pub remote: bool,
    /// `main`, or `origin/main` for a remote one.
    pub name: String,
    /// The full ref, which is what git is handed: a bare name is read as the
    /// tag when a tag shares it.
    pub refname: String,
    pub rel: String,
    pub author: String,
    pub has_upstream: bool,
    /// `origin/feat/x`, or empty when it tracks nothing.
    pub upstream: String,
    /// Raw `%(upstream:track)`: "", "[gone]", "[ahead 2, behind 1]", …
    pub track: String,
}

pub struct Commit {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub committer: String,
    pub refs: Vec<RefLabel>,
    pub subject: String,
}

/// A ref pointing at a commit, named the way `git log --decorate` names it.
pub struct RefLabel {
    pub kind: RefKind,
    pub name: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefKind {
    /// The checked-out branch.
    Head,
    /// `HEAD` itself, when it names no branch.
    Detached,
    Branch,
    Remote,
    Tag,
}

impl RefLabel {
    /// What git prints for it: `HEAD -> main`, `origin/main`, `tag: v1`.
    pub fn text(&self) -> String {
        match self.kind {
            RefKind::Head => format!("HEAD -> {}", self.name),
            RefKind::Detached => "HEAD".to_string(),
            RefKind::Branch | RefKind::Remote => self.name.clone(),
            RefKind::Tag => format!("tag: {}", self.name),
        }
    }
}

/// The labels that fit in `room` columns once drawn as `(a, b, +2) `: whole
/// labels in git's order, then how many were left off. The first is always
/// kept, cut short with `…` when it alone is too wide, so a commit never looks
/// like it has no refs.
pub fn fit_refs(refs: &[RefLabel], room: usize) -> (Vec<(RefKind, String)>, usize) {
    let more = |left: usize| {
        if left == 0 {
            0
        } else {
            3 + left.to_string().len()
        }
    };
    let mut kept = Vec::new();
    let mut used = 3;
    for (i, r) in refs.iter().enumerate() {
        let text = r.text();
        let sep = if i == 0 { 0 } else { 2 };
        let len = text.chars().count();
        if used + sep + len + more(refs.len() - i - 1) > room {
            if i == 0 {
                let fit = room.saturating_sub(used + more(refs.len() - 1)).max(2);
                let cut: String = text.chars().take(fit - 1).collect();
                kept.push((r.kind, format!("{cut}…")));
            }
            break;
        }
        used += sep + len;
        kept.push((r.kind, text));
    }
    let hidden = refs.len() - kept.len();
    (kept, hidden)
}

/// The log flag `parse_refs` needs: full names are what tell a local branch
/// called `origin/x` from the remote's `x`.
pub const DECORATE: &str = "--decorate=full";

/// `%D` back into labels. Only branches, remote branches and tags are kept:
/// the stash and `itag`'s copy of remote tags are not something a log reader
/// asked about, and `origin/HEAD` only repeats the remote's default branch.
pub fn parse_refs(d: &str) -> Vec<RefLabel> {
    d.split(", ")
        .filter_map(|r| {
            let (kind, name) = if let Some(b) = r.strip_prefix("HEAD -> refs/heads/") {
                (RefKind::Head, b)
            } else if r == "HEAD" {
                (RefKind::Detached, r)
            } else if let Some(b) = r.strip_prefix("refs/heads/") {
                (RefKind::Branch, b)
            } else if let Some(t) = r.strip_prefix("tag: refs/tags/") {
                (RefKind::Tag, t)
            } else if let Some(b) = r.strip_prefix("refs/remotes/")
                && !b.ends_with("/HEAD")
            {
                (RefKind::Remote, b)
            } else {
                return None;
            };
            Some(RefLabel {
                kind,
                name: name.to_string(),
            })
        })
        .collect()
}

pub struct FileEntry {
    pub status: char,
    pub path: String,
}

pub struct Tag {
    pub name: String,
    pub date: String,
    pub annotated: bool,
    /// What the ref itself names: the tag object when annotated, the commit
    /// otherwise. Two sides agree on a tag only when this matches.
    pub object: String,
    /// The commit underneath, which is where the tag's log starts.
    pub commit: String,
    /// The tag's own message when annotated, its commit's subject otherwise.
    pub subject: String,
    pub state: TagState,
}

/// Where a tag stands against the remote.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagState {
    /// The remote has not answered yet, or could not be asked.
    Unknown,
    /// Here and not on the remote.
    Local,
    /// On both, naming the same object.
    Pushed,
    /// On both, naming different objects: one side tagged again.
    Differs,
    /// On the remote and not here, which a fetch has not brought in yet.
    Remote,
}

/// A remote's side of a tag, as `git ls-remote` reports it.
#[derive(Default)]
pub struct RemoteTag {
    pub object: String,
    pub commit: String,
}

/// What asking the remote for its tags came back with.
pub enum RemoteTags {
    NoRemote,
    /// Offline, or it wanted a credential typed in. Names the remote asked.
    Unreachable(String),
    Answered {
        remote: String,
        tags: HashMap<String, RemoteTag>,
        /// False when this is git's copy as of the last fetch rather than what
        /// the remote just said: shown while it is asked again, and kept when
        /// it cannot be.
        fresh: bool,
    },
}

/// Hashes reachable from local branches but on **no** remote - i.e. commits you
/// haven't pushed anywhere. A commit not in this set is on some remote (pushed).
/// Empty when the repo has no remotes (nothing is "pushed").
pub fn load_unpushed(repo: &str) -> HashSet<String> {
    git_capture(repo, &["rev-list", "--branches", "--not", "--remotes"])
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// The newest commit hash this log would produce, or empty when it has none.
/// One commit is all it takes to tell an empty log from a full one, and it is
/// the one thing a caller cannot wait for the stream to answer.
pub fn first_commit(repo: &str, extra: &[&str]) -> String {
    let mut args = vec!["log", "-n", "1", "--pretty=format:%H"];
    args.extend_from_slice(extra);
    git_capture(repo, &args).unwrap_or_default()
}

/// What a commit row has no room for: both people, its parents, the whole
/// message.
pub struct CommitDetails {
    /// `name <email>`, and when, both dated and relative.
    pub author: String,
    pub authored: String,
    pub committer: String,
    pub committed: String,
    /// Short hashes; more than one is a merge.
    pub parents: Vec<String>,
    pub message: String,
}

pub fn commit_details(repo: &str, rev: &str) -> Option<CommitDetails> {
    let fmt = format!("--format=%an <%ae>{SEP}%ad (%ar){SEP}%cn <%ce>{SEP}%cd (%cr){SEP}%p{SEP}%B");
    let out = git_capture(
        repo,
        &["show", "-s", "--date=format:%Y-%m-%d %H:%M", &fmt, rev],
    )?;
    let mut f = out.split(SEP);
    Some(CommitDetails {
        author: f.next()?.to_string(),
        authored: f.next()?.to_string(),
        committer: f.next()?.to_string(),
        committed: f.next()?.to_string(),
        parents: f.next()?.split_whitespace().map(str::to_string).collect(),
        message: f.next().unwrap_or("").trim().to_string(),
    })
}

/// An annotated tag's tagger (`name <email>`) and whole message; None for a
/// lightweight one, which has neither.
pub fn tag_details(repo: &str, name: &str) -> Option<(String, String)> {
    let fmt = format!("--format=%(taggername) %(taggeremail){SEP}%(contents)");
    let tag = format!("refs/tags/{name}");
    let out = git_capture(repo, &["for-each-ref", &fmt, &tag])?;
    let (tagger, message) = out.split_once(SEP)?;
    let tagger = tagger.trim();
    (!tagger.is_empty()).then(|| (tagger.to_string(), message.trim().to_string()))
}

/// Every remote and the URL it fetches from.
pub fn remote_urls(repo: &str) -> Vec<(String, String)> {
    let out = git_capture(repo, &["remote", "-v"]).unwrap_or_default();
    out.lines()
        .filter(|l| l.ends_with("(fetch)"))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.to_string(), f.next()?.to_string()))
        })
        .collect()
}

/// One `--pretty=format:` line back into a `Commit`.
fn parse_commit(line: &str) -> Option<Commit> {
    let mut f = line.split(SEP);
    Some(Commit {
        hash: f.next()?.to_string(),
        short: f.next()?.to_string(),
        date: f.next()?.to_string(),
        committer: f.next()?.to_string(),
        refs: parse_refs(f.next()?),
        subject: f.next().unwrap_or("").to_string(),
    })
}

/// The `for-each-ref` format both branch readers ask for.
fn branch_format() -> String {
    format!(
        "--format=%(HEAD){SEP}%(refname){SEP}%(committerdate:relative){SEP}%(authorname){SEP}%(upstream){SEP}%(upstream:track)"
    )
}

/// One `for-each-ref` line back into a `Branch`. Names are cut from the full
/// refs rather than taken from `:short`, which answers `heads/v1` when a tag is
/// also called `v1`.
fn parse_branch(line: &str) -> Option<Branch> {
    let mut f = line.split(SEP);
    let head = f.next()?;
    let refname = f.next()?;
    let rel = f.next().unwrap_or("").to_string();
    let author = f.next().unwrap_or("").to_string();
    let upstream = f.next().unwrap_or("");
    let track = f.next().unwrap_or("").to_string();
    // Skip the symbolic `refs/remotes/*/HEAD` alias - it's noise.
    if refname.ends_with("/HEAD") {
        return None;
    }
    let name = refname
        .strip_prefix("refs/heads/")
        .or_else(|| refname.strip_prefix("refs/remotes/"))?;
    Some(Branch {
        is_head: head.trim() == "*",
        remote: refname.starts_with("refs/remotes/"),
        name: name.to_string(),
        refname: refname.to_string(),
        rel,
        author,
        has_upstream: !upstream.is_empty(),
        upstream: upstream
            .strip_prefix("refs/remotes/")
            .unwrap_or(upstream)
            .to_string(),
        track,
    })
}

/// Every branch (local + remote-tracking) with its push state, newest first.
/// The blocking read, for the one caller that has to know before the screen is
/// up whether the repo has any branches at all.
pub fn load_branches(repo: &str) -> Vec<Branch> {
    let fmt = branch_format();
    git_capture(
        repo,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            &fmt,
            "refs/heads",
            "refs/remotes",
        ],
    )
    .map(|out| out.lines().filter_map(parse_branch).collect())
    .unwrap_or_default()
}

/// The same read, off the input path, so walking a list of repos never waits on
/// one of them.
pub fn stream_branches(repo: String, seq: u64, latest: Arc<AtomicU64>, tx: Sender<Batch>) {
    thread::spawn(move || {
        let fmt = branch_format();
        let args = [
            "for-each-ref",
            "--sort=-committerdate",
            fmt.as_str(),
            "refs/heads",
            "refs/remotes",
        ];
        stream_rows(
            &repo,
            &args,
            seq,
            &latest,
            &tx,
            parse_branch,
            |seq, rows, done| Batch::Branches { seq, rows, done },
        );
    });
}

/// Spawn a git query and hand its rows over in batches as they are parsed,
/// stopping the moment `latest` names a newer request. Shared by every
/// streaming reader: what differs between them is only the arguments, the line
/// parser, and which `Batch` the rows go into.
fn stream_rows<T, P, W>(
    repo: &str,
    args: &[&str],
    seq: u64,
    latest: &Arc<AtomicU64>,
    tx: &Sender<Batch>,
    parse: P,
    wrap: W,
) where
    P: Fn(&str) -> Option<T>,
    W: Fn(u64, Vec<T>, bool) -> Batch,
{
    let spawned = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        let _ = tx.send(wrap(seq, Vec::new(), true));
        return;
    };
    let Some(out) = child.stdout.take() else {
        let _ = child.wait();
        return;
    };

    let mut rows: Vec<T> = Vec::with_capacity(BATCH);
    let mut whole = true;
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        if latest.load(Ordering::Relaxed) != seq {
            whole = false;
            break;
        }
        if let Some(row) = parse(&line) {
            rows.push(row);
        }
        if rows.len() >= BATCH {
            let batch = std::mem::take(&mut rows);
            if tx.send(wrap(seq, batch, false)).is_err() {
                whole = false;
                break;
            }
            rows.reserve(BATCH);
        }
    }
    // Abandoned: stop git rather than let it read a history to the end for a
    // pane that is already showing something else.
    if !whole {
        let _ = child.kill();
    }
    let _ = child.wait();
    if whole {
        let _ = tx.send(wrap(seq, rows, true));
    }
}

/// Run `git log` on a background thread, handing rows over as they are parsed
/// so the pane fills while the user keeps moving. The walk stops early the
/// moment `latest` stops naming this request: finishing one nobody will look at
/// only starves the one they are waiting for, and on a slow filesystem that is
/// the whole difference.
pub fn stream_commits(
    repo: String,
    args: Vec<String>,
    limit: usize,
    seq: u64,
    latest: Arc<AtomicU64>,
    tx: Sender<Batch>,
) {
    thread::spawn(move || log_rows(&repo, &args, limit, seq, &latest, &tx));
}

/// A tag's own commits: what `rev` holds that the tag before it on the same
/// line of history does not, which is what went into that release. The oldest
/// tag has no tag under it, so it shows its whole history.
pub fn stream_tag_commits(
    repo: String,
    rev: String,
    limit: usize,
    seq: u64,
    latest: Arc<AtomicU64>,
    tx: Sender<Batch>,
) {
    thread::spawn(move || {
        let parent = format!("{rev}^");
        let before = git_capture(&repo, &["describe", "--tags", "--abbrev=0", &parent]);
        let mut args = vec![rev];
        if let Some(prev) = &before {
            args.push(format!("^refs/tags/{prev}"));
        }
        let _ = tx.send(Batch::TagBase { seq, prev: before });
        log_rows(&repo, &args, limit, seq, &latest, &tx);
    });
}

/// The `git log` both commit streams run, `args` naming what it walks.
fn log_rows(
    repo: &str,
    args: &[String],
    limit: usize,
    seq: u64,
    latest: &Arc<AtomicU64>,
    tx: &Sender<Batch>,
) {
    let n = limit.to_string();
    let fmt = format!("--pretty=format:%H{SEP}%h{SEP}%ad{SEP}%cn{SEP}%D{SEP}%s");
    let mut argv = vec![
        "log",
        "-n",
        n.as_str(),
        "--date=format:%Y-%m-%d %H:%M",
        DECORATE,
        fmt.as_str(),
    ];
    argv.extend(args.iter().map(String::as_str));
    stream_rows(
        repo,
        &argv,
        seq,
        latest,
        tx,
        parse_commit,
        |seq, rows, done| Batch::Commits { seq, rows, done },
    );
}

/// Every local tag, newest first.
pub fn load_tags(repo: &str) -> Vec<Tag> {
    let fmt = format!(
        "--format=%(refname){SEP}%(creatordate:format:%Y-%m-%d %H:%M){SEP}%(objecttype){SEP}%(objectname){SEP}%(*objectname){SEP}%(contents:subject)"
    );
    // The last `--sort` is the primary key: tags made in the same second fall
    // back to version order rather than to the name read as plain text.
    let args = [
        "for-each-ref",
        "--sort=-v:refname",
        "--sort=-creatordate",
        &fmt,
        "refs/tags",
    ];
    git_capture(repo, &args)
        .map(|out| out.lines().filter_map(parse_tag).collect())
        .unwrap_or_default()
}

/// One `for-each-ref` line back into a `Tag`. The name comes from `%(refname)`
/// because `:short` answers `tags/v1` when a branch is also called `v1`.
fn parse_tag(line: &str) -> Option<Tag> {
    let mut f = line.split(SEP);
    let name = f.next()?.strip_prefix("refs/tags/")?.to_string();
    let date = f.next()?.to_string();
    let annotated = f.next()? == "tag";
    let object = f.next()?.to_string();
    let peeled = f.next()?;
    let commit = if peeled.is_empty() {
        object.clone()
    } else {
        peeled.to_string()
    };
    Some(Tag {
        name,
        date,
        annotated,
        object,
        commit,
        subject: f.next().unwrap_or("").to_string(),
        state: TagState::Unknown,
    })
}

/// The remote a repo's tags are compared against: `origin` when there is one,
/// since that is where tags get pushed, else whichever remote git lists first.
pub fn tag_remote(repo: &str) -> Option<String> {
    let remotes = git_capture(repo, &["remote"]).unwrap_or_default();
    remotes
        .lines()
        .find(|r| *r == "origin")
        .or_else(|| remotes.lines().next())
        .map(str::to_string)
}

/// The fetch rule that has git keep a copy of `remote`'s tags, the way the rule
/// every clone gets keeps `origin/main`. Outside `refs/remotes/` on purpose:
/// under it every tag would list as a remote branch in `git branch -r`, and a
/// branch called `tags/x` would land on the same ref as a tag called `x`.
pub fn tag_rule(remote: &str) -> String {
    format!("+refs/tags/*:refs/remote-tags/{remote}/*")
}

/// Whether this repo already has `tag_rule` for `remote`, so git keeps the copy
/// current on every fetch and push.
pub fn tracks_tags(repo: &str, remote: &str) -> bool {
    let key = format!("remote.{remote}.fetch");
    git_capture(repo, &["config", "--get-all", &key])
        .is_some_and(|rules| rules.lines().any(|r| r == tag_rule(remote)))
}

/// git's copy of `remote`'s tags, as of the last fetch or push.
pub fn tag_copy(repo: &str, remote: &str) -> HashMap<String, RemoteTag> {
    let prefix = format!("refs/remote-tags/{remote}/");
    let fmt = format!("--format=%(refname){SEP}%(objectname){SEP}%(*objectname)");
    git_capture(repo, &["for-each-ref", &fmt, &prefix])
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    let name = f.next()?.strip_prefix(&prefix)?.to_string();
                    let object = f.next()?.to_string();
                    let peeled = f.next().unwrap_or("");
                    let commit = if peeled.is_empty() {
                        object.clone()
                    } else {
                        peeled.to_string()
                    };
                    Some((name, RemoteTag { object, commit }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ask `remote` which tags it has, off the input path, since it is a network
/// round trip and the tag list is on screen long before it answers. A repo
/// that tracks its tags refreshes git's copy instead - a tags-only fetch that
/// touches no branch and none of the tags you made - and reads it back, so the
/// answer is kept for the next time; when that fetch fails the copy is still
/// what git last knew.
pub fn stream_remote_tags(
    repo: String,
    remote: Option<String>,
    tracked: bool,
    seq: u64,
    tx: Sender<Batch>,
) {
    thread::spawn(move || {
        let answer = match remote {
            None => RemoteTags::NoRemote,
            Some(remote) if tracked => refresh_copy(&repo, remote),
            Some(remote) => ask(&repo, remote),
        };
        let _ = tx.send(Batch::RemoteTags { seq, answer });
    });
}

fn refresh_copy(repo: &str, remote: String) -> RemoteTags {
    let rule = tag_rule(&remote);
    let fresh = unattended_git(repo)
        .args(["fetch", "--no-tags", "--prune", &remote, &rule])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    RemoteTags::Answered {
        tags: tag_copy(repo, &remote),
        remote,
        fresh,
    }
}

fn ask(repo: &str, remote: String) -> RemoteTags {
    let out = unattended_git(repo)
        .args(["ls-remote", "--tags", &remote])
        .output();
    match out {
        Ok(out) if out.status.success() => RemoteTags::Answered {
            remote,
            tags: parse_ls_remote(&String::from_utf8_lossy(&out.stdout)),
            fresh: true,
        },
        _ => RemoteTags::Unreachable(remote),
    }
}

/// `git fetch --all --prune`, through `unattended_git` since it goes over the
/// network with a TUI up. Pruning is the point as much as fetching: git only
/// marks a branch gone once its tracking ref is really absent. The error is
/// git's own words.
pub fn fetch_prune(repo: &str) -> Result<(), String> {
    let mut cmd = unattended_git(repo);
    cmd.args(["fetch", "--all", "--prune", "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let out = cmd
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Fast-forward the checked-out branch to its upstream, never merging:
/// `--ff-only` refuses whenever the branch has commits of its own, so nothing
/// here can be lost. Ok carries how many commits it moved by, None when the
/// branch tracks nothing; the error is git's own words.
pub fn fast_forward(repo: &str) -> Result<Option<usize>, String> {
    if git_capture(repo, &["rev-parse", "--abbrev-ref", "@{u}"]).is_none() {
        return Ok(None);
    }
    let before = git_capture(repo, &["rev-parse", "HEAD"]).unwrap_or_default();
    let (ok, out) = crate::git::git_run(repo, &["merge", "--ff-only", "--quiet", "@{u}"]);
    if !ok {
        return Err(out);
    }
    let moved = git_capture(repo, &["rev-list", "--count", &format!("{before}..HEAD")])
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    Ok(Some(moved))
}

/// Delete `refname` on `remote` (`git push <remote> :<refname>`), through
/// `unattended_git` since it goes over the network with a TUI up. The error is
/// git's own words - a protected tag or branch is refused by the server, and
/// what it said is the explanation.
pub fn push_delete(repo: &str, remote: &str, refname: &str) -> Result<(), String> {
    let mut cmd = unattended_git(repo);
    cmd.args(["push", remote, &format!(":{refname}")])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let out = cmd
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The remote's default branch as git last recorded it (`refs/remotes/<remote>/HEAD`,
/// set at clone), without its `<remote>/` prefix.
pub fn default_branch(repo: &str, remote: &str) -> Option<String> {
    let head = format!("refs/remotes/{remote}/HEAD");
    let target = git_capture(repo, &["symbolic-ref", "--short", &head])?;
    target
        .strip_prefix(&format!("{remote}/"))
        .map(str::to_string)
}

/// The remote's main line as a ref this clone has: its default branch, else
/// `main` or `master` when git never recorded one.
pub fn trunk(repo: &str, remote: &str) -> Option<String> {
    let names = default_branch(repo, remote)
        .into_iter()
        .chain(["main".to_string(), "master".to_string()]);
    names
        .map(|n| format!("refs/remotes/{remote}/{n}"))
        .find(|r| git_capture(repo, &["rev-parse", "--verify", "-q", r]).is_some())
}

/// Whether every change on `branch` is already in `trunk`: merging it in would
/// leave trunk's tree exactly as it is. That holds however the work got there,
/// by merge, squash, rebase or cherry-pick, which counting commits cannot tell,
/// since the last three give the same changes new ids. A conflict, or a
/// git older than 2.38 without `merge-tree --write-tree`, reads as not landed,
/// so the answer only ever errs towards asking for the name.
pub fn landed(repo: &str, branch: &str, trunk: &str) -> bool {
    let tip = format!("refs/heads/{branch}");
    let merged = git_capture(repo, &["merge-tree", "--write-tree", trunk, &tip]);
    let tree = format!("{trunk}^{{tree}}");
    let current = git_capture(repo, &["rev-parse", &tree]);
    matches!((merged, current), (Some(m), Some(c)) if m.lines().next() == Some(c.as_str()))
}

/// How many of a local branch's commits no remote has: what deleting it would
/// throw away for good.
pub fn unique_commits(repo: &str, branch: &str) -> usize {
    let tip = format!("refs/heads/{branch}");
    git_capture(repo, &["rev-list", "--count", &tip, "--not", "--remotes"])
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// `git ls-remote --tags` lists an annotated tag twice: the tag object, then
/// the commit under it as `<name>^{}`. A lightweight tag is only the first.
fn parse_ls_remote(out: &str) -> HashMap<String, RemoteTag> {
    let mut tags: HashMap<String, RemoteTag> = HashMap::new();
    for line in out.lines() {
        let Some((sha, name)) = line
            .split_once('\t')
            .and_then(|(sha, r)| Some((sha, r.strip_prefix("refs/tags/")?)))
        else {
            continue;
        };
        let (name, peeled) = match name.strip_suffix("^{}") {
            Some(name) => (name, true),
            None => (name, false),
        };
        let tag = tags.entry(name.to_string()).or_default();
        if peeled {
            tag.commit = sha.to_string();
        } else {
            tag.object = sha.to_string();
            if tag.commit.is_empty() {
                tag.commit = sha.to_string();
            }
        }
    }
    tags
}

/// `git -C <repo>` for a call that reaches a remote while a TUI owns the
/// terminal, where a password or passphrase prompt would be typed into a screen
/// that is not reading it: every prompt becomes a failure instead.
fn unattended_git(repo: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    // ssh asks on /dev/tty rather than stdin, so only its own flag stops it.
    // `GIT_SSH` names a program rather than a command line, with nowhere to
    // add a flag, so it is left alone.
    if std::env::var_os("GIT_SSH").is_none() {
        let ssh = std::env::var("GIT_SSH_COMMAND")
            .ok()
            .or_else(|| git_capture(repo, &["config", "core.sshCommand"]))
            .unwrap_or_else(|| "ssh".to_string());
        cmd.env(
            "GIT_SSH_COMMAND",
            format!("{ssh} -o BatchMode=yes -o ConnectTimeout=10"),
        );
    }
    cmd
}

/// The same read as `load_files`, off the input path. One `git show` is a
/// single small listing, so it arrives in one piece rather than in batches.
pub fn stream_files(repo: String, hash: String, paths: Vec<String>, seq: u64, tx: Sender<Batch>) {
    thread::spawn(move || {
        let spec: Vec<&str> = paths.iter().map(String::as_str).collect();
        let rows = load_files(&repo, &hash, &spec);
        let _ = tx.send(Batch::Files {
            seq,
            rows,
            done: true,
        });
    });
}

/// How a merge is diffed: against its first parent, as `commit_diff_ctx` reads it.
/// git's default combined diff lists a cleanly merged file and then shows no
/// hunks for it, which left the diff pane blank.
const FIRST_PARENT: &str = "--diff-merges=first-parent";

/// The files a commit touched, with their status (cheap - no diff content).
/// When `pathspec` is non-empty, only files matching it are returned (so a
/// path-filtered `ilog` shows just that file's change in each commit).
pub fn load_files(repo: &str, hash: &str, pathspec: &[&str]) -> Vec<FileEntry> {
    let mut args = vec!["show", "--name-status", "--format=", FIRST_PARENT, hash];
    if !pathspec.is_empty() {
        args.push("--");
        args.extend_from_slice(pathspec);
    }
    git_capture(repo, &args)
        .map(|out| {
            out.lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| {
                    let mut parts = line.split('\t');
                    let status = parts.next()?.chars().next()?;
                    // last field handles renames ("R100\told\tnew" -> new path)
                    let path = parts.next_back()?.to_string();
                    Some(FileEntry { status, path })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One file's raw `git show` diff text (fetched once, then re-rendered locally
/// for scrolling without re-shelling out to git).
pub fn load_diff_raw(repo: &str, hash: &str, path: &str) -> String {
    git_capture(repo, &["show", "--format=", FIRST_PARENT, hash, "--", path]).unwrap_or_default()
}

/// One file's whole text at `rev`, untrimmed because indentation is data here.
/// Nothing when the path did not exist at that revision.
pub fn load_blob(repo: &str, rev: &str, path: &str) -> Option<String> {
    git_capture_raw(repo, &["show", &format!("{rev}:{path}")])
}

/// The two revisions a commit's diff is against, for `prepare_diff` to prime its
/// highlighter with. A root commit has no `^`, so its old side simply reads back
/// nothing and that half is highlighted from the hunk as before.
pub fn commit_diff_ctx(repo: &str, hash: &str, path: &str) -> DiffContext {
    let side = |rev: String| -> Blob {
        let (repo, path) = (repo.to_string(), path.to_string());
        Box::new(move || load_blob(&repo, &rev, &path))
    };
    DiffContext {
        old: Some(side(format!("{hash}^"))),
        new: Some(side(hash.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{DECORATE, RefKind, RefLabel, fit_refs, parse_refs, tag_copy, tag_rule};
    use super::{RemoteTags, ask, landed, load_branches, load_diff_raw, load_files, load_tags};
    use crate::git::git_capture;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A throwaway repo under the system temp dir, deleted when it goes out of
    /// scope - including when an assertion panics, so a failing test leaves the
    /// machine as it found it.
    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new(tag: &str) -> TempRepo {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("sluuz-{tag}-{stamp}"));
            fs::create_dir_all(&dir).expect("temp dir");
            let repo = TempRepo(dir);
            repo.git(&["init", "-q"]);
            repo
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("utf-8 temp path")
        }

        fn git(&self, args: &[&str]) {
            git_capture(self.path(), args).unwrap_or_else(|| panic!("git {args:?}"));
        }

        /// Runs `args` as a named author with signing off, so the machine's own
        /// git config cannot make a commit or a merge fail.
        fn as_author(&self, args: &[&str]) {
            let mut all = vec![
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ];
            all.extend_from_slice(args);
            self.git(&all);
        }

        fn commit(&self, file: &str, text: &str, msg: &str) {
            fs::write(self.0.join(file), text).expect("write fixture");
            self.git(&["add", file]);
            self.as_author(&["commit", "-q", "-m", msg]);
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn every_file_a_merge_lists_has_a_diff_to_show() {
        // The two sides edit opposite ends of one file, so it merges cleanly:
        // the case git's default combined diff lists and then prints no hunk for.
        let repo = TempRepo::new("merge");
        repo.commit("f.py", "a\nb\nc\nd\ne\nf\ng\n", "base");
        repo.git(&["checkout", "-q", "-b", "side"]);
        repo.commit("f.py", "A\nb\nc\nd\ne\nf\ng\n", "edit the top");
        repo.git(&["checkout", "-q", "-"]);
        repo.commit("f.py", "a\nb\nc\nd\ne\nf\nG\n", "edit the bottom");
        repo.as_author(&["merge", "-q", "--no-ff", "--no-edit", "side"]);

        let files = load_files(repo.path(), "HEAD", &[]);
        assert!(
            !files.is_empty(),
            "the merge brought a change in, so it lists a file"
        );
        for file in files {
            assert!(
                load_diff_raw(repo.path(), "HEAD", &file.path).contains("@@"),
                "`{}` is listed for the merge but has no hunk to show",
                file.path
            );
        }
    }

    #[test]
    fn a_squash_merged_branch_counts_as_landed_and_new_work_does_not() {
        let repo = TempRepo::new("landed");
        repo.commit("base.txt", "base\n", "base");
        repo.git(&["branch", "-M", "main"]);
        repo.git(&["checkout", "-q", "-b", "feature"]);
        repo.commit("a.txt", "a\n", "add a");
        repo.commit("b.txt", "b\n", "add b");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["merge", "-q", "--squash", "feature"]);
        repo.as_author(&["commit", "-q", "-m", "feature, squashed"]);
        repo.commit("later.txt", "later\n", "main moves on");

        assert!(
            landed(repo.path(), "feature", "main"),
            "a squash gives feature's changes new ids on main, yet every one of them is there"
        );

        repo.git(&["checkout", "-q", "feature"]);
        repo.commit("c.txt", "c\n", "work main never got");
        assert!(
            !landed(repo.path(), "feature", "main"),
            "a commit main does not have means deleting feature would lose it"
        );
    }

    #[test]
    fn an_annotated_tag_is_compared_by_its_object_and_logged_by_its_commit() {
        // The repo is its own remote, so every tag is by definition the same on
        // both sides: any disagreement is a reader getting a side wrong.
        let repo = TempRepo::new("tags");
        repo.commit("f.txt", "f\n", "tagged");
        repo.as_author(&["tag", "-a", "v1", "-m", "annotated"]);
        repo.git(&["tag", "v0"]);
        repo.git(&["fetch", "-q", ".", &tag_rule("self")]);
        let head = git_capture(repo.path(), &["rev-parse", "HEAD"]).expect("HEAD");

        let RemoteTags::Answered { tags: asked, .. } = ask(repo.path(), ".".to_string()) else {
            panic!("`ls-remote` against the repo itself did not answer");
        };
        let copy = tag_copy(repo.path(), "self");
        let local = load_tags(repo.path());
        assert_eq!(local.len(), 2, "both tags should be read");

        for tag in &local {
            assert_eq!(
                tag.commit, head,
                "`{}` should log from its commit",
                tag.name
            );
            assert_eq!(
                tag.annotated,
                tag.object != tag.commit,
                "`{}`: only an annotated tag names an object other than its commit",
                tag.name
            );
            for (reader, remote) in [("ls-remote", &asked), ("git's copy", &copy)] {
                let theirs = remote
                    .get(&tag.name)
                    .unwrap_or_else(|| panic!("{reader} is missing `{}`", tag.name));
                assert_eq!(
                    (&theirs.object, &theirs.commit),
                    (&tag.object, &tag.commit),
                    "{reader} reads `{}` differently from the clone, so an identical tag would mark as differing",
                    tag.name
                );
            }
        }
    }

    #[test]
    fn a_branch_and_a_tag_sharing_a_name_each_keep_it() {
        let repo = TempRepo::new("shared-name");
        repo.commit("f.txt", "f\n", "base");
        repo.git(&["branch", "v1"]);
        repo.git(&["tag", "v1"]);

        let branches: Vec<String> = load_branches(repo.path())
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(
            branches.iter().any(|b| b == "v1"),
            "the branch should read as `v1`, which `git branch -D` accepts, got {branches:?}"
        );
        let tags: Vec<String> = load_tags(repo.path()).into_iter().map(|t| t.name).collect();
        assert_eq!(tags, ["v1"], "the tag should read as `v1`");
    }

    #[test]
    fn a_log_labels_only_branches_remotes_and_tags() {
        // Every kind of ref git decorates with, on one commit, read the way the
        // log views read them. A local branch named like a remote one is the
        // case short names cannot tell apart.
        let repo = TempRepo::new("refs");
        repo.commit("f", "a", "one");
        repo.git(&["branch", "origin/lookalike"]);
        repo.git(&["tag", "v1"]);
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        repo.git(&["update-ref", "refs/remote-tags/origin/v1", "HEAD"]);
        repo.git(&["update-ref", "refs/stash", "HEAD"]);
        let head = git_capture(repo.path(), &["symbolic-ref", "--short", "HEAD"]).expect("HEAD");

        let d = git_capture(repo.path(), &["log", "-1", "--format=%D", DECORATE]).expect("log");
        let mut got: Vec<(RefKind, String)> = parse_refs(&d)
            .into_iter()
            .map(|r| (r.kind, r.name))
            .collect();
        got.sort_by(|a, b| a.1.cmp(&b.1));

        let mut want = vec![
            (RefKind::Head, head),
            (RefKind::Branch, "origin/lookalike".to_string()),
            (RefKind::Remote, "origin/main".to_string()),
            (RefKind::Tag, "v1".to_string()),
        ];
        want.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            got, want,
            "from `{d}`: the stash, origin/HEAD and refs/remote-tags should be left out"
        );
    }

    #[test]
    fn ref_labels_never_run_past_their_room_and_keep_the_first() {
        let refs: Vec<RefLabel> = [
            (RefKind::Head, "feature/a-rather-long-branch-name"),
            (RefKind::Remote, "origin/feature/a-rather-long-branch-name"),
            (RefKind::Tag, "v2.2.0-rc.1"),
            (RefKind::Branch, "main"),
        ]
        .into_iter()
        .map(|(kind, name)| RefLabel {
            kind,
            name: name.to_string(),
        })
        .collect();

        for room in 12..160 {
            let (kept, hidden) = fit_refs(&refs, room);
            // `(a, b, +2) `
            let drawn = 3
                + kept.iter().map(|(_, t)| t.chars().count()).sum::<usize>()
                + 2 * (kept.len() - 1)
                + if hidden > 0 {
                    3 + hidden.to_string().len()
                } else {
                    0
                };
            assert!(
                !kept.is_empty(),
                "room {room}: the first label went missing"
            );
            assert_eq!(
                kept.len() + hidden,
                refs.len(),
                "room {room}: labels lost count"
            );
            assert!(
                drawn <= room,
                "room {room}: drew {drawn} columns from {kept:?} +{hidden}"
            );
            for (i, (_, text)) in kept.iter().enumerate().skip(1) {
                assert_eq!(
                    *text,
                    refs[i].text(),
                    "room {room}: only the first may be cut"
                );
            }
        }
    }
}
