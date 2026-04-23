//! `gt search` — find commits where a string was added or removed (git pickaxe).
//!
//! Equivalent to: git log -S <pattern> --pickaxe-all -p
//! But with colorized output and optional multi-repo parallel search.

use crate::git::find_repos;
use colored::Colorize;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(clap::Args)]
pub struct Args {
    /// The string to search for in git history
    pub pattern: String,

    /// Search all git repos found recursively under the current directory
    #[arg(short, long)]
    pub recursive: bool,

    /// Maximum number of commits to display per repository
    #[arg(short, long, default_value_t = 20)]
    pub limit: usize,
}

pub fn run(args: Args) {
    println!("{} {}\n", "Searching for:".dimmed(), args.pattern.bold());

    let repos: Vec<PathBuf> = if args.recursive {
        find_repos(Path::new("."), 10)
    } else {
        vec![PathBuf::from(".")]
    };

    // `par_iter` is rayon's parallel version of `iter`.
    // It splits the work across a thread pool automatically.
    // We collect (repo_name, formatted_output) pairs so we can sort before printing.
    let mut results: Vec<(String, String)> = repos
        .par_iter()
        .filter_map(|repo| {
            let name = display_name(repo);
            search_repo(repo, &name, &args.pattern, args.limit)
                .map(|output| (name, output))
        })
        .collect();

    if results.is_empty() {
        println!("{}", "No matches found.".dimmed());
        return;
    }

    // Sort by name so output is deterministic regardless of thread scheduling
    results.sort_by(|a, b| a.0.cmp(&b.0));

    for (_, output) in results {
        print!("{}", output);
    }
}

// ── internal types ────────────────────────────────────────────────────────────

/// One commit that matched, with the diff lines containing the pattern.
struct CommitMatch {
    hash: String,
    message: String,
    /// (is_addition, line) — additions are green, deletions are red
    diff_lines: Vec<(bool, String)>,
}

// ── core logic ────────────────────────────────────────────────────────────────

/// Run the search in one repository. Returns formatted output, or None if no hits.
fn search_repo(repo: &Path, name: &str, pattern: &str, limit: usize) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-C",
            repo.to_str()?,
            "log",
            "--all",
            "-S", pattern,       // pickaxe: only commits that changed the count of `pattern`
            "--pickaxe-all",     // show the whole diff, not just the file that changed
            "-F",                // treat pattern as a fixed string, not a regex
            "--pretty=format:COMMIT:%h|||%s",
            "-p",                // include the diff patch
        ])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let commits = parse_log(&stdout, pattern);

    if commits.is_empty() {
        return None;
    }

    let total = commits.len();
    let mut out = String::new();

    out.push_str(&format!(
        "{} {}\n",
        format!("━━━ {}", name).cyan().bold(),
        format!("{} commit(s)", total).dimmed()
    ));

    for commit in commits.iter().take(limit) {
        out.push_str(&format!(
            "\n  {} {}\n",
            format!("▸ {}", commit.hash).yellow(),
            commit.message
        ));
        for (is_addition, line) in &commit.diff_lines {
            if *is_addition {
                out.push_str(&format!("    {}\n", line.green()));
            } else {
                out.push_str(&format!("    {}\n", line.red()));
            }
        }
    }

    if total > limit {
        out.push_str(&format!(
            "\n  {}\n",
            format!("↳ {} more — use -l {} to see all", total - limit, total).dimmed()
        ));
    }

    out.push('\n');
    Some(out)
}

/// Parse raw `git log -p` output into a list of CommitMatch structs.
fn parse_log(output: &str, pattern: &str) -> Vec<CommitMatch> {
    let mut commits: Vec<CommitMatch> = Vec::new();
    // `current` holds the commit we're building as we scan lines.
    // `Option` lets us represent "not yet started" cleanly.
    let mut current: Option<CommitMatch> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("COMMIT:") {
            // A new commit header: flush the previous one first.
            if let Some(c) = current.take() {
                commits.push(c);
            }
            // Format: "COMMIT:<short_hash>|||<subject>"
            let (hash, message) = rest.split_once("|||").unwrap_or((rest, ""));
            current = Some(CommitMatch {
                hash: hash.to_string(),
                message: message.to_string(),
                diff_lines: Vec::new(),
            });
        } else if let Some(c) = current.as_mut() {
            // Capture +/- diff lines that contain the search pattern.
            // Skip the file header lines (+++ b/file and --- a/file).
            let is_add = line.starts_with('+') && !line.starts_with("+++");
            let is_del = line.starts_with('-') && !line.starts_with("---");

            if (is_add || is_del) && line.contains(pattern) {
                c.diff_lines.push((is_add, line.to_string()));
            }
        }
    }

    // The last commit has no following COMMIT: line to trigger a flush, so flush manually.
    if let Some(c) = current.take() {
        commits.push(c);
    }

    commits
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Get a human-readable repo name from its path.
fn display_name(repo: &Path) -> String {
    repo.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| repo.to_string_lossy().into_owned())
}
