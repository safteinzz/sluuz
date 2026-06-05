//! `gt status` — at-a-glance working-tree state for every repo under a path.
//!
//! For each repo it shows the current branch, how many files are dirty, and how
//! far ahead/behind its upstream it is — so "which of my repos have uncommitted
//! or unpushed work?" is one command instead of cd-ing through each.

use crate::git::{display_name, find_repos, git_capture};
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

struct RepoStatus {
    name: String,
    branch: String,
    has_upstream: bool,
    dirty: usize,
    ahead: usize,
    behind: usize,
}

impl RepoStatus {
    /// Whether the repo needs attention: uncommitted, unpushed, or unpulled.
    fn needs_attention(&self) -> bool {
        self.dirty > 0 || self.ahead > 0 || self.behind > 0
    }
}

pub fn run(args: Args) {
    let repos = find_repos(&args.path, args.depth);

    let mut statuses: Vec<RepoStatus> = repos
        .par_iter()
        .filter_map(|repo| {
            let name = display_name(repo);
            let repo_str = repo.to_str()?;
            Some(repo_status(name, repo_str))
        })
        .collect();

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

/// Read one repo's status via `git status --porcelain=2 --branch`, which packs
/// branch, upstream, ahead/behind, and changed files into one machine-readable
/// listing.
fn repo_status(name: String, repo: &str) -> RepoStatus {
    let out = git_capture(repo, &["status", "--porcelain=2", "--branch"]).unwrap_or_default();

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
        name,
        branch,
        has_upstream,
        dirty,
        ahead,
        behind,
    }
}
