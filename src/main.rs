//! sluuz - a git superset. Binary: `slu`.
//!
//! Every git command passes straight through (`slu commit`, `slu push`, `slu
//! log`), and the extra verbs add cross-repo / history superpowers, each with an
//! interactive twin that `slu --help` lists as its own group.
//!
//! This file is the clap `Cmd` enum, the hand-built help template and the
//! dispatch match; what you can run is `slu --help`, which renders from the
//! manifest, those doc comments and `AFTER`, and is the only copy of that
//! list.

mod app;
mod commands;
mod git;
mod history;
mod tui;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::process::Command;

/// Shown at the bottom of `slu --help`: the one shape the command list can't
/// convey - that everything else is just git - and then what a script can and
/// cannot expect from stdout.
const AFTER: &str = concat!(
    "\
Ways to run it (not subcommands):
  slu <git command>   real git, passed straight through (`slu commit -m \"fix\"`, `slu push`)

A passthrough is git itself, so the output and the exit code are git's own.
sluuz's own verbs print a table for people rather than data, except
`completions`, whose stdout is the script.
Run `slu <command> --help` for a command's details.",
    "\n\n",
    env!("CARGO_PKG_REPOSITORY"),
    "\ncontributors: ",
    env!("CARGO_PKG_AUTHORS"),
);

/// `-V` stays a bare version string for scripts; `--version` spells out the
/// license, where it lives, and who's contributed. Every field comes from
/// Cargo.toml, so none of it can drift from the manifest.
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n",
    env!("CARGO_PKG_LICENSE"),
    "  ",
    env!("CARGO_PKG_REPOSITORY"),
    "\ncontributors: ",
    env!("CARGO_PKG_AUTHORS"),
);

// `derive` lets clap generate all the argument parsing boilerplate from annotations.
#[derive(Parser)]
// The app is `sluuz` (shown in --version); the command you type is the shorter
// `slu` (shown in usage via bin_name).
#[command(
    name = "sluuz",
    bin_name = "slu",
    version,
    long_version = LONG_VERSION,
    about,
    after_help = AFTER,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

// Each enum variant holds its own Args struct, defined in the subcommand's
// module. `verbatim_doc_comment` keeps the second line (the options) on its own
// line in `slu --help` instead of being collapsed into the description.
#[derive(Subcommand)]
enum Cmd {
    /// Search history for a string (pickaxe)  <PATTERN>
    ///   -r        search every repo under the cwd
    ///   -l N      max commits shown per repo (20)
    #[command(verbatim_doc_comment)]
    Search(commands::search::Args),
    /// Scan repos for sensitive terms - secrets, tokens  [PATH]
    ///   -t TERMS  custom comma-separated terms
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Scan(commands::scan::Args),
    /// Show working-tree state across all repos  [PATH]
    ///   --dirty   only repos needing attention
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Repos(commands::repos::Args),
    /// Fetch (and optionally fast-forward) all repos  [PATH]
    ///   --pull    fast-forward the branch where safe
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Sync(commands::sync::Args),
    /// Find finished branches (upstream gone), safe to delete  [PATH]
    ///   -a        include already-clean repos
    ///   -p        drop remote branches the remote no longer has
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Tidy(commands::tidy::Args),
    /// Run any git command in every repo  <ARGS>...
    ///   slu each pull --ff-only
    #[command(verbatim_doc_comment)]
    Each(commands::each::Args),
    /// Show a prettier history view (aligned log)
    ///   -a        include all branches
    ///   -g        show git's commit graph
    ///   -n N      max commits (30)
    #[command(verbatim_doc_comment)]
    Trace(commands::trace::Args),
    /// Print a completion script that reuses git's own  <bash|zsh|fish>
    ///   --add     append the loader to your shell's rc file for you
    #[command(verbatim_doc_comment)]
    Completions(commands::completions::Args),
    /// Manage sluuz itself: `self update` reinstalls, `self check` looks for a newer release
    #[command(name = "self", subcommand)]
    Selfie(commands::selfcmd::Cmd),
    /// Interactive history search across repos (TUI)  [PATH]
    ///   type terms in the bar, enter runs the search
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Iscan(commands::iscan::Args),
    /// Interactive repo explorer (TUI)  [PATH]
    ///   repos → branches → commits → diff, enter drills in
    ///   --dirty   start on repos with uncommitted work
    ///   -d N      directory depth to scan (3)
    #[command(verbatim_doc_comment)]
    Irepos(commands::irepos::Args),
    /// Interactive branch explorer (TUI)
    ///   every branch with its push state · d deletes, s/S sync
    ///   -g        start on the finished ones (upstream gone)
    ///   -r        start on the remote's
    #[command(verbatim_doc_comment)]
    Ibranch(commands::ibranch::Args),
    /// Interactive tag explorer (TUI)
    ///   every tag against the remote · enter opens what went into it, d deletes
    #[command(verbatim_doc_comment)]
    Itag(commands::itag::Args),
    /// Interactive log explorer (TUI)  [PATH]...
    ///   commits on top, the selected commit's diff below
    ///   -a        include all branches
    ///   -n N      commits to load (200)
    #[command(verbatim_doc_comment)]
    Ilog(commands::ilog::Args),
    /// Interactive git status - stage/unstage + diffs (TUI)
    ///   this repo · ←→/hl tabs · s/u/space stage
    #[command(verbatim_doc_comment)]
    Istatus(commands::istatus::Args),
    /// Any other command is passed straight through to git
    #[command(external_subcommand)]
    Git(Vec<String>),
}

