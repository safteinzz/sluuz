//! `slu each <git args>` - run any git command in every repo under the current
//! directory, in parallel.
//!
//! This generalizes the dedicated multi-repo commands: `slu each pull --ff-only`,
//! `slu each "log --oneline -1"`, `slu each switch main`. Whatever you'd type
//! after `git`, it runs in each repo and prints the output grouped per repo.

use crate::git::{display_name, find_repos, git_run};
use colored::Colorize;
use rayon::prelude::*;
use std::path::Path;

#[derive(clap::Args)]
pub struct Args {
    /// The git command (and its args) to run in every repo,
    /// e.g. `slu each pull --ff-only`
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    pub args: Vec<String>,
}

struct Outcome {
    name: String,
    ok: bool,
    output: String,
}

pub fn run(args: Args) {
    let repos = find_repos(Path::new("."), 3);
    if repos.is_empty() {
        println!("{}", "No git repos found.".dimmed());
        return;
    }

    println!("{} {}\n", "git".dimmed(), args.args.join(" ").bold());

    let argv: Vec<&str> = args.args.iter().map(String::as_str).collect();

    let mut outcomes: Vec<Outcome> = repos
        .par_iter()
        .filter_map(|repo| {
            let name = display_name(repo);
            let repo_str = repo.to_str()?;
            let (ok, output) = git_run(repo_str, &argv);
            Some(Outcome { name, ok, output })
        })
        .collect();

    outcomes.sort_by(|a, b| a.name.cmp(&b.name));

    let mut failed = 0usize;
    for o in &outcomes {
        let mark = if o.ok {
            "✓".green()
        } else {
            failed += 1;
            "✗".red()
        };
        println!("{} {}", mark, format!("━━━ {}", o.name).cyan().bold());
        if o.output.is_empty() {
            println!("  {}", "(no output)".dimmed());
        } else {
            for line in o.output.lines() {
                println!("  {}", line);
            }
        }
        println!();
    }

    let summary = format!("{} repos · {} failed", outcomes.len(), failed);
    if failed > 0 {
        println!("{}", summary.red());
    } else {
        println!("{}", summary.dimmed());
    }
}
