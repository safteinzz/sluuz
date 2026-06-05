//! Shared git-history search used by both `search` and `scan`.
//!
//! Both commands work the same way under the hood: run git's pickaxe
//! (`git log -S <term>`) across all branches, then parse the patch output into
//! commits → files → matching diff lines.
//!
//! Pickaxe is used rather than grepping the rendered diff because it also
//! detects changes inside binary/encrypted blobs, where `git log -p` prints no
//! +/- text lines at all. That is the difference that lets `scan` catch secrets
//! committed in encrypted `.env` files.

use std::collections::HashMap;
use std::process::Command;

/// One commit that matched, with the files that changed and their diff lines.
pub struct CommitMatch {
    /// Abbreviated hash (%h) — for display.
    pub short: String,
    /// Full hash (%H) — stable key for merging and `branch --contains`.
    pub full: String,
    /// Author date, `YYYY-MM-DD` (%ad with --date=short).
    pub date: String,
    /// Commit subject line (%s).
    pub subject: String,
    pub files: Vec<FileMatch>,
}

/// One file within a matching commit.
pub struct FileMatch {
    pub path: String,
    /// (is_addition, line). Empty for binary/encrypted files — git shows no
    /// +/- lines for those, but pickaxe still flags the file as changed.
    pub lines: Vec<(bool, String)>,
}

/// Run git's pickaxe for each term across all branches and merge the results.
/// `case_insensitive` controls both the pickaxe match and the line filtering.
///
/// One `git log` is run per term (pickaxe `-S` takes a single string), and the
/// per-term results are merged by commit hash so a commit matching several
/// terms appears once.
pub fn pickaxe(repo: &str, terms: &[String], case_insensitive: bool) -> Vec<CommitMatch> {
    let mut order: Vec<String> = Vec::new();
    let mut by_hash: HashMap<String, CommitMatch> = HashMap::new();

    for term in terms {
        let output = match run_log(repo, term, case_insensitive) {
            Some(o) => o,
            None => continue,
        };
        for commit in parse_log(&output, term, case_insensitive) {
            match by_hash.get_mut(&commit.full) {
                Some(existing) => merge(existing, commit),
                None => {
                    order.push(commit.full.clone());
                    by_hash.insert(commit.full.clone(), commit);
                }
            }
        }
    }

    // Preserve first-seen order (roughly newest-first from the first term's run).
    order.into_iter().filter_map(|h| by_hash.remove(&h)).collect()
}

/// List the branches (local and remote) that contain `hash`.
pub fn branches_for(repo: &str, hash: &str) -> Vec<String> {
    Command::new("git")
        .args([
            "-C",
            repo,
            "branch",
            "-a",
            "--contains",
            hash,
            "--format=%(refname:short)",
        ])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

// ── internals ───────────────────────────────────────────────────────────────

/// Run `git log --all -S <term> -p` for one term, returning raw stdout.
fn run_log(repo: &str, term: &str, case_insensitive: bool) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["-C", repo, "log", "--all"]);
    if case_insensitive {
        cmd.arg("-i"); // --regexp-ignore-case, also applies to -S
    }
    cmd.args([
        "-S", term,                                   // pickaxe: count of term changed
        "-F",                                         // term is a fixed string, not regex
        "--date=short",
        "--pretty=format:COMMIT:%h|%H|%ad|||%s",
        "-p",                                         // include the patch
    ]);
    let output = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse raw `git log -p` output into a list of CommitMatch structs.
fn parse_log(output: &str, term: &str, case_insensitive: bool) -> Vec<CommitMatch> {
    let needle = if case_insensitive {
        term.to_lowercase()
    } else {
        term.to_string()
    };

    let mut commits: Vec<CommitMatch> = Vec::new();
    let mut current: Option<CommitMatch> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("COMMIT:") {
            // A new commit header: flush the previous one first.
            if let Some(c) = current.take() {
                commits.push(c);
            }
            // Format: "COMMIT:<short>|<full>|<date>|||<subject>"
            let (meta, subject) = rest.split_once("|||").unwrap_or((rest, ""));
            let mut parts = meta.splitn(3, '|');
            current = Some(CommitMatch {
                short: parts.next().unwrap_or("").to_string(),
                full: parts.next().unwrap_or("").to_string(),
                date: parts.next().unwrap_or("").to_string(),
                subject: subject.to_string(),
                files: Vec::new(),
            });
        } else if let Some(c) = current.as_mut() {
            if let Some(path) = parse_diff_header(line) {
                // A new file diff begins. Without --pickaxe-all, every file git
                // shows here is one where the term's count changed.
                c.files.push(FileMatch {
                    path,
                    lines: Vec::new(),
                });
            } else if let Some(file) = c.files.last_mut() {
                // Capture +/- lines that contain the term, attaching them to the
                // file whose diff we're currently inside. Skip the +++/--- headers.
                let is_add = line.starts_with('+') && !line.starts_with("+++");
                let is_del = line.starts_with('-') && !line.starts_with("---");

                if is_add || is_del {
                    let hay = if case_insensitive {
                        line.to_lowercase()
                    } else {
                        line.to_string()
                    };
                    if hay.contains(&needle) {
                        file.lines.push((is_add, line.to_string()));
                    }
                }
            }
        }
    }

    if let Some(c) = current.take() {
        commits.push(c);
    }

    commits
}

/// Extract the file path from a `diff --git a/<path> b/<path>` header line.
/// Uses the `b/` (destination) path; for a deletion git still records the real
/// name there. Returns None for any other line.
fn parse_diff_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    // Split on the last " b/" so paths containing " b/" earlier don't trip us up.
    let idx = rest.rfind(" b/")?;
    Some(rest[idx + 3..].to_string())
}

/// Fold the files/lines of `from` into `into` (same commit matched by another
/// term). Dedups lines by text so a line containing two terms isn't repeated.
fn merge(into: &mut CommitMatch, from: CommitMatch) {
    for file in from.files {
        if let Some(existing) = into.files.iter_mut().find(|e| e.path == file.path) {
            for line in file.lines {
                if !existing.lines.iter().any(|l| l.1 == line.1) {
                    existing.lines.push(line);
                }
            }
        } else {
            into.files.push(file);
        }
    }
}
