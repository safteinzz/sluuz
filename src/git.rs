//! Shared git utility functions used across subcommands.

use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// ASCII unit separator — a field delimiter that won't appear in git's output,
/// so `--format` strings can split on it without quoting anything.
pub const SEP: char = '\u{1f}';

/// Find all git repositories under `base`, searching up to `max_depth` levels deep.
/// Returns each repo's root directory (the parent of its `.git` directory).
pub fn find_repos(base: &Path, max_depth: usize) -> Vec<PathBuf> {
    WalkDir::new(base)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())                                         // skip permission errors
        .filter(|entry| entry.file_name() == ".git" && entry.file_type().is_dir())
        .filter_map(|entry| entry.path().parent().map(Path::to_path_buf))       // .git → repo root
        .collect()
}

/// One repo's working-tree state: what `slu repos` prints as a row and what the
/// repos level of the interactive drill shows.
pub struct RepoStatus {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub has_upstream: bool,
    pub dirty: usize,
    pub ahead: usize,
    pub behind: usize,
}

impl RepoStatus {
    /// Whether the repo needs attention: uncommitted, unpushed, or unpulled.
    pub fn needs_attention(&self) -> bool {
        self.dirty > 0 || self.ahead > 0 || self.behind > 0
    }

    /// Uncommitted work in the working tree.
    pub fn is_dirty(&self) -> bool {
        self.dirty > 0
    }

    /// Commits that exist here and on no remote.
    pub fn is_unpushed(&self) -> bool {
        self.ahead > 0
    }
}

/// Read one repo's status via `git status --porcelain=2 --branch`, which packs
/// branch, upstream, ahead/behind, and changed files into one machine-readable
/// listing.
pub fn repo_status(repo: &Path) -> RepoStatus {
    let path = repo.to_string_lossy().into_owned();
    let out = git_capture(&path, &["status", "--porcelain=2", "--branch"]).unwrap_or_default();

    let mut branch = "(detached)".to_string();
    let mut has_upstream = false;
    let mut ahead = 0usize;
    let mut behind = 0usize;
    let mut dirty = 0usize;

    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest.to_string();
        } else if line.starts_with("# branch.upstream ") {
            has_upstream = true;
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // "+<ahead> -<behind>"
            for tok in rest.split_whitespace() {
                if let Some(a) = tok.strip_prefix('+') {
                    ahead = a.parse().unwrap_or(0);
                } else if let Some(b) = tok.strip_prefix('-') {
                    behind = b.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            // Any non-header line is a changed/untracked entry.
            dirty += 1;
        }
    }

    RepoStatus {
        name: display_name(repo),
        path,
        branch,
        has_upstream,
        dirty,
        ahead,
        behind,
    }
}

/// Get a human-readable repo name from its path (its directory name).
pub fn display_name(repo: &Path) -> String {
    repo.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| repo.to_string_lossy().into_owned())
}

/// Run `git -C <repo> <args>` and return trimmed stdout, or None if git fails
/// or exits non-zero. For read-only queries where you only want the output.
pub fn git_capture(repo: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Like `git_capture`, but returns stdout **untrimmed**. Use it whenever leading
/// whitespace is data — notably `git status --porcelain`, where the first column
/// is a space when a file has no staged change. Trimming would eat that space on
/// the first record and shift the whole line ("Cargo.lock" → "argo.lock").
pub fn git_capture_raw(repo: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run `git -C <repo> <args>` and return (success, combined stdout+stderr).
/// For commands like fetch/pull where progress goes to stderr and you want to
/// report what happened regardless of exit status.
pub fn git_run(repo: &str, args: &[&str]) -> (bool, String) {
    match Command::new("git").arg("-C").arg(repo).args(args).output() {
        Ok(output) => {
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            (output.status.success(), combined.trim().to_string())
        }
        Err(e) => (false, e.to_string()),
    }
}
