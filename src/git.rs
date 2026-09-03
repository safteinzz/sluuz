//! Shared git utility functions used across subcommands.
//!
//! `load` holds the queries behind the interactive views. They live here rather
//! than under `tui/` because reading git is not terminal work, and a plain
//! command may need the same query.

pub mod load;

use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// ASCII unit separator - a field delimiter that won't appear in git's output,
/// so `--format` strings can split on it without quoting anything.
pub const SEP: char = '\u{1f}';

/// Find all git repositories under `base`, searching up to `max_depth` levels deep.
/// Returns each repo's root directory (the parent of its `.git` directory).
pub fn find_repos(base: &Path, max_depth: usize) -> Vec<PathBuf> {
    WalkDir::new(base)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok()) // skip permission errors
        .filter(|entry| entry.file_name() == ".git" && entry.file_type().is_dir())
        .filter_map(|entry| entry.path().parent().map(Path::to_path_buf)) // .git → repo root
        .collect()
}

/// One repo's working-tree state: what `slu repos` prints as a row and what the
/// repos level of the interactive drill shows.
pub struct RepoStatus {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub has_upstream: bool,
    pub dirty: usize,
    pub ahead: usize,
    pub behind: usize,
    /// `origin`'s URL, shortened to `host:owner/repo`. Empty when the repo has
    /// no origin.
    pub origin: String,
}

impl RepoStatus {
    /// Whether the repo needs attention: uncommitted, unpushed, or unpulled.
    pub fn needs_attention(&self) -> bool {
        self.dirty > 0 || self.ahead > 0 || self.behind > 0
    }

    /// Uncommitted work in the working tree.
    pub fn is_dirty(&self) -> bool {
        self.dirty > 0
    }

    /// Commits that exist here and on no remote.
    pub fn is_unpushed(&self) -> bool {
        self.ahead > 0
    }
}

/// Read one repo's status via `git status --porcelain=2 --branch`, which packs
/// branch, upstream, ahead/behind, and changed files into one machine-readable
/// listing.
pub fn repo_status(repo: &Path) -> RepoStatus {
    let path = repo.to_string_lossy().into_owned();
    let out = git_capture(&path, &["status", "--porcelain=2", "--branch"]).unwrap_or_default();

    let mut branch = "(detached)".to_string();
    let mut has_upstream = false;
    let mut ahead = 0usize;
    let mut behind = 0usize;
    let mut dirty = 0usize;

    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest.to_string();
        } else if line.starts_with("# branch.upstream ") {
            has_upstream = true;
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // "+<ahead> -<behind>"
            for tok in rest.split_whitespace() {
                if let Some(a) = tok.strip_prefix('+') {
                    ahead = a.parse().unwrap_or(0);
                } else if let Some(b) = tok.strip_prefix('-') {
                    behind = b.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            // Any non-header line is a changed/untracked entry.
            dirty += 1;
        }
    }

    // Only `origin`: it is what all but a handful of repos call their remote,
    // and a second column of URLs would cost more room than it is worth.
    let origin = git_capture(&path, &["config", "--get", "remote.origin.url"])
        .map(|u| short_remote(&u))
        .unwrap_or_default();

    RepoStatus {
        name: display_name(repo),
        path,
        branch,
        has_upstream,
        dirty,
        ahead,
        behind,
        origin,
    }
}

/// Shorten a remote URL to `host:owner/repo`, the part that identifies it.
/// Handles the scp-like form git uses for SSH (`git@host:owner/repo.git`) and
/// real URLs (`https://host/owner/repo.git`, `ssh://git@host:22/owner/repo`).
/// Anything else (a filesystem path, mostly) comes back as it went in, since
/// there is no host to pull out of it.
pub fn short_remote(url: &str) -> String {
    let url = url.trim();
    let url = url.strip_suffix('/').unwrap_or(url);
    let stripped = url.strip_suffix(".git").unwrap_or(url);

    // A real URL: <scheme>://[user@]host[:port]/path
    if let Some((_, rest)) = stripped.split_once("://") {
        let rest = rest.rsplit('@').next().unwrap_or(rest); // drop any userinfo
        let Some((hostport, path)) = rest.split_once('/') else {
            return stripped.to_string();
        };
        // Keep the host, drop the port: it identifies the server, not the repo.
        let host = hostport.split(':').next().unwrap_or(hostport);
        return format!("{host}:{path}");
    }

    // The scp-like form: [user@]host:path - but not a Windows drive (C:\…).
    if let Some((hostpart, path)) = stripped.split_once(':')
        && !path.starts_with('\\')
        && !path.starts_with('/')
        && hostpart.contains('.')
    {
        let host = hostpart.rsplit('@').next().unwrap_or(hostpart);
        return format!("{host}:{path}");
    }

    // Not a URL at all, so it is a filesystem path: a mirror, a bare repo on a
    // share, a submodule's relative origin. Those identify by their tail, and
    // the head is usually a prefix every repo in the tree shares, so keep the
    // last two components.
    let parts: Vec<&str> = stripped
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.len() > 2 {
        return format!("…/{}", parts[parts.len() - 2..].join("/"));
    }
    stripped.to_string()
}

/// Get a human-readable repo name from its path (its directory name).
pub fn display_name(repo: &Path) -> String {
    repo.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| repo.to_string_lossy().into_owned())
}

