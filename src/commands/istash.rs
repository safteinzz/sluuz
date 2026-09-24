//! `slu istash`: the interactive stash explorer (TUI). The commits level of the
//! drill, walking the stash: every stash, the files it holds, and their diffs.
//! `a` applies the stash under the cursor, `p` pops it, `d` drops it behind a
//! typed gate. See `app/stashes.rs`.

use crate::app::App;
use crate::git::git_capture;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu istash: needs an interactive terminal, so run `git stash list` instead");
        std::process::exit(1);
    }

    // Anchor at the repo root: git reports file paths root-relative, so a diff
    // asked for from a subdirectory would come up blank.
    let repo =
        git_capture(".", &["rev-parse", "--show-toplevel"]).unwrap_or_else(|| ".".to_string());

    match App::at_stashes(repo) {
        Some(app) => app.run("istash"),
        None => {
            eprintln!(
                "slu istash: nothing is stashed here (or this is not a git repo), so run `git stash` first"
            );
            std::process::exit(1);
        }
    }
}
