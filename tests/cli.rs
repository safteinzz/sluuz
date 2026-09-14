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
