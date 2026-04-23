//! `gt scan` — scan git repositories for sensitive terms in commit history.
//!
//! Checks every commit across all branches for lines matching the configured terms,
//! then reports which commit, branch, and file each hit came from.

use crate::git::find_repos;
use colored::Colorize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    // Normalise terms: lowercase and trim whitespace around commas
    let terms: Vec<String> = args
        .terms
        .split(',')
        .map(|s: &str| s.trim().to_lowercase())
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

        let name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.to_string_lossy().into_owned());

        println!(
            "{} {}",
            format!("📁 {}", name).bold(),
            repo.display().to_string().dimmed()
        );

        let hits = scan_repo(repo, &terms);

        if hits.is_empty() {
            println!("   {}\n", "✓ no matches".green());
        } else {
            repos_with_hits += 1;
            total_hits += hits.len();
            println!("   {}\n", format!("⚠  {} hit(s)", hits.len()).red().bold());

            for hit in &hits {
                println!("   {}  {}", hit.date.dimmed(), hit.hash.cyan().bold());
                println!("   {} {}", "branch │".magenta(), hit.branches.join("  "));
                println!("   {} {}", "file   │".dimmed(), hit.file);
                println!("   {} {}", "hit    │".yellow(), hit.line.trim());
                println!();
            }
        }
    }

    // Summary footer
    println!("{}", "━".repeat(56));
    println!("{}", "SUMMARY".bold());
    println!("  repos scanned  : {}", total_repos.to_string().bold());

    if repos_with_hits > 0 {
        println!(
            "  repos with hits : {}",
            repos_with_hits.to_string().red().bold()
        );
        println!(
            "  total hits      : {}",
            total_hits.to_string().red().bold()
        );
        println!("\n  {}", "to remove a commit from history:".dimmed());
        println!("  {}", "git rebase -i <hash>^".dimmed());
    } else {
        println!("  {}", "✓ all clean".green().bold());
    }

    println!();
}

// ── internal types ────────────────────────────────────────────────────────────

struct Hit {
    hash: String,
    date: String,
    file: String,
    line: String,
    branches: Vec<String>,
}

// ── core logic ────────────────────────────────────────────────────────────────

/// Scan one repository for sensitive terms across its full commit history.
fn scan_repo(repo: &Path, terms: &[String]) -> Vec<Hit> {
    let repo_str = match repo.to_str() {
        Some(s) => s,
        None => return Vec::new(),
    };

    let output = match Command::new("git")
        .args([
            "-C", repo_str,
            "log", "--all", "-p",
            "--format=COMMIT:%H|%ad", // full hash + author date
            "--date=short",
        ])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut hits: Vec<Hit> = Vec::new();
    // Track (hash|file|line) combos we've already recorded to avoid duplicates.
    // The same line can appear in multiple diffs if a commit touches many files.
    let mut seen: HashSet<String> = HashSet::new();

    // State we carry forward as we parse lines top-to-bottom
    let mut current_hash = String::new();
    let mut current_date = String::new();
    let mut current_file = String::new();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("COMMIT:") {
            // "COMMIT:<full_hash>|<date>"
            let (hash, date) = rest.split_once('|').unwrap_or((rest, ""));
            current_hash = hash.to_string();
            current_date = date.to_string();
            current_file = String::new(); // reset for each commit
        } else if line.starts_with("diff --git ") {
            // "diff --git a/path/to/file b/path/to/file"
            // Extract the "b/" side — that's the current path of the file.
            current_file = line.split(" b/").nth(1).unwrap_or("").to_string();
        } else {
            // Check if this line contains any of the sensitive terms
            let lower = line.to_lowercase();
            if terms.iter().any(|term| lower.contains(term.as_str())) {
                let dedup_key = format!("{}|{}|{}", current_hash, current_file, line.trim());

                // `HashSet::insert` returns false if the value was already present
                if seen.insert(dedup_key) {
                    let branches = get_branches(repo, &current_hash);
                    hits.push(Hit {
                        hash: current_hash.clone(),
                        date: current_date.clone(),
                        file: current_file.clone(),
                        line: line.to_string(),
                        branches,
                    });
                }
            }
        }
    }

    hits
}

/// Look up which branches contain a given commit hash.
fn get_branches(repo: &Path, hash: &str) -> Vec<String> {
    Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap_or("."),
            "branch", "-a", "--contains", hash,
        ])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                // Strip the "* " prefix git puts on the current branch
                .map(|l| l.trim_start_matches(|c: char| c == '*' || c == ' ').to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default() // if git fails, just return an empty Vec
}
