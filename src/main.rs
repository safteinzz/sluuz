//! sluuz — a git superset. Binary: `slu`.
//!
//! Every git command passes straight through (`slu commit`, `slu push`, `slu
//! log`), and these extra verbs add cross-repo / history superpowers:
//!   slu search <pattern>   Pickaxe a string through all branches/repos
//!   slu scan   [path]      Audit repositories for leaked secrets
//!   slu repos  [path]      Working-tree state across all repos under a path
//!   slu sync   [path]      Fetch (and optionally fast-forward) all repos
//!   slu tidy   [path]      Find merged, deletable branches across all repos
//!   slu each   <git args>  Run any git command in every repo

mod commands;
mod git;
mod history;

use clap::{Parser, Subcommand};
use std::process::Command;

/// Shown at the bottom of `slu --help` so the common invocations are visible
/// without digging into each subcommand's help.
const EXAMPLES: &str = "\x1b[1mExamples:\x1b[0m
  slu commit -m \"fix\"       Plain git — passed straight through
  slu push                   …so are push, log, rebase, diff, everything
  slu search -r api_key      Sleuth a string across every repo, all branches
  slu scan -t aws,token      Audit repos for custom secret terms
  slu repos --dirty          Show only repos with uncommitted/unpushed work
  slu sync --pull            Fetch all repos and fast-forward where safe
  slu tidy                   List merged branches that are safe to delete
  slu each pull --ff-only    Run any git command in every repo

Run `slu <command> --help` for options specific to a superpower command.";

// `derive` lets clap generate all the argument parsing boilerplate from annotations.
#[derive(Parser)]
#[command(
    name = "slu",
    version,
    about = "git, but it sleuths — a git superset with cross-repo & history superpowers",
    after_help = EXAMPLES,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

// Each enum variant holds its own Args struct, defined in the subcommand's module.
#[derive(Subcommand)]
enum Cmd {
    /// Search git history for commits that added or removed a string
    Search(commands::search::Args),
    /// Scan repositories for sensitive terms (passwords, secrets, tokens)
    Scan(commands::scan::Args),
    /// Show working-tree state (branch, dirty, ahead/behind) across all repos
    Repos(commands::repos::Args),
    /// Fetch (and optionally fast-forward) all repos under a path in parallel
    Sync(commands::sync::Args),
    /// Find merged, safe-to-delete branches across all repos
    Tidy(commands::tidy::Args),
    /// Run any git command in every repo under the current directory
    Each(commands::each::Args),
    /// Any other command is passed straight through to git
    #[command(external_subcommand)]
    Git(Vec<String>),
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Scan(args) => commands::scan::run(args),
        Cmd::Repos(args) => commands::repos::run(args),
        Cmd::Sync(args) => commands::sync::run(args),
        Cmd::Tidy(args) => commands::tidy::run(args),
        Cmd::Each(args) => commands::each::run(args),
        Cmd::Git(args) => passthrough(&args),
    }
}

/// Forward to real `git`, inheriting stdio so editors, pagers, prompts, and
/// colors all work, then exit with git's own status code.
fn passthrough(args: &[String]) -> ! {
    match Command::new("git").args(args).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("slu: could not run git: {e}");
            std::process::exit(127);
        }
    }
}
