//! `slu ilog` - interactive log explorer (TUI). The commits level of the drill,
//! entered directly: this repo's log, its files, and their diffs.
//!
//! Commits (top, j/k) show the selected commit's files below (Ctrl-j/k); Enter
//! opens one into a side-by-side, syntax-highlighted diff. `h`/`l` slide the
//! scope between local (unpushed), all, and pushed. Esc quits, since this is
//! where the app was entered. See `app/` for the levels above it.

use crate::app::App;
use crate::git::git_capture;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {
    /// Include commits from all branches
    #[arg(short, long)]
    pub all: bool,

    /// Maximum number of commits to load
    #[arg(short = 'n', long, default_value_t = 200)]
    pub number: usize,

    /// Only commits that touch these paths (file or directory); diffs show just
    /// that file's change
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu ilog needs an interactive terminal - use `slu trace` for plain output");
        return;
    }

    // The user typed their paths relative to the cwd, but git runs against the
    // repo root - so rewrite them to root-relative, or `slu ilog .env` from a
    // subdirectory would look for `<root>/.env`.
    let prefix = git_capture(".", &["rev-parse", "--show-prefix"]).unwrap_or_default();
    let paths: Vec<String> = args.paths.iter().map(|p| rebase_path(p, &prefix)).collect();
    let repo =
        git_capture(".", &["rev-parse", "--show-toplevel"]).unwrap_or_else(|| ".".to_string());

    // Build the `git log` args: [--all] then `-- <paths>` to filter to a file.
    let mut log_args: Vec<String> = Vec::new();
    if args.all {
        log_args.push("--all".to_string());
    }
    if !paths.is_empty() {
        log_args.push("--".to_string());
        log_args.extend(paths.iter().cloned());
    }

    match App::at_commits(repo, log_args, args.number, paths) {
        Some(app) => app.run("ilog"),
        None => eprintln!("no commits (or not a git repo)"),
    }
}

/// Rewrite a user-typed path so it is relative to the repo root. `prefix` is
/// `git rev-parse --show-prefix` (the cwd's path below the root, e.g. `PROD/`,
/// empty at the root). Absolute paths are left alone - git resolves those
/// itself. `.` and `..` segments are folded lexically so `../x` from `PROD/`
/// becomes `x`, not `PROD/../x`.
fn rebase_path(path: &str, prefix: &str) -> String {
    if prefix.is_empty() || std::path::Path::new(path).is_absolute() {
        return path.to_string();
    }
    let joined = format!("{prefix}{path}");
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::rebase_path;

    #[test]
    fn at_repo_root_paths_pass_through() {
        assert_eq!(rebase_path(".env", ""), ".env");
        assert_eq!(rebase_path("src/main.rs", ""), "src/main.rs");
    }

    #[test]
    fn subdir_paths_become_root_relative() {
        // `slu ilog .env` from PROD/ must look for PROD/.env, not <root>/.env.
        assert_eq!(rebase_path(".env", "PROD/"), "PROD/.env");
        assert_eq!(rebase_path("src/a.rs", "a/b/"), "a/b/src/a.rs");
    }

    #[test]
    fn dot_and_dotdot_segments_fold() {
        assert_eq!(rebase_path("./.env", "PROD/"), "PROD/.env");
        assert_eq!(rebase_path("../.env", "PROD/"), ".env");
        assert_eq!(rebase_path("../other/x", "a/b/"), "a/other/x");
    }

    #[test]
    fn absolute_paths_are_left_to_git() {
        assert_eq!(rebase_path("/tmp/x", "PROD/"), "/tmp/x");
    }
}
