//! `slu self` - manage the installed binary itself.
//!
//! `self update` shells out to `cargo install sluuz --force`, the crate name even when the command you type is shorter.
//! `self check` asks the registry for the latest release through `cargo search`,
//! so there is no HTTP client in the dependency tree and the answer comes from
//! the same registry `cargo install` would pull from.
//!
//! Windows cannot overwrite a running `.exe`, so cargo's final move would fail
//! with "Access is denied". The running `slu.exe` is renamed aside first
//! (Windows allows renaming, just not overwriting, a running binary), which
//! frees the path for cargo. If the install fails it is moved back, so the user
//! is never left without a binary.

use colored::Colorize;
use std::io::{self, Write};
use std::process::Command;

const CRATE: &str = "sluuz";

#[derive(clap::Subcommand)]
pub enum Cmd {
    /// Reinstall the latest release from crates.io
    ///   -y   skip the confirmation prompt
    #[command(verbatim_doc_comment)]
    Update(UpdateArgs),
    /// Ask crates.io whether a newer release exists, without installing anything
    Check,
}

#[derive(clap::Args)]
pub struct UpdateArgs {
    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub yes: bool,
}

pub fn run(cmd: Cmd) {
    match cmd {
        Cmd::Update(args) => update(args),
        Cmd::Check => check(),
    }
}

fn update(args: UpdateArgs) {
    if !args.yes && !confirm() {
        println!("{}", "Aborted.".dimmed());
        return;
    }

    println!(
        "{} {}\n",
        "Updating sluuz via".dimmed(),
        "cargo install sluuz --force".bold()
    );

    // On Windows, free the running exe's path so cargo can replace it. The
    // binding is cfg'd out elsewhere rather than held as a unit value, since a
    // unit let is a lint and an `#[allow]` is never the answer.
    #[cfg(windows)]
    let token = begin_self_replace();

    match Command::new("cargo")
        .args(["install", CRATE, "--force"])
        .status()
    {
        Ok(status) if status.success() => {
            println!("\n{}", "✓ sluuz is up to date.".green());
        }
        Ok(status) => {
            #[cfg(windows)]
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
            #[cfg(windows)]
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

/// Compare the installed version with the newest one on crates.io. Nothing is
/// downloaded or written, so this is safe to run on a machine you do not want to
/// change.
fn check() {
    let current = env!("CARGO_PKG_VERSION");
    match latest() {
        Ok(latest) if newer(&latest, current) => {
            println!(
                "{} {} {}",
                format!("sluuz {latest}").bold(),
                "is available, you have".dimmed(),
                current.bold()
            );
            println!("{} {}", "run".dimmed(), "slu self update".bold());
        }
        Ok(_) => println!(
            "{} {}",
            format!("sluuz {current}").bold(),
            "is the latest release.".dimmed()
        ),
        Err(e) => {
            eprintln!("{} {e}", "slu: could not reach crates.io:".red());
            std::process::exit(1);
        }
    }
}

/// `cargo search` prints `sluuz = "X.Y.Z"    # description` for an exact name
/// match, which is the whole reason no HTTP client is needed here.
fn latest() -> Result<String, String> {
    let out = Command::new("cargo")
        .args(["search", CRATE, "--limit", "1"])
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let prefix = format!("{CRATE} = \"");
    text.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .and_then(|rest| rest.split('"').next())
        .map(str::to_string)
        .ok_or_else(|| format!("the registry did not list `{CRATE}`"))
}

/// Compare dotted versions field by field, so `0.10.0` correctly beats `0.9.9`
/// where a plain string compare would not.
fn newer(a: &str, b: &str) -> bool {
    let fields = |v: &str| {
        v.split(['.', '-'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    fields(a) > fields(b)
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

// -- self-replacement (Windows only) ------------------------------------------

#[cfg(windows)]
type ReplaceToken = Option<(std::path::PathBuf, std::path::PathBuf)>;

/// Rename the running `slu.exe` to `slu.exe.old`, freeing its path for cargo.
/// Returns the (original, renamed) paths so we can undo on failure. A leftover
/// `.old` from a previous update is removed first (its process is gone, so it is
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

/// If the install failed, move the renamed binary back so the user keeps a
/// working `slu`. cargo only creates the new exe on success, so if the original
/// path is empty we restore ours. On non-Windows this is a no-op.
#[cfg(windows)]
fn undo_self_replace(token: &ReplaceToken) {
    if let Some((exe, old)) = token
        && !exe.exists()
    {
        let _ = std::fs::rename(old, exe);
    }
}

#[cfg(test)]
mod tests {
    use super::newer;

    /// The check compares field by field, because a plain string compare puts
    /// `0.9.9` above `0.10.0` and would tell everyone they are up to date.
    #[test]
    fn a_newer_release_is_recognised_field_by_field() {
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(newer("0.4.2", "0.4.1"));
        assert!(!newer("0.4.1", "0.4.1"));
        assert!(!newer("0.4.0", "0.4.1"));
        assert!(!newer("0.9.9", "0.10.0"));
    }
}
