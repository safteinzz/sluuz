//! `slu update` - update sluuz itself to the latest release on crates.io.
//!
//! Shells out to `cargo install sluuz --force`. Note the crate is `sluuz` even
//! though the command you type is `slu`, so that's what cargo reinstalls.
//!
//! Windows can't overwrite a running `.exe`, so cargo's final move would fail
//! with "Access is denied". To work around that we rename the running
//! `slu.exe` aside first (Windows allows renaming, just not overwriting, a
//! running binary) - which frees the path for cargo. If the install fails we
//! move it back so the user is never left without a binary.

use colored::Colorize;
use std::io::{self, Write};
use std::process::Command;

#[derive(clap::Args)]
pub struct Args {
    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub yes: bool,
}

// `token` is unit on non-Windows (self-replacement is a Windows-only concern).
#[allow(clippy::let_unit_value)]
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

    // On Windows, free the running exe's path so cargo can replace it.
    let token = begin_self_replace();

    match Command::new("cargo")
        .args(["install", "sluuz", "--force"])
        .status()
    {
        Ok(status) if status.success() => {
            println!("\n{}", "✓ sluuz is up to date.".green());
        }
        Ok(status) => {
            undo_self_replace(&token);
            eprintln!("\n{}", "✗ update failed.".red());
            if cfg!(windows) {
                eprintln!(
                    "{}",
                    "If it still can't replace slu.exe, run this in a fresh terminal:".dimmed()
                );
                eprintln!("    {}", "cargo install sluuz --force".bold());
            }
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) => {
            undo_self_replace(&token);
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

// ── self-replacement (Windows only) ───────────────────────────────────────────

#[cfg(windows)]
type ReplaceToken = Option<(std::path::PathBuf, std::path::PathBuf)>;
#[cfg(not(windows))]
type ReplaceToken = ();

/// Rename the running `slu.exe` to `slu.exe.old`, freeing its path for cargo.
/// Returns the (original, renamed) paths so we can undo on failure. A leftover
/// `.old` from a previous update is removed first (its process is gone, so it's
/// now unlocked). On non-Windows this is a no-op.
#[cfg(windows)]
fn begin_self_replace() -> ReplaceToken {
    let exe = std::env::current_exe().ok()?;
    let mut old_os = exe.clone().into_os_string();
    old_os.push(".old");
    let old = std::path::PathBuf::from(old_os);

    let _ = std::fs::remove_file(&old); // clear any prior leftover
    match std::fs::rename(&exe, &old) {
        Ok(()) => Some((exe, old)),
        Err(_) => None, // couldn't free the path; let cargo try anyway
    }
}

#[cfg(not(windows))]
fn begin_self_replace() -> ReplaceToken {}

/// If the install failed, move the renamed binary back so the user keeps a
/// working `slu`. cargo only creates the new exe on success, so if the original
/// path is empty we restore ours. On non-Windows this is a no-op.
#[cfg(windows)]
fn undo_self_replace(token: &ReplaceToken) {
    if let Some((exe, old)) = token {
        if !exe.exists() {
            let _ = std::fs::rename(old, exe);
        }
    }
}

#[cfg(not(windows))]
fn undo_self_replace(_token: &ReplaceToken) {}
