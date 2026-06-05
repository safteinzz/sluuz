//! `gt branches` — find local branches that are already merged (safe to delete)
//! across every repo under a path, with how long since each was last touched.
//!
//! "Merged" means merged into the current branch (`git branch --merged`), so
//! these are branches whose work is already in your checked-out line of history.

use crate::git::{display_name, find_repos, git_capture};
use colored::Colorize;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// Base directory to search for repos (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// How many directory levels deep to look for repos
    #[arg(short, long, default_value_t = 3)]
    pub depth: usize,

    /// Show every repo, including those with nothing to clean up
    #[arg(short, long)]
    pub all: bool,
}

struct Branch {
    name: String,
    age: String,
}

pub fn run(args: Args) {
    let repos = find_repos(&args.path, args.depth);

    let mut total_repos = 0usize;
    let mut total_branches = 0usize;

    for repo in &repos {
        let repo_str = match repo.to_str() {
            Some(s) => s,
            None => continue,
        };
        let name = display_name(repo);
        let current = git_capture(repo_str, &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap_or_else(|| "HEAD".to_string());

        let merged = merged_branches(repo_str, &current);

        if merged.is_empty() {
            if args.all {
                println!(
                    "{} {}",
                    format!("📁 {}", name).bold(),
                    "nothing to clean up".dimmed()
                );
            }
            continue;
        }

        total_repos += 1;
        total_branches += merged.len();

        println!(
            "{}  {}",
            format!("📁 {}", name).bold(),
            format!("(on {})", current).dimmed()
        );
        println!("   {}", "merged, safe to delete:".dimmed());

        let width = merged.iter().map(|b| b.name.len()).max().unwrap_or(0);
        for b in &merged {
            println!(
                "     {}  {}",
                format!("{:width$}", b.name).yellow(),
                b.age.dimmed()
            );
        }
        // Handy one-liner to actually delete them.
        let names = merged
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        println!("   {} git branch -d {}\n", "↳".dimmed(), names.dimmed());
    }

    if total_branches == 0 {
        println!("{}", "No merged branches to clean up.".green());
    } else {
        println!(
            "{}",
            format!(
                "{} branch(es) across {} repo(s) can be deleted",
                total_branches, total_repos
            )
            .dimmed()
        );
    }
}

/// Local branches merged into `current`, excluding `current` itself, each with
/// a relative "last commit" age.
fn merged_branches(repo: &str, current: &str) -> Vec<Branch> {
    // `--format` must precede `--merged`: `--merged` takes an optional commit
    // argument, so `--merged --format=…` would swallow the format as that commit.
    let out = git_capture(repo, &["branch", "--format=%(refname:short)", "--merged"])
        .unwrap_or_default();

    out.lines()
        .map(str::trim)
        .filter(|b| !b.is_empty() && *b != current)
        .map(|b| {
            let age = git_capture(repo, &["log", "-1", "--format=%cr", b])
                .unwrap_or_else(|| "unknown".to_string());
            Branch {
                name: b.to_string(),
                age,
            }
        })
        .collect()
}
