//! `gt scan` — scan git repositories for sensitive terms in commit history.
//!
//! Checks every commit across all branches for a list of sensitive terms, then
//! reports which commit, branch, and file each hit came from. Matching uses
//! git's pickaxe (via `crate::history`) so it also catches secrets committed in
//! binary/encrypted files, where a plain diff grep would see nothing.

use crate::git::{display_name, find_repos};
use crate::history::{self, CommitMatch};
use colored::Colorize;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// Base directory to search for repos (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Comma-separated list of terms to flag as sensitive
    #[arg(
        short,
        long,
        default_value = "password,secret,token,api_key,passwd,credentials"
    )]
    pub terms: String,

    /// How many directory levels deep to look for repos
    #[arg(short, long, default_value_t = 3)]
    pub depth: usize,
}

pub fn run(args: Args) {
    // Normalise terms: trim whitespace, drop empties. Matching itself is
    // case-insensitive, handled inside history::pickaxe.
    let terms: Vec<String> = args
        .terms
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    println!("\n{}", "REPO SCANNER".bold());
    println!("{}", format!("base  : {}", args.path.display()).dimmed());
    println!("{}", format!("terms : {}", terms.join(", ")).dimmed());
    println!("{}\n", "━".repeat(56));

    let repos = find_repos(&args.path, args.depth);

    let mut total_repos = 0usize;
    let mut repos_with_hits = 0usize;
    let mut total_hits = 0usize;

    for repo in &repos {
        total_repos += 1;

        let name = display_name(repo);
        println!(
            "{} {}",
            format!("📁 {}", name).bold(),
            repo.display().to_string().dimmed()
        );

        let repo_str = match repo.to_str() {
            Some(s) => s,
            None => {
                println!("   {}\n", "✓ no matches".green());
                continue;
            }
        };

        let commits = history::pickaxe(repo_str, &terms, true);

        if commits.is_empty() {
            println!("   {}\n", "✓ no matches".green());
            continue;
        }

        let hits = count_hits(&commits);
        repos_with_hits += 1;
        total_hits += hits;
        println!("   {}\n", format!("⚠  {} hit(s)", hits).red().bold());

        for commit in &commits {
            let branches = history::branches_for(repo_str, &commit.full);

            println!(
                "   {}  {}  {}",
                commit.date.dimmed(),
                commit.short.cyan().bold(),
                commit.subject
            );
            if !branches.is_empty() {
                println!("   {} {}", "branch │".magenta(), branches.join("  "));
            }
            for file in &commit.files {
                println!("   {} {}", "file   │".dimmed(), file.path);
                if file.lines.is_empty() {
                    println!(
                        "   {} {}",
                        "hit    │".yellow(),
                        "(binary or no visible diff)".dimmed()
                    );
                }
                for (_is_addition, line) in &file.lines {
                    println!("   {} {}", "hit    │".yellow(), line.trim());
                }
            }
            println!();
        }
    }

    print_summary(total_repos, repos_with_hits, total_hits);
}

/// Count findings across a repo's commits: one per matched line, or one per
/// binary/encrypted file that pickaxe flagged but showed no visible lines.
fn count_hits(commits: &[CommitMatch]) -> usize {
    commits
        .iter()
        .flat_map(|c| c.files.iter())
        .map(|f| f.lines.len().max(1))
        .sum()
}

fn print_summary(total_repos: usize, repos_with_hits: usize, total_hits: usize) {
    println!("{}", "━".repeat(56));
    println!("{}", "SUMMARY".bold());
    println!("  repos scanned   : {}", total_repos.to_string().bold());

    if repos_with_hits > 0 {
        println!(
            "  repos with hits : {}",
            repos_with_hits.to_string().red().bold()
        );
        println!("  total hits      : {}", total_hits.to_string().red().bold());
        println!("\n  {}", "to remove a secret from history:".dimmed());
        println!(
            "  {}",
            "  recent, top of one branch : git reset --hard <hash>^ && git push --force".dimmed()
        );
        println!(
            "  {}",
            "  deep or on many branches  : git filter-repo --replace-text  (rewrites all history)"
                .dimmed()
        );
    } else {
        println!("  {}", "✓ all clean".green().bold());
    }

    println!();
}
