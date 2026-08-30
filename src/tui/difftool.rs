//! Handing the terminal to the user's `git difftool` and taking it back.
//!
//! The TUI must be fully torn down first, or an interactive tool like vimdiff
//! draws into a screen ratatui still owns. That teardown is also why a failure
//! here needs care: git writes its complaint to the normal screen, and we take
//! that screen away in the same instant by re-entering the alternate one. So we
//! keep a copy of what the tool said and hand it back as a modal - without it,
//! a difftool that cannot run looks exactly like a key that did nothing.

use super::widgets::Modal;
use super::{pop_keyboard_enhancement, push_keyboard_enhancement};
use crate::git::git_capture;
use ratatui::DefaultTerminal;
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};

/// git's magic empty-tree hash - the "before" side for a root commit that has no
/// parent to diff against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// How many lines of the tool's complaint the modal keeps: enough to explain a
/// failure, never enough to fill the screen with it.
const MAX_ERR_LINES: usize = 40;

/// What came back from handing the terminal over.
pub enum DiffTool {
    /// The tool ran and had nothing to say.
    Quiet,
    /// A one-liner for the status line: something happened, but there is
    /// nothing to read about it.
    Note(String),
    /// It failed and said why. The caller shows this until it is dismissed.
    Failed(Modal),
}

/// Is a difftool configured? `git difftool` uses `diff.tool`, falling back to
/// `merge.tool`. We check up front so that, if neither is set, we can show a
/// helpful message instead of tearing down the TUI only for git to error out.
fn has_difftool(dir: &str) -> bool {
    ["diff.tool", "merge.tool"]
        .iter()
        .any(|k| git_capture(dir, &["config", k]).is_some_and(|v| !v.is_empty()))
}

/// Open one commit's file in the user's difftool, comparing it against its first
/// parent (or the empty tree for a root commit) - mirroring what `git show`
/// displays.
pub fn difftool_commit(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    dir: &str,
    hash: &str,
    path: &str,
) -> DiffTool {
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
/// cleanly - no screen flicker - when no difftool is configured.
pub fn run_difftool(
    terminal: &mut DefaultTerminal,
    enhanced: bool,
    dir: &str,
    args: &[&str],
) -> DiffTool {
    if !has_difftool(dir) {
        return DiffTool::Failed(Modal::new(
            " no difftool configured ",
            "Enter hands the file to `git difftool`, but neither diff.tool nor \
             merge.tool is set, so there is nothing to open it with.\n\n\
             Set one and try again, for example:\n\n    \
             git config --global diff.tool vimdiff\n\n\
             `git difftool --tool-help` lists the tools your git can drive.",
        ));
    }

    // Hand the terminal back to the external tool.
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    let outcome = handoff(dir, args);

    // Re-enter the TUI exactly as run() first set it up.
    *terminal = ratatui::init();
    if enhanced {
        push_keyboard_enhancement();
    }
    let _ = terminal.clear();

    match outcome {
        Err(e) => DiffTool::Failed(Modal::new(
            " difftool would not start ",
            format!("Could not run git difftool: {e}"),
        )),
        Ok((status, said)) if !status.success() => {
            let code = match status.code() {
                Some(c) => c.to_string(),
                None => "a signal".to_string(),
            };
            if said.is_empty() {
                // A tool that ran fine and was quit with a non-zero code (vim's
                // `:cq`) has nothing to explain, so it doesn't get a box.
                DiffTool::Note(format!("difftool exited with status {code}"))
            } else {
                DiffTool::Failed(Modal::new(
                    " difftool failed ",
                    format!("git difftool exited with status {code}.\n\n{said}"),
                ))
            }
        }
        Ok(_) => DiffTool::Quiet,
    }
}

/// Run the tool with the terminal handed over, keeping a copy of its stderr.
/// A thread drains the pipe while the tool runs, so a chatty one can't fill it
/// and hang waiting for someone to read.
fn handoff(dir: &str, args: &[&str]) -> io::Result<(ExitStatus, String)> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("difftool")
        .arg("-y")
        .args(args)
        .stderr(Stdio::piped())
        .spawn()?;

    let mut pipe = child.stderr.take().expect("stderr is piped");
    let drain = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    });
    let status = child.wait()?;
    let said = drain.join().unwrap_or_default();
    Ok((status, tail(&String::from_utf8_lossy(&said))))
}

/// The tail of what the tool printed: blanks dropped and consecutive repeats
/// collapsed, because `difftool -y` walks a file at a time and says the same
/// thing about each one.
fn tail(said: &str) -> String {
    let mut lines: Vec<&str> = said
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    lines.dedup();
    let start = lines.len().saturating_sub(MAX_ERR_LINES);
    lines[start..].join("\n")
}
