//! sluuz — git search and multi-repo management CLI
//!
//! Subcommands:
//!   sluuz search <pattern>   Search git history for commits that added/removed a string
//!   sluuz scan   [path]      Scan repositories for leaked secrets / sensitive terms
//!   sluuz status [path]      Show working-tree state across all repos under a path
//!   sluuz fetch  [path]      Fetch (and optionally fast-forward) all repos in parallel
//!   sluuz branches [path]    Find merged, deletable branches across all repos

mod commands;
mod git;
mod history;

use clap::{Parser, Subcommand};

/// Shown at the bottom of `sluuz --help` so the common invocations are visible
/// without digging into each subcommand's help.
const EXAMPLES: &str = "\x1b[1mExamples:\x1b[0m
  sluuz search -r api_key        Find a string across every repo, all branches
  sluuz scan -t aws,token        Audit repos for custom secret terms
  sluuz status --dirty           Show only repos with uncommitted/unpushed work
  sluuz fetch --pull             Fetch all repos and fast-forward where safe
  sluuz branches                 List merged branches that are safe to delete

Run `sluuz <command> --help` for options specific to a command.";

// `derive` lets clap generate all the argument parsing boilerplate from annotations.
#[derive(Parser)]
#[command(
    name = "sluuz",
    version,
    about = "Git search and multi-repo management tools",
    after_help = EXAMPLES
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
    Status(commands::status::Args),
    /// Fetch (and optionally fast-forward) all repos under a path in parallel
    Fetch(commands::fetch::Args),
    /// Find merged, safe-to-delete branches across all repos
    Branches(commands::branches::Args),
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Scan(args) => commands::scan::run(args),
        Cmd::Status(args) => commands::status::run(args),
        Cmd::Fetch(args) => commands::fetch::run(args),
        Cmd::Branches(args) => commands::branches::run(args),
    }
}
