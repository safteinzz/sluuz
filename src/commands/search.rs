//! `gt search` — find commits where a string was added or removed (git pickaxe).
//!
//! Equivalent to: git log -S <pattern> -p
//! But with colorized output, the matching file(s), the branches that contain
//! each commit, and optional multi-repo parallel search.

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

/// One commit that matched, with the files that changed and their diff lines.
struct CommitMatch {
    hash: String,
    message: String,
    files: Vec<FileMatch>,
}

/// One file within a matching commit, with the diff lines containing the pattern.
struct FileMatch {
    path: String,
    /// (is_addition, line) — additions are green, deletions are red.
    /// May be empty for binary/encrypted files (git shows no +/- lines for those).
    lines: Vec<(bool, String)>,
}

// ── core logic ────────────────────────────────────────────────────────────────

/// Run the search in one repository. Returns formatted output, or None if no hits.
fn search_repo(repo: &Path, name: &str, pattern: &str, limit: usize) -> Option<String> {
    let repo_str = repo.to_str()?;
    let output = Command::new("git")
        .args([
            "-C",
            repo_str,
            "log",
            "--all",
            "-S", pattern,       // pickaxe: only commits that changed the count of `pattern`
            "-F",                // treat pattern as a fixed string, not a regex
            "--pretty=format:COMMIT:%h|||%s",
            "-p",                // include the diff patch (only for the file(s) that changed)
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

        let branches = branches_for(repo_str, &commit.hash);
        if !branches.is_empty() {
            out.push_str(&format!(
                "    {} {}\n",
                "on".magenta(),
                branches.join(", ").dimmed()
            ));
        }

        for file in &commit.files {
            out.push_str(&format!("    {}\n", file.path.blue().bold()));
            if file.lines.is_empty() {
                out.push_str(&format!(
                    "      {}\n",
                    "(binary or no visible diff)".dimmed()
                ));
            }
            for (is_addition, line) in &file.lines {
                if *is_addition {
                    out.push_str(&format!("      {}\n", line.green()));
                } else {
                    out.push_str(&format!("      {}\n", line.red()));
                }
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
                files: Vec::new(),
            });
        } else if let Some(c) = current.as_mut() {
            if let Some(path) = parse_diff_header(line) {
                // A new file diff begins. Since we don't pass --pickaxe-all,
                // every file git shows here is one where `pattern`'s count changed.
                c.files.push(FileMatch {
                    path,
                    lines: Vec::new(),
                });
            } else if let Some(file) = c.files.last_mut() {
                // Capture +/- diff lines that contain the search pattern, attaching
                // them to the file whose diff we're currently inside.
                // Skip the file header lines (+++ b/file and --- a/file).
                let is_add = line.starts_with('+') && !line.starts_with("+++");
                let is_del = line.starts_with('-') && !line.starts_with("---");

                if (is_add || is_del) && line.contains(pattern) {
                    file.lines.push((is_add, line.to_string()));
                }
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

/// Extract the file path from a `diff --git a/<path> b/<path>` header line.
/// Returns None for any other line. Uses the `b/` (destination) path; for a
/// deletion git still records the real name there.
fn parse_diff_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    // The header is "a/<path> b/<path>". Split on the last " b/" so paths that
    // happen to contain " b/" earlier don't trip us up.
    let idx = rest.rfind(" b/")?;
    Some(rest[idx + 3..].to_string())
}

/// List the branches (local and remote) that contain `hash`.
fn branches_for(repo: &str, hash: &str) -> Vec<String> {
    Command::new("git")
        .args([
            "-C",
            repo,
            "branch",
            "-a",
            "--contains",
            hash,
            "--format=%(refname:short)",
        ])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Get a human-readable repo name from its path.
fn display_name(repo: &Path) -> String {
    repo.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| repo.to_string_lossy().into_owned())
}
