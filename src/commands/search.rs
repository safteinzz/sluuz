//! `gt search` — find commits where a string was added or removed (git pickaxe).
//!
//! Equivalent to: git log -S <pattern> -p, but with colorized output, the
//! matching file(s), the branches that contain each commit, and optional
//! multi-repo parallel search. The actual history walking lives in
//! `crate::history`, shared with `scan`.

use crate::git::{display_name, find_repos};
use crate::history::{self, CommitMatch};
use colored::Colorize;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

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

    // A single, case-sensitive term — matches the original fixed-string behavior.
    let terms = vec![args.pattern.clone()];

    // `par_iter` is rayon's parallel version of `iter`; it spreads the per-repo
    // git work across a thread pool. We collect (name, output) pairs so we can
    // sort before printing for deterministic output.
    let mut results: Vec<(String, String)> = repos
        .par_iter()
        .filter_map(|repo| {
            let name = display_name(repo);
            let repo_str = repo.to_str()?;
            let commits = history::pickaxe(repo_str, &terms, false);
            if commits.is_empty() {
                return None;
            }
            Some((name.clone(), format_repo(&name, repo_str, &commits, args.limit)))
        })
        .collect();

    if results.is_empty() {
        println!("{}", "No matches found.".dimmed());
        return;
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));

    for (_, output) in results {
        print!("{}", output);
    }
}

/// Format one repository's matches into a colorized block.
fn format_repo(name: &str, repo: &str, commits: &[CommitMatch], limit: usize) -> String {
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
            format!("▸ {}", commit.short).yellow(),
            commit.subject
        ));

        let branches = history::branches_for(repo, &commit.full);
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
                out.push_str(&format!("      {}\n", "(binary or no visible diff)".dimmed()));
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
    out
}
