//! `slu trace` - a better history view (a prettier `log`).
//!
//! Deliberately NOT named `log`: sluuz never shadows a real git command, so
//! `slu log` still passes straight through to git. `trace` is the enhanced view:
//! an aligned, colorized list (hash · full date · relative time · subject ·
//! author). `--graph` keeps git's own commit graph (which we don't try to
//! out-render) but enriches the per-commit line.

use crate::git::{git_capture, SEP};
use colored::Colorize;

/// Committer name is capped at this width (long names get truncated).
const NAME_MAX: usize = 18;

#[derive(clap::Args)]
pub struct Args {
    /// Maximum number of commits to show
    #[arg(short = 'n', long, default_value_t = 30)]
    pub number: usize,

    /// Include commits from all branches
    #[arg(short, long)]
    pub all: bool,

    /// Show git's commit graph (topology) instead of the flat aligned view
    #[arg(short, long)]
    pub graph: bool,
}

pub fn run(args: Args) {
    if args.graph {
        graph_log(args.all, args.number);
    } else {
        pretty_log(args.all, args.number);
    }
}

/// ASCII record separator - marks where git's graph prefix ends and our fields
/// begin, so we can keep git's graph rail and still render our own columns.
const REC: char = '\u{1e}';

/// One parsed commit row.
struct Row {
    hash: String,
    date: String,
    rel: String,
    committer: String,
    subject: String,
}

/// The flat aligned view (no graph).
fn pretty_log(all: bool, limit: usize) {
    let n = limit.to_string();
    let fmt = format!("--pretty=format:%h{SEP}%ad{SEP}%ar{SEP}%cn{SEP}%s");
    let mut args = vec!["log", "-n", &n, "--date=format:%Y-%m-%d %H:%M", &fmt];
    if all {
        args.push("--all");
    }

    let out = match git_capture(".", &args) {
        Some(o) if !o.is_empty() => o,
        _ => {
            eprintln!("{}", "no commits (or not a git repo)".dimmed());
            return;
        }
    };

    let rows: Vec<Row> = out.lines().filter_map(parse_row).collect();
    let rel_w = rel_width(rows.iter());
    let width = term_width();

    for r in &rows {
        print_row("", r, rel_w, width);
    }
}

/// The graph view - we keep git's own commit graph (the `│ ├─╮` rails) but
/// re-render each commit line through our renderer so the columns match the
/// flat view exactly. Graph-only lines (`|\`, `|/`) are passed through as-is.
fn graph_log(all: bool, limit: usize) {
    let n = limit.to_string();
    let fmt = format!("--pretty=format:{REC}%h{SEP}%ad{SEP}%ar{SEP}%cn{SEP}%s");
    let mut args = vec![
        "log",
        "--graph",
        "--color=never",
        "--date=format:%Y-%m-%d %H:%M",
        "-n",
        &n,
        &fmt,
    ];
    if all {
        args.push("--all");
    }

    let out = match git_capture(".", &args) {
        Some(o) if !o.is_empty() => o,
        _ => {
            eprintln!("{}", "no commits (or not a git repo)".dimmed());
            return;
        }
    };

    // Each commit line is "<graph rail>\x1e<fields>"; graph-only lines have no \x1e.
    let parsed: Vec<(String, Option<Row>)> = out
        .lines()
        .map(|line| match line.find(REC) {
            Some(i) => (line[..i].to_string(), parse_row(&line[i + REC.len_utf8()..])),
            None => (line.to_string(), None),
        })
        .collect();

    let rel_w = rel_width(parsed.iter().filter_map(|(_, r)| r.as_ref()));
    let width = term_width();

    for (rail, row) in &parsed {
        match row {
            Some(r) => print_row(rail, r, rel_w, width),
            None => println!("{rail}"),
        }
    }
}

/// Parse one tab-of-separators line into a Row.
fn parse_row(line: &str) -> Option<Row> {
    let mut f = line.split(SEP);
    Some(Row {
        hash: f.next()?.to_string(),
        date: f.next()?.to_string(),
        rel: f.next()?.to_string(),
        committer: f.next()?.to_string(),
        subject: f.next().unwrap_or("").to_string(),
    })
}

/// Width of the parenthesized relative-time column: widest "(... ago)".
fn rel_width<'a>(rows: impl Iterator<Item = &'a Row>) -> usize {
    rows.map(|r| r.rel.chars().count() + 2).max().unwrap_or(0)
}

/// Render one commit: `<rail>hash  date  (relative)  <committer> subject`.
/// The relative token is right-aligned as a whole, so padding lands *before*
/// the `(` - "(2 days ago)" / "   (10 days ago)". Subject is truncated to fit.
fn print_row(rail: &str, r: &Row, rel_w: usize, width: usize) {
    let who = truncate(&r.committer, NAME_MAX);

    // Plain (uncolored) width of everything left of the subject, so truncation
    // accounts for the graph rail too.
    let left = rail.chars().count() + 7 + 2 + 16 + 2 + rel_w + 2 + who.chars().count() + 3;
    let subject = truncate(&r.subject, width.saturating_sub(left).max(10));

    let hash = format!("{:<7}", r.hash);
    let rel_token = format!("({})", r.rel);
    let rel = format!("{rel_token:>rel_w$}");
    let who = format!("<{who}>");

    println!(
        "{rail}{}  {}  {}  {} {}",
        hash.yellow(),
        r.date.green(),
        rel.magenta(),
        who.blue(),
        subject
    );
}

fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(100)
}

/// Truncate to `max` display columns, adding an ellipsis when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
