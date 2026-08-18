//! Git loaders behind the interactive views: commits, the files a commit
//! touched, one file's raw diff, and which commits are unpushed.
//!
//! Every loader takes the repo to read, so one view can walk several repos in
//! the same session. `"."` is the current one.

use crate::git::{git_capture, SEP};
use std::collections::HashSet;

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

/// Hashes reachable from local branches but on **no** remote — i.e. commits you
/// haven't pushed anywhere. A commit not in this set is on some remote (pushed).
/// Empty when the repo has no remotes (nothing is "pushed").
pub fn load_unpushed(repo: &str) -> HashSet<String> {
    git_capture(repo, &["rev-list", "--branches", "--not", "--remotes"])
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Load commits as `git log -n <limit> [extra…]`. `extra` is e.g. `["--all"]`
/// or a branch name, appended after the format args.
pub fn load_commits(repo: &str, extra: &[&str], limit: usize) -> Vec<Commit> {
    let n = limit.to_string();
    let fmt = format!("--pretty=format:%H{SEP}%h{SEP}%ad{SEP}%cn{SEP}%s");
    let mut args = vec!["log", "-n", &n, "--date=format:%Y-%m-%d %H:%M", &fmt];
    args.extend_from_slice(extra);
    git_capture(repo, &args)
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    Some(Commit {
                        hash: f.next()?.to_string(),
                        short: f.next()?.to_string(),
                        date: f.next()?.to_string(),
                        committer: f.next()?.to_string(),
                        subject: f.next().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The files a commit touched, with their status (cheap — no diff content).
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
