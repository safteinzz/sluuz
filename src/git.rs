//! Shared git utility functions used across subcommands.

use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

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
