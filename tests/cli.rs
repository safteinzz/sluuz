//! What the plain commands promise a script: a failure goes to stderr and the
//! exit code says so.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// An empty folder under the system temp dir, deleted when it goes out of
/// scope, including when an assertion panics.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("sluuz-{tag}-{stamp}"));
        fs::create_dir_all(&dir).expect("temp dir");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_scan_that_finds_no_repo_exits_non_zero() {
    let dir = TempDir::new("norepo");
    // Stops git looking above the temp dir, so a machine whose temp dir sits
    // inside a repo cannot hand the scan one.
    let ceiling = dir.0.parent().expect("temp dir has a parent");

    for cmd in ["repos", "sync", "tidy"] {
        let out = Command::new(env!("CARGO_BIN_EXE_slu"))
            .arg(cmd)
            .current_dir(&dir.0)
            .env("GIT_CEILING_DIRECTORIES", ceiling)
            .output()
            .expect("run slu");
        assert!(!out.status.success(), "`slu {cmd}` with no repo exited 0");
        assert!(
            out.stdout.is_empty(),
            "`slu {cmd}` put its failure on stdout"
        );
        assert!(
            !out.stderr.is_empty(),
            "`slu {cmd}` failed without saying why"
        );
    }
}

#[test]
fn a_reader_that_stops_early_ends_the_output_quietly() {
    let dir = TempDir::new("pipe");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&dir.0)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("run git")
            .success();
        assert!(ok, "`git {}` failed", args.join(" "));
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "one",
    ]);

    // What `slu trace | head` does to it, every time: the read end is gone
    // before the first line is written.
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let out = Command::new(env!("CARGO_BIN_EXE_slu"))
        .arg("trace")
        .current_dir(&dir.0)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdout(writer)
        .output()
        .expect("run slu");
    assert!(
        out.status.success(),
        "`slu trace` into a closed pipe exited {:?}",
        out.status.code()
    );
    assert!(
        out.stderr.is_empty(),
        "`slu trace` into a closed pipe complained: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
