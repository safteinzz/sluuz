//! `slu iremote` - interactive remote explorer (TUI). The remotes level of the
//! drill, entered directly: this repo's remotes with the checked-out branch
//! measured against each, the branches each one holds, and below those the
//! usual commits and diffs.
//!
//! `p` pushes the checked-out branch to the remote under the cursor, `s`/`S`
//! sync that one remote, and `c`/`e`/`d` add, change and remove remotes.

use crate::app::App;
use crate::git::git_capture;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!(
            "slu iremote needs an interactive terminal - use `slu remote -v` or `slu branch -vv` instead"
        );
        std::process::exit(1);
    }

    let Some(repo) = git_capture(".", &["rev-parse", "--show-toplevel"]) else {
        eprintln!("slu iremote: not a git repo - run it inside one");
        std::process::exit(1);
    };
    App::at_remotes(repo).run("iremote");
}
