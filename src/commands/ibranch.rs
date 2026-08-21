//! `slu ibranch` - interactive branch explorer (TUI). The branches level of the
//! drill, entered directly: this repo's branches, their commits, and the diffs
//! under those.
//!
//! Branches (top, j/k) preview their commits below (Ctrl-j/k); `h`/`l` slide the
//! scope between local, all, and remote. Enter drills into the commits level,
//! Esc quits, since this is where the app was entered. See `app/` for the rest.
//!
//! Push state is visible at a glance: a branch marked `↑` has no remote yet (or
//! its upstream is gone / it's ahead); commits marked `↑` aren't pushed anywhere.

use crate::app::{branch_scope, App};
use crate::git::git_capture;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {
    /// Start in the "all" scope (local + remote-tracking branches)
    #[arg(short, long)]
    pub all: bool,

    /// Start in the "remote" scope (remote-tracking branches only)
    #[arg(short, long)]
    pub remotes: bool,
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu ibranch needs an interactive terminal - use `slu tidy` / `git branch` instead");
        return;
    }

    // Anchor at the repo root: git reports file paths root-relative, so a diff
    // asked for from a subdirectory would come up blank.
    let repo = git_capture(".", &["rev-parse", "--show-toplevel"]).unwrap_or_else(|| ".".to_string());

    match App::at_branches(repo, branch_scope(args.all, args.remotes)) {
        Some(app) => app.run("ibranch"),
        None => eprintln!("no branches (or not a git repo)"),
    }
}
