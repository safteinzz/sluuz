//! Shared git utility functions used across subcommands.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Find all git repositories under `base`, searching up to `max_depth` levels deep.
/// Returns each repo's root directory (the parent of its `.git` directory).
pub fn find_repos(base: &Path, max_depth: usize) -> Vec<PathBuf> {
    WalkDir::new(base)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())                                         // skip permission errors
        .filter(|entry| entry.file_name() == ".git" && entry.file_type().is_dir())
        .filter_map(|entry| entry.path().parent().map(Path::to_path_buf))       // .git → repo root
        .collect()
}
