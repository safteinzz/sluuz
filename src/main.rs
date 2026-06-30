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
//!   slu trace              A prettier history view (does not shadow `git log`)
//!   slu ilog               Interactive log explorer (TUI)
//!   slu ibranch            Interactive branch explorer (TUI)
//!   slu completions <sh>   Print a tab-completion script (reuses git's)

mod commands;
mod git;
mod history;
mod tui;

use clap::{Parser, Subcommand};
use std::process::Command;

/// Shown at the bottom of `slu --help`: the one thing the command list can't
/// convey — that everything else is just git.
const PASSTHROUGH: &str = "Anything else is real git, passed straight through: slu commit -m \"fix\", slu push, slu rebase …
Run `slu <command> --help` for the full detail of any command.";

// `derive` lets clap generate all the argument parsing boilerplate from annotations.
#[derive(Parser)]
// The app is `sluuz` (shown in --version); the command you type is the shorter
// `slu` (shown in usage via bin_name).
#[command(
    name = "sluuz",
    bin_name = "slu",
    version,
    about = "git, but it sleuths — a git superset with cross-repo & history superpowers",
    after_help = PASSTHROUGH,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

// Each enum variant holds its own Args struct, defined in the subcommand's
// module. `verbatim_doc_comment` keeps the second line (the options) on its own
// line in `slu --help` instead of being collapsed into the description.
#[derive(Subcommand)]
enum Cmd {
    /// Search history for a string (pickaxe) <pattern>
    ///   -r        search every repo under the cwd
    ///   -l N      max commits shown per repo (20)
    #[command(verbatim_doc_comment)]
    Search(commands::search::Args),
    /// Scan repos for sensitive terms — secrets, tokens [path]
    ///   -t terms  custom comma-separated terms
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Scan(commands::scan::Args),
    /// Working-tree state across all repos [path]
    ///   --dirty   only repos needing attention
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Repos(commands::repos::Args),
    /// Fetch (and optionally fast-forward) all repos [path]
    ///   --pull    fast-forward the branch where safe
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Sync(commands::sync::Args),
    /// Find merged, safe-to-delete branches [path]
    ///   -a        include already-clean repos
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Tidy(commands::tidy::Args),
    /// Run any git command in every repo  (e.g. slu each pull --ff-only)
    Each(commands::each::Args),
    /// A prettier history view (aligned log)
    ///   -a        include all branches
    ///   -g        show git's commit graph
    ///   -n N      max commits (30)
    #[command(verbatim_doc_comment)]
    Trace(commands::trace::Args),
    /// Interactive log explorer (TUI)
    ///   -a        include all branches
    ///   -n N      commits to load (200)
    #[command(verbatim_doc_comment)]
    Ilog(commands::ilog::Args),
    /// Interactive branch explorer (TUI)
    ///   -r        remotes only
    ///   -a        local + remote
    #[command(verbatim_doc_comment)]
    Ibranch(commands::ibranch::Args),
    /// Update sluuz to the latest release
    ///   -y        skip the confirmation prompt
    #[command(verbatim_doc_comment)]
    Update(commands::update::Args),
    /// Print a tab-completion script <bash|zsh|fish>
    ///   --add     append the loader to your shell's rc file for you
    ///   reuses git's own completion (branches, refs, flags)
    #[command(verbatim_doc_comment)]
    Completions(commands::completions::Args),
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
        Cmd::Trace(args) => commands::trace::run(args),
        Cmd::Ilog(args) => commands::ilog::run(args),
        Cmd::Ibranch(args) => commands::ibranch::run(args),
        Cmd::Update(args) => commands::update::run(args),
        Cmd::Completions(args) => commands::completions::run(args),
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
