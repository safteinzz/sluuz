//! `slu tidy` - find local branches whose upstream is **gone** (the remote
//! branch they tracked was deleted) across every repo under a path, with how
//! long since each was last touched.
//!
//! A gone upstream is the sign a branch is finished - e.g. a merged PR the
//! remote auto-deleted - but not proof its work reached the remote: a commit
//! made after the merge did not. So each one is checked against the remote's
//! main line the way `d` in `slu ibranch` checks it, and only those whose
//! changes are all there are offered for deletion. Branches still alive on the
//! remote, or that never had an upstream, are left alone. This is the
//! non-interactive, multi-repo view; `slu ibranch` is the TUI.

use crate::git::load;
use crate::git::{
    SEP, display_name, find_repos, first_line, git_capture, git_run, prunes_on_fetch,
};
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

    /// Show every repo, including those with nothing to clean up
    #[arg(short, long)]
    pub all: bool,

    /// Drop remote-tracking branches the remote no longer has, so the list is
    /// current rather than as fresh as your last fetch
    #[arg(short, long)]
    pub prune: bool,
}

struct Branch {
    name: String,
    age: String,
    /// Every change on it is already in the remote's main line, so deleting it
    /// loses nothing.
    landed: bool,
}

pub fn run(args: Args) {
    let repos = find_repos(&args.path, args.depth);

    // Asked for fresh refs: prune every repo first, in parallel, since this is
    // one network round trip each and a tree of them is slow enough serially to
    // feel like a hang.
    let failed = if args.prune {
        prune_all(&repos)
    } else {
        Vec::new()
    };

    let mut total_repos = 0usize;
    let mut total_branches = 0usize;
    let mut total_holding = 0usize;
    // A repo that does not prune on fetch can be holding a remote-tracking ref
    // for a branch the remote deleted, which is exactly what hides a branch
    // from this report.
    let mut stale_possible = false;

    for repo in &repos {
        let repo_str = match repo.to_str() {
            Some(s) => s,
            None => continue,
        };
        let name = display_name(repo);
        let current = git_capture(repo_str, &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap_or_else(|| "HEAD".to_string());

        // One repo that does not prune is enough to say so, and this is a
        // `git config` read per repo otherwise - the whole point of `tidy` is
        // that it is cheap enough to run constantly.
        if !stale_possible && !args.prune {
            stale_possible = !prunes_on_fetch(repo_str);
        }

        let gone = gone_branches(repo_str, &current);

        if gone.is_empty() {
            if args.all {
                println!(
                    "{} {}",
                    format!("📁 {}", name).bold(),
                    "nothing to clean up".dimmed()
                );
            }
            continue;
        }

        let (safe, holding): (Vec<&Branch>, Vec<&Branch>) = gone.iter().partition(|b| b.landed);
        total_repos += usize::from(!safe.is_empty());
        total_branches += safe.len();
        total_holding += holding.len();

        println!(
            "{}  {}",
            format!("📁 {}", name).bold(),
            format!("(on {})", current).dimmed()
        );
        let width = gone.iter().map(|b| b.name.len()).max().unwrap_or(0);
        let list = |branches: &[&Branch]| {
            for b in branches {
                println!(
                    "     {}  {}",
                    format!("{:width$}", b.name).yellow(),
                    b.age.dimmed()
                );
            }
        };
        if !safe.is_empty() {
            println!(
                "   {}",
                "upstream gone, changes already on the remote - safe to delete:".dimmed()
            );
            list(&safe);
            let names = safe
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            // `-D`: their changes were just shown to be on the remote, and `-d`
            // refuses any branch that reached it by squash or rebase.
            println!("   {} git branch -D {}", "↳".dimmed(), names.dimmed());
        }
        if !holding.is_empty() {
            println!(
                "   {}",
                "upstream gone, but holding changes the remote does not have - left alone:"
                    .dimmed()
            );
            list(&holding);
            println!(
                "   {} {}",
                "↳".dimmed(),
                "look before deleting: slu ibranch -g".dimmed()
            );
        }
        println!();
    }

    if total_branches == 0 && total_holding == 0 {
        println!(
            "{}",
            "No branches with a gone upstream to clean up.".green()
        );
    } else {
        let mut summary = format!(
            "{} branch(es) across {} repo(s) can be deleted",
            total_branches, total_repos
        );
        if total_holding > 0 {
            summary.push_str(&format!(
                " · {total_holding} hold changes the remote does not have"
            ));
        }
        println!("{}", summary.dimmed());
    }

    // Said out loud rather than assumed: git only marks a branch `[gone]` once
    // the remote-tracking ref is really absent, and a plain `git fetch` never
    // removes one. Without pruning, "nothing to clean up" is not an answer.
    if stale_possible {
        println!(
            "{}",
            "refs are only as fresh as your last prune - run `slu tidy --prune`, or `git config --global fetch.prune true`"
                .dimmed()
        );
    }

    // A prune that did not happen means the report is not the one that was
    // asked for, so it is a failure even though the listing still printed.
    if !failed.is_empty() {
        for (name, err) in &failed {
            eprintln!("slu tidy: {name}: {err}");
        }
        std::process::exit(1);
    }
}

/// Drop every remote-tracking ref the remote no longer has, in every repo,
/// returning the ones that failed with the first line of git's own complaint.
///
/// `git remote prune` rather than `git fetch --prune`: both ask the remote for
/// its ref list, but only the fetch also drags objects down with it, and
/// updating the repo is `slu sync`'s job rather than this report's.
fn prune_all(repos: &[PathBuf]) -> Vec<(String, String)> {
    repos
        .par_iter()
        .filter_map(|repo| {
            let repo_str = repo.to_str()?;
            let failure = prune(repo_str)?;
            Some((display_name(repo), failure))
        })
        .collect()
}

/// Prune every remote this repo has, stopping at the first that fails. None
/// when it worked, or when there are no remotes to ask.
fn prune(repo: &str) -> Option<String> {
    let remotes = git_capture(repo, &["remote"]).unwrap_or_default();
    for remote in remotes.lines().filter(|r| !r.is_empty()) {
        let (ok, out) = git_run(repo, &["remote", "prune", remote]);
        if !ok {
            return Some(first_line(&out));
        }
    }
    None
}

/// Local branches whose upstream is gone (the remote branch they tracked was
/// deleted), excluding the checked-out `current` branch, each with a relative
/// "last commit" age and whether its changes already reached the remote's main
/// line. Same detection as `d` in `slu ibranch`.
///
/// `%(refname)` is stripped to the plain branch name ourselves - `%(refname:short)`
/// would return `heads/v0.2.20` when a tag of the same name exists, which
/// `git branch -d` can't take.
fn gone_branches(repo: &str, current: &str) -> Vec<Branch> {
    let fmt = format!(
        "--format=%(refname){SEP}%(upstream:track){SEP}%(committerdate:relative){SEP}%(upstream:remotename)"
    );
    git_capture(
        repo,
        &["for-each-ref", "--sort=-committerdate", &fmt, "refs/heads"],
    )
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
                let age = f.next().unwrap_or("").to_string();
                let remote = f.next().filter(|r| !r.is_empty()).unwrap_or("origin");
                Some(Branch {
                    landed: landed(repo, name, remote),
                    name: name.to_string(),
                    age,
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Whether deleting `branch` loses nothing: none of its commits are missing
/// from every remote, or all their changes are in `remote`'s main line anyway.
/// A repo whose remote has no main line to compare with reads as not landed.
fn landed(repo: &str, branch: &str, remote: &str) -> bool {
    load::unique_commits(repo, branch) == 0
        || load::trunk(repo, remote).is_some_and(|t| load::landed(repo, branch, &t))
}
