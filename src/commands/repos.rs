//! `slu repos` - at-a-glance working-tree state for every repo under a path.
//!
//! For each repo it shows the current branch, how many files are dirty, and how
//! far ahead/behind its upstream it is - so "which of my repos have uncommitted
//! or unpushed work?" is one command instead of cd-ing through each.

use crate::git::{RepoStatus, find_repos, repo_status};
use colored::Colorize;
use rayon::prelude::*;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// Base directory to search for repos (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// How many directory levels deep to look for repos
    #[arg(short, long, default_value_t = 3)]
    pub depth: usize,

    /// Only show repos that need attention (dirty or ahead/behind)
    #[arg(long)]
    pub dirty: bool,
}

pub fn run(args: Args) {
    let repos = find_repos(&args.path, args.depth);

    let mut statuses: Vec<RepoStatus> = repos.par_iter().map(|r| repo_status(r)).collect();

    statuses.sort_by(|a, b| a.name.cmp(&b.name));

    let shown: Vec<&RepoStatus> = statuses
        .iter()
        .filter(|s| !args.dirty || s.needs_attention())
        .collect();

    if shown.is_empty() {
        println!("{}", "All repos clean and in sync.".green());
        return;
    }

    // Pad name/branch columns so the state column lines up. Padding is computed
    // on the plain strings, then color is applied, so ANSI codes don't skew it.
    let name_w = shown.iter().map(|s| s.name.len()).max().unwrap_or(0);
    let branch_w = shown.iter().map(|s| s.branch.len()).max().unwrap_or(0);

    let mut dirty_repos = 0usize;
    let mut unpushed_repos = 0usize;

    for s in &shown {
        if s.dirty > 0 {
            dirty_repos += 1;
        }
        if s.ahead > 0 {
            unpushed_repos += 1;
        }

        let name = format!("{:name_w$}", s.name);
        let branch = format!("{:branch_w$}", s.branch);
        println!("  {}  {}  {}", name.bold(), branch.cyan(), state(s));
    }

    println!();
    println!(
        "{}",
        format!(
            "{} repos · {} dirty · {} with unpushed commits",
            shown.len(),
            dirty_repos,
            unpushed_repos
        )
        .dimmed()
    );
}

/// Build the colorized state column for one repo.
fn state(s: &RepoStatus) -> String {
    let mut flags: Vec<String> = Vec::new();
    if s.dirty > 0 {
        flags.push(format!("✚{}", s.dirty).yellow().to_string());
    }
    if s.ahead > 0 {
        flags.push(format!("↑{}", s.ahead).green().to_string());
    }
    if s.behind > 0 {
        flags.push(format!("↓{}", s.behind).red().to_string());
    }

    if !flags.is_empty() {
        return flags.join(" ");
    }
    if s.has_upstream {
        "✓ clean".green().to_string()
    } else {
        "✓ clean (no upstream)".dimmed().to_string()
    }
}
