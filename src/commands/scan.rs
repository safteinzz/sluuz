//! `gt scan` - scan git repositories for sensitive terms in commit history.
//!
//! Checks every commit across all branches for a list of sensitive terms, then
//! reports which commit, branch, and file each hit came from. Matching uses
//! git's pickaxe (via `crate::history`) so it also catches secrets committed in
//! binary/encrypted files, where a plain diff grep would see nothing.

use crate::git::{display_name, repos_or_exit};
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

    outln!("\n{}", "REPO SCANNER".bold());
    outln!("{}", format!("base  : {}", args.path.display()).dimmed());
    outln!("{}", format!("terms : {}", terms.join(", ")).dimmed());
    outln!("{}\n", "━".repeat(56));

    let repos = repos_or_exit(&args.path, args.depth);

    let mut total_repos = 0usize;
    let mut repos_with_hits = 0usize;
    let mut total_hits = 0usize;

    for repo in &repos {
        total_repos += 1;

        let name = display_name(repo);
        outln!(
            "{} {}",
            format!("📁 {}", name).bold(),
            repo.display().to_string().dimmed()
        );

        let repo_str = match repo.to_str() {
            Some(s) => s,
            None => {
                outln!("   {}\n", "✓ no matches".green());
                continue;
            }
        };

        let commits = history::pickaxe(repo_str, &terms, true);

        if commits.is_empty() {
            outln!("   {}\n", "✓ no matches".green());
            continue;
        }

        let hits = count_hits(&commits);
        repos_with_hits += 1;
        total_hits += hits;
        outln!("   {}\n", format!("⚠  {} hit(s)", hits).red().bold());

        for commit in &commits {
            let branches = history::branches_for(repo_str, &commit.full);

            outln!(
                "   {}  {}  {}",
                commit.date.dimmed(),
                commit.short.cyan().bold(),
                commit.subject
            );
            if !branches.is_empty() {
                outln!("   {} {}", "branch │".magenta(), branches.join("  "));
            }
            for file in &commit.files {
                outln!("   {} {}", "file   │".dimmed(), file.path);
                if file.lines.is_empty() {
                    outln!(
                        "   {} {}",
                        "hit    │".yellow(),
                        "(binary or no visible diff)".dimmed()
                    );
                }
                for (_is_addition, line) in &file.lines {
                    outln!("   {} {}", "hit    │".yellow(), line.trim());
                }
            }
            outln!();
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
    outln!("{}", "━".repeat(56));
    outln!("{}", "SUMMARY".bold());
    outln!("  repos scanned   : {}", total_repos.to_string().bold());

    if repos_with_hits > 0 {
        outln!(
            "  repos with hits : {}",
            repos_with_hits.to_string().red().bold()
        );
        outln!(
            "  total hits      : {}",
            total_hits.to_string().red().bold()
        );
        outln!("\n  {}", "to remove a secret from history:".dimmed());
        outln!(
            "  {}",
            "  recent, top of one branch : git reset --hard <hash>^ && git push --force".dimmed()
        );
        outln!(
            "  {}",
            "  deep or on many branches  : git filter-repo --replace-text  (rewrites all history)"
                .dimmed()
        );
    } else {
        outln!("  {}", "✓ all clean".green().bold());
    }

    outln!();
}
