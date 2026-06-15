//! `slu update` — update sluuz itself to the latest release on crates.io.
//!
//! Just shells out to `cargo install sluuz --force`. Note the crate is `sluuz`
//! even though the command you type is `slu`, so that's what cargo reinstalls.

use colored::Colorize;
use std::io::{self, Write};
use std::process::Command;

#[derive(clap::Args)]
pub struct Args {
    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub yes: bool,
}

pub fn run(args: Args) {
    if !args.yes && !confirm() {
        println!("{}", "Aborted.".dimmed());
        return;
    }

    println!(
        "{} {}\n",
        "Updating sluuz via".dimmed(),
        "cargo install sluuz --force".bold()
    );

    match Command::new("cargo")
        .args(["install", "sluuz", "--force"])
        .status()
    {
        Ok(status) if status.success() => {
            println!("\n{}", "✓ sluuz is up to date.".green());
        }
        Ok(status) => {
            eprintln!("\n{}", "✗ update failed.".red());
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("{} {e}", "slu: could not run cargo:".red());
            eprintln!(
                "{}",
                "is cargo installed and on your PATH? (https://rustup.rs)".dimmed()
            );
            std::process::exit(127);
        }
    }
}

/// Ask the user to confirm. Defaults to No, so a bare Enter cancels.
fn confirm() -> bool {
    print!(
        "{} {} ",
        "Update sluuz to the latest release via cargo?".bold(),
        "[y/N]".dimmed()
    );
    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}
