//! Handing the terminal to the user's `git difftool` and taking it back.
//!
//! The TUI must be fully torn down first, or an interactive tool like vimdiff
//! draws into a screen ratatui still owns.

use super::{pop_keyboard_enhancement, push_keyboard_enhancement};
use crate::git::git_capture;
use ratatui::DefaultTerminal;
use std::process::Command;

/// git's magic empty-tree hash — the "before" side for a root commit that has no
/// parent to diff against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Is a difftool configured? `git difftool` uses `diff.tool`, falling back to
/// `merge.tool`. We check up front so that, if neither is set, we can show a
/// helpful message instead of tearing down the TUI only for git to error out.
fn has_difftool(dir: &str) -> bool {
    ["diff.tool", "merge.tool"]
        .iter()
        .any(|k| git_capture(dir, &["config", k]).is_some_and(|v| !v.is_empty()))
}

/// Open one commit's file in the user's difftool, comparing it against its first
/// parent (or the empty tree for a root commit) — mirroring what `git show`
/// displays. Returns a status line for the caller to surface.
pub fn difftool_commit(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    dir: &str,
    hash: &str,
    path: &str,
) -> String {
    let base = if commit_has_parent(dir, hash) {
        format!("{hash}^")
    } else {
        EMPTY_TREE.to_string()
    };
    run_difftool(terminal, enhanced, dir, &[&base, hash, "--", path])
}

fn commit_has_parent(dir: &str, hash: &str) -> bool {
    git_capture(dir, &["rev-list", "--parents", "-n", "1", hash])
        .map(|s| s.split_whitespace().count() > 1)
        .unwrap_or(false)
}

/// Suspend the TUI, run `git -C <dir> difftool -y <args>` with the terminal
/// handed over (so a terminal tool like vimdiff works), then re-enter. Bails
/// cleanly — no screen flicker — when no difftool is configured. Returns "" on
/// success, else a short message to show the user.
pub fn run_difftool(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    dir: &str,
    args: &[&str],
) -> String {
    if !has_difftool(dir) {
        return "no difftool set — configure one: git config --global diff.tool <tool>".to_string();
    }

    // Hand the terminal back to the external tool.
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("difftool")
        .arg("-y")
        .args(args)
        .status();

    // Re-enter the TUI exactly as run() first set it up.
    *terminal = ratatui::init();
    if enhanced {
        push_keyboard_enhancement();
    }
    let _ = terminal.clear();

    match status {
        Ok(s) if s.success() => String::new(),
        Ok(_) => "difftool exited with an error".to_string(),
        Err(e) => format!("could not launch difftool: {e}"),
    }
}
