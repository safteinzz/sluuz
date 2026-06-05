//! `gt fetch` — fetch (and optionally fast-forward) every repo under a path in
//! parallel.
//!
//! By default it only fetches + prunes, which never touches your working tree.
//! With `--pull` it additionally runs `git pull --ff-only`, which fast-forwards
//! the current branch when it safely can and refuses (rather than merging) when
//! it can't — so it can't create merge commits or conflicts.

use crate::git::{display_name, find_repos, git_run};
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

    /// After fetching, fast-forward the current branch (git pull --ff-only)
    #[arg(long)]
    pub pull: bool,
}

enum State {
    Ok,
    Skip,
    Fail,
}

struct Outcome {
    name: String,
    state: State,
    note: String,
}

pub fn run(args: Args) {
    let repos = find_repos(&args.path, args.depth);

    if repos.is_empty() {
        println!("{}", "No git repos found.".dimmed());
        return;
    }

    println!(
        "{} {} repo(s)…\n",
        if args.pull { "Fetching + pulling" } else { "Fetching" }.dimmed(),
        repos.len()
    );

    let mut outcomes: Vec<Outcome> = repos
        .par_iter()
        .filter_map(|repo| {
            let name = display_name(repo);
            let repo_str = repo.to_str()?;
            Some(process(name, repo_str, args.pull))
        })
        .collect();

    outcomes.sort_by(|a, b| a.name.cmp(&b.name));

    let name_w = outcomes.iter().map(|o| o.name.len()).max().unwrap_or(0);
    let mut failed = 0usize;

    for o in &outcomes {
        let mark = match o.state {
            State::Ok => "✓".green(),
            State::Skip => "–".dimmed(),
            State::Fail => {
                failed += 1;
                "✗".red()
            }
        };
        println!("  {}  {:name_w$}  {}", mark, o.name.bold(), o.note.dimmed());
    }

    println!();
    let summary = format!("{} repos · {} failed", outcomes.len(), failed);
    if failed > 0 {
        println!("{}", summary.red());
    } else {
        println!("{}", summary.dimmed());
    }
}

fn process(name: String, repo: &str, pull: bool) -> Outcome {
    let (ok, out) = git_run(repo, &["fetch", "--all", "--prune"]);
    if !ok {
        return Outcome {
            name,
            state: State::Fail,
            note: first_line(&out),
        };
    }

    if !pull {
        // Fetch output is empty when nothing new arrived.
        let note = if out.is_empty() { "up to date" } else { "fetched" };
        return Outcome {
            name,
            state: State::Ok,
            note: note.to_string(),
        };
    }

    // --pull: try a fast-forward-only pull.
    let (pulled, pout) = git_run(repo, &["pull", "--ff-only"]);
    if pulled {
        let note = if pout.contains("Already up to date") {
            "up to date"
        } else {
            "fast-forwarded"
        };
        return Outcome {
            name,
            state: State::Ok,
            note: note.to_string(),
        };
    }

    // A missing upstream isn't a failure — there's just nothing to pull.
    if pout.contains("no tracking information") {
        return Outcome {
            name,
            state: State::Skip,
            note: "fetched, no upstream to pull".to_string(),
        };
    }

    Outcome {
        name,
        state: State::Fail,
        note: first_line(&pout),
    }
}

/// First non-empty line of git output, for a compact one-line note.
fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}