/// Run `git -C <repo> <args>` and return trimmed stdout, or None if git fails
/// or exits non-zero. For read-only queries where you only want the output.
pub fn git_capture(repo: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Like `git_capture`, but returns stdout **untrimmed**. Use it whenever leading
/// whitespace is data - notably `git status --porcelain`, where the first column
/// is a space when a file has no staged change. Trimming would eat that space on
/// the first record and shift the whole line ("Cargo.lock" → "argo.lock").
pub fn git_capture_raw(repo: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Does a fetch in this repo delete remote-tracking refs the remote no longer
/// has? `tidy` reads `[gone]`, which git only sets once that ref is actually
/// absent, so a repo that never prunes hides exactly the branches `tidy` is for
/// and reports "nothing to clean up" as though it were a fact.
///
/// `remote.<name>.prune` overrides `fetch.prune`, so it is asked first and its
/// answer is final when it has one.
pub fn prunes_on_fetch(repo: &str) -> bool {
    for key in ["remote.origin.prune", "fetch.prune"] {
        if let Some(v) = git_capture(repo, &["config", "--get", "--type=bool", key]) {
            return v == "true";
        }
    }
    false
}

/// The first non-blank line of a program's output - what a failure is reported
/// as when the whole of it would not fit.
pub fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Run `git -C <repo> <args>` and return (success, combined stdout+stderr).
/// For commands like fetch/pull where progress goes to stderr and you want to
/// report what happened regardless of exit status.
pub fn git_run(repo: &str, args: &[&str]) -> (bool, String) {
    match Command::new("git").arg("-C").arg(repo).args(args).output() {
        Ok(output) => {
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            (output.status.success(), combined.trim().to_string())
        }
        Err(e) => (false, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{git_capture, prunes_on_fetch, short_remote};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A throwaway git repo under the system temp dir, named so two runs cannot
    /// collide, and deleted when it goes out of scope - including when an
    /// assertion panics, or a failing test would leave the machine dirtier than
    /// it found it. Nothing outside the temp directory is ever touched.
    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new(tag: &str) -> TempRepo {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("sluuz-{tag}-{stamp}"));
            fs::create_dir_all(&dir).expect("temp dir");
            git_capture(dir.to_str().expect("utf-8 temp path"), &["init", "-q"]).expect("git init");
            TempRepo(dir)
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("utf-8 temp path")
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn set(repo: &str, key: &str, value: &str) {
        git_capture(repo, &["config", key, value]).expect("git config");
    }

    #[test]
    fn a_remotes_own_prune_setting_beats_the_general_one() {
        // Which way round this goes decides whether `tidy` tells you your list
        // may be short. Both keys are set in every case, so whatever the
        // machine running the suite has in its own git config cannot change the
        // answer.
        let dir = TempRepo::new("prune");
        let repo = dir.path();

        set(repo, "fetch.prune", "false");
        set(repo, "remote.origin.prune", "true");
        assert!(prunes_on_fetch(repo), "the remote's own setting wins");

        set(repo, "fetch.prune", "true");
        set(repo, "remote.origin.prune", "false");
        assert!(!prunes_on_fetch(repo), "and it wins in the other direction");
    }

    #[test]
    fn the_ssh_form_git_prints_by_default() {
        assert_eq!(
            short_remote("git@github.com:safteinzz/sluuz.git"),
            "github.com:safteinzz/sluuz"
        );
        assert_eq!(
            short_remote("git@gitlab.com:safteinzz/sluuz.git"),
            "gitlab.com:safteinzz/sluuz"
        );
    }

    #[test]
    fn https_urls_lose_the_scheme_and_the_dot_git() {
        assert_eq!(
            short_remote("https://gitlab.com/safteinzz/sluuz.git"),
            "gitlab.com:safteinzz/sluuz"
        );
        assert_eq!(
            short_remote("https://github.com/safteinzz/sluuz"),
            "github.com:safteinzz/sluuz"
        );
    }

    #[test]
    fn a_url_loses_its_credentials_and_its_port() {
        assert_eq!(
            short_remote("https://token@github.com/o/r.git"),
            "github.com:o/r"
        );
        assert_eq!(
            short_remote("ssh://git@host.example:2222/o/r.git"),
            "host.example:o/r"
        );
    }

    #[test]
    fn a_self_hosted_path_keeps_every_segment() {
        assert_eq!(
            short_remote("https://git.example.com/team/group/sub/repo.git"),
            "git.example.com:team/group/sub/repo"
        );
    }

    #[test]
    fn a_local_remote_is_named_by_its_tail() {
        // Every repo in a tree of mirrors shares the head of this path, so the
        // end is the only part that says which one it is.
        assert_eq!(
            short_remote("/home/me/projects/sluuz/test-playground/crates/vibox.git"),
            "…/crates/vibox"
        );
        assert_eq!(short_remote("../mirror/repo.git"), "…/mirror/repo");
        // Nothing was dropped here, so there is no ellipsis to earn.
        assert_eq!(short_remote("/srv/repo.git"), "/srv/repo");
    }

    #[test]
    fn something_unparseable_comes_back_as_it_went_in() {
        assert_eq!(short_remote("weird"), "weird");
        assert_eq!(short_remote(""), "");
    }
}
