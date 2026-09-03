//! Git loaders behind the interactive views: commits, the files a commit
//! touched, one file's raw diff, and which commits are unpushed.
//!
//! Every loader takes the repo to read, so one view can walk several repos in
//! the same session. `"."` is the current one.

use crate::git::{SEP, git_capture};
use std::collections::HashSet;
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
}

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

pub struct Commit {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub committer: String,
    pub subject: String,
}

pub struct FileEntry {
    pub status: char,
    pub path: String,
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

/// One `--pretty=format:` line back into a `Commit`.
fn parse_commit(line: &str) -> Option<Commit> {
    let mut f = line.split(SEP);
    Some(Commit {
        hash: f.next()?.to_string(),
        short: f.next()?.to_string(),
        date: f.next()?.to_string(),
        committer: f.next()?.to_string(),
        subject: f.next().unwrap_or("").to_string(),
    })
}

/// The `for-each-ref` format both branch readers ask for.
fn branch_format() -> String {
    format!(
        "--format=%(HEAD){SEP}%(refname){SEP}%(refname:short){SEP}%(committerdate:relative){SEP}%(authorname){SEP}%(upstream){SEP}%(upstream:track)"
    )
}

/// One `for-each-ref` line back into a `Branch`.
fn parse_branch(line: &str) -> Option<Branch> {
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
    thread::spawn(move || {
        let n = limit.to_string();
        let fmt = format!("--pretty=format:%H{SEP}%h{SEP}%ad{SEP}%cn{SEP}%s");
        let mut argv = vec![
            "log",
            "-n",
            n.as_str(),
            "--date=format:%Y-%m-%d %H:%M",
            fmt.as_str(),
        ];
        argv.extend(args.iter().map(String::as_str));
        stream_rows(
            &repo,
            &argv,
            seq,
            &latest,
            &tx,
            parse_commit,
            |seq, rows, done| Batch::Commits { seq, rows, done },
        );
    });
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

/// The files a commit touched, with their status (cheap - no diff content).
/// When `pathspec` is non-empty, only files matching it are returned (so a
/// path-filtered `ilog` shows just that file's change in each commit).
pub fn load_files(repo: &str, hash: &str, pathspec: &[&str]) -> Vec<FileEntry> {
    let mut args = vec!["show", "--name-status", "--format=", hash];
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
    git_capture(repo, &["show", "--format=", hash, "--", path]).unwrap_or_default()
}
