//! `slu itag` - interactive tag explorer (TUI). The tags level of the drill,
//! entered directly: this repo's tags marked against its remote, the commits
//! each one added since the tag before it, and the diffs under those.
//!
//! git keeps no record of which tags a remote has, so the remote is asked with
//! `git ls-remote` once the screen is up. The marks stay blank until it
//! answers, and a remote that cannot be reached says so on the pane's title.

use crate::app::App;
use crate::git::git_capture;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu itag needs an interactive terminal - use `git tag` instead");
        std::process::exit(1);
    }

    // Anchor at the repo root: git reports file paths root-relative, so a diff
    // asked for from a subdirectory would come up blank.
    let repo =
        git_capture(".", &["rev-parse", "--show-toplevel"]).unwrap_or_else(|| ".".to_string());

    match App::at_tags(repo) {
        Some(app) => app.run("itag"),
        None => {
            eprintln!("slu itag: no tags here (or not a git repo) - `git tag <name>` makes one");
            std::process::exit(1);
        }
    }
}
