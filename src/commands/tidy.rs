//! `slu tidy` — find local branches whose upstream is **gone** (the remote
//! branch they tracked was deleted) across every repo under a path, with how
//! long since each was last touched.
//!
//! These are the genuinely-finished branches — e.g. a merged PR the remote
//! auto-deleted — the same set `slu itidy` offers to delete interactively.
//! Branches still alive on the remote, or that never had an upstream, are left
//! alone. This is the non-interactive, multi-repo view; `slu itidy` is the TUI.

use crate::git::{display_name, find_repos, git_capture, SEP};
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

        let merged = gone_branches(repo_str, &current);

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
        println!("   {}", "upstream gone — safe to delete:".dimmed());

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
        println!("{}", "No branches with a gone upstream to clean up.".green());
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

/// Local branches whose upstream is gone (the remote branch they tracked was
/// deleted), excluding the checked-out `current` branch, each with a relative
/// "last commit" age. Same detection as `slu itidy`.
///
/// `%(refname)` is stripped to the plain branch name ourselves — `%(refname:short)`
/// would return `heads/v0.2.20` when a tag of the same name exists, which
/// `git branch -d` can't take.
fn gone_branches(repo: &str, current: &str) -> Vec<Branch> {
    let fmt = format!("--format=%(refname){SEP}%(upstream:track){SEP}%(committerdate:relative)");
    git_capture(repo, &["for-each-ref", "--sort=-committerdate", &fmt, "refs/heads"])
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    let refname = f.next()?;
                    let track = f.next()?;
                    // `[gone]` marks a tracking branch whose remote was deleted.
                    if !track.contains("gone") {
                        return None;
                    }
                    let name = refname.strip_prefix("refs/heads/").unwrap_or(refname);
                    if name == current {
                        return None; // can't delete the checked-out branch
                    }
                    Some(Branch {
                        name: name.to_string(),
                        age: f.next().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
