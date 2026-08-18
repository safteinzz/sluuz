//! `slu irepos` — interactive repo explorer (TUI). The top of the drill: every
//! repo under a path, then its branches, its commits, and their diffs.
//!
//! Repos (top, j/k) preview the selected one's branches below (Ctrl-j/k), with
//! the same state flags `slu repos` prints, so the list doubles as a dashboard.
//! `h`/`l` slide the scope between dirty, all, and unpushed. Enter drills in,
//! Esc steps back, and quits here.

use crate::app::{repo_scope, App};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// Base directory to search for repos (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// How many directory levels deep to look for repos
    #[arg(short, long, default_value_t = 3)]
    pub depth: usize,

    /// Start in the "dirty" scope (only repos with uncommitted work)
    #[arg(long)]
    pub dirty: bool,
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu irepos needs an interactive terminal — use `slu repos` for plain output");
        return;
    }

    match App::at_repos(&args.path, args.depth, repo_scope(args.dirty)) {
        Some(app) => app.run("irepos"),
        None => eprintln!("no git repos found under {}", args.path.display()),
    }
}