fn main() {
    // The command list is rendered here rather than by clap, which has no way to
    // give one subcommand a different heading from another. Everything in it
    // still comes from clap's own metadata, so it cannot drift from the
    // commands that actually exist.
    let mut built = Cli::command();
    built.build();
    let template = help_template(&built);
    let matches = Cli::command().help_template(template).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };

    match cli.command {
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Scan(args) => commands::scan::run(args),
        Cmd::Repos(args) => commands::repos::run(args),
        Cmd::Sync(args) => commands::sync::run(args),
        Cmd::Tidy(args) => commands::tidy::run(args),
        Cmd::Each(args) => commands::each::run(args),
        Cmd::Trace(args) => commands::trace::run(args),
        Cmd::Ilog(args) => commands::ilog::run(args),
        Cmd::Irepos(args) => commands::irepos::run(args),
        Cmd::Ibranch(args) => commands::ibranch::run(args),
        Cmd::Itag(args) => commands::itag::run(args),
        Cmd::Iscan(args) => commands::iscan::run(args),
        Cmd::Istatus(args) => commands::istatus::run(args),
        Cmd::Selfie(cmd) => commands::selfcmd::run(cmd),
        Cmd::Completions(args) => commands::completions::run(args),
        Cmd::Git(args) => passthrough(&args),
    }
}

/// `slu --help` with the commands split in two: the ones that print and exit,
/// then the ones that take over the terminal. The pairs line up across the two
/// groups (`scan`/`iscan`, `repos`/`irepos`, `trace`/`ilog`, `tag`/`itag`),
/// which is the shape of the tool.
fn help_template(cmd: &clap::Command) -> String {
    let subs: Vec<&clap::Command> = cmd.get_subcommands().filter(|c| !c.is_hide_set()).collect();
    // One column width across both groups, or the two lists would not line up
    // with each other and the pairing would stop being visible.
    let width = subs
        .iter()
        .map(|c| c.get_name().chars().count())
        .max()
        .unwrap_or(0);
    let (tui, plain): (Vec<&&clap::Command>, Vec<&&clap::Command>) =
        subs.iter().partition(|c| is_interactive(c));

    // Rendering the list by hand means also painting it by hand: clap only
    // styles what it renders itself, and every other crate's help has bold
    // headings and bold command names. The styles come from clap so this list
    // is painted with the same palette as the `{options}` block below it.
    let styles = cmd.get_styles();
    let header = styles.get_header();
    let literal = styles.get_literal();

    let mut list = String::new();
    for (heading, group) in [("Commands:", plain), ("Interactive (TUI):", tui)] {
        list.push_str(&format!("{header}{heading}{header:#}\n"));
        for c in group {
            list.push_str(&entry(c, width, literal));
        }
        list.push('\n');
    }

    format!(
        "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{list}{header}Options:{header:#}\n{{options}}{{after-help}}\n"
    )
}

/// One command's row: its name, then its description, whose extra lines are
/// indented to the same column so a multi-line one still reads as one entry.
fn entry(cmd: &clap::Command, width: usize, literal: &clap::builder::styling::Style) -> String {
    let about = cmd.get_about().map(ToString::to_string).unwrap_or_default();
    let mut lines = about.lines();
    let name = cmd.get_name();
    // The name is padded outside its own styling: escape codes are zero-width
    // to a reader and full-width to `{:<width$}`, so styling the padded string
    // would push every description off the column.
    let pad = " ".repeat(width - name.chars().count());
    let mut row = format!(
        "  {literal}{name}{literal:#}{pad}  {}\n",
        lines.next().unwrap_or("")
    );
    for line in lines {
        row.push_str(&format!("  {:<width$}  {line}\n", ""));
    }
    row
}

/// A command is interactive when its name starts with `i`, which is already the
/// rule the verbs are named by (`iscan`, `irepos`, `ilog`, …) rather than a
/// second list kept beside them for the help to read. Nothing here can drift:
/// a command named to the house rule is grouped by it.
fn is_interactive(cmd: &clap::Command) -> bool {
    cmd.get_name().starts_with('i')
}

/// Forward to real `git`, inheriting stdio so editors, pagers, prompts, and
/// colors all work, then exit with git's own status code.
fn passthrough(args: &[String]) -> ! {
    match Command::new("git").args(args).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("slu: could not run git: {e}");
            std::process::exit(127);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, help_template};
    use clap::CommandFactory;

    #[test]
    fn every_interactive_command_is_listed_under_its_own_heading() {
        // `slu --help` promises two groups, and which group a command lands in
        // is decided by the `i` its name is meant to start with. Reading the
        // rendered list back is what catches a command named against that rule,
        // which would otherwise only be visible to whoever next read the help.
        let mut built = Cli::command();
        built.build();
        let rendered = help_template(&built);
        let (plain, interactive) = rendered
            .split_once("Interactive (TUI):")
            .expect("the list is split into two groups");

        let literal = built.get_styles().get_literal();
        for sub in built.get_subcommands() {
            let name = sub.get_name();
            // The name carries its own styling, so anchoring on it also pins
            // that the hand-rendered list is painted like clap's own.
            let row = format!("\n  {literal}{name}{literal:#}");
            let (half, group) = if name.starts_with('i') {
                (interactive, "interactive")
            } else {
                (plain, "plain")
            };
            assert!(
                half.contains(&row),
                "`{name}` is missing from the {group} group"
            );
        }
    }
}
