//! gt — git-tools CLI
//!
//! Subcommands:
//!   gt search <pattern>   Search git history for commits that added/removed a string
//!   gt scan   [path]      Scan repositories for leaked secrets / sensitive terms

mod commands;
mod git;

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
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Scan(args) => commands::scan::run(args),
    }
}
