//! gt — git-tools CLI
//!
//! Subcommands:
//!   gt search <pattern>   Search git history for commits that added/removed a string
//!   gt scan   [path]      Scan repositories for leaked secrets / sensitive terms
//!   gt status [path]      Show working-tree state across all repos under a path
//!   gt fetch  [path]      Fetch (and optionally fast-forward) all repos in parallel
//!   gt branches [path]    Find merged, deletable branches across all repos

mod commands;
mod git;
mod history;

use clap::{Parser, Subcommand};

// `derive` lets clap generate all the argument parsing boilerplate from annotations.
#[derive(Parser)]
#[command(name = "gt", version, about = "Git search and management tools")]
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
