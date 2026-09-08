//! Loading and highlighting a diff off the UI thread.
//!
//! `git diff` on a big file plus the syntect pass is long enough to stall a held
//! `j`, and both the four-level browser and `istatus` move a cursor over files.
//! They differ only in which git command produces the raw text, so that is the
//! one thing a caller passes in.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::tui::highlight::{DiffContext, RenderedDiff, prepare_diff};

/// How long that may take before a pane admits it is loading. A wait nobody
/// notices says nothing, or the word blinks on every keypress in a small repo.
const SLOW: Duration = Duration::from_millis(120);

pub struct DiffFeed {
    tx: Sender<(u64, RenderedDiff)>,
    rx: Receiver<(u64, RenderedDiff)>,
    /// The newest request. An answer carrying anything else belongs to a row the
    /// cursor has already left.
    seq: u64,
    loading: bool,
    since: Option<Instant>,
}

impl Default for DiffFeed {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        DiffFeed {
            tx,
            rx,
            seq: 0,
            loading: false,
            since: None,
        }
    }
}

impl DiffFeed {
    /// Ask for a diff. `raw` runs on the worker and returns what git said,
    /// paired with where to read the file's two sides when a hunk starts too far
    /// down the file to highlight on its own.
    pub fn request(&mut self, raw: impl FnOnce() -> (String, DiffContext) + Send + 'static) {
        self.seq += 1;
        self.loading = true;
        self.since = Some(Instant::now());
        let (seq, tx) = (self.seq, self.tx.clone());
        std::thread::spawn(move || {
            let (text, ctx) = raw();
            let _ = tx.send((seq, prepare_diff(&text, ctx)));
        });
    }

    /// Nothing is coming: the cursor is on a row with no diff to show.
    pub fn idle(&mut self) {
        self.seq += 1;
        self.loading = false;
        self.since = None;
    }

    /// The newest prepared diff, if one has arrived since the last call.
    pub fn take(&mut self) -> Option<RenderedDiff> {
        let mut newest = None;
        while let Ok((seq, prepared)) = self.rx.try_recv() {
            if seq == self.seq {
                newest = Some(prepared);
                self.loading = false;
            }
        }
        newest
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    pub fn slow(&self) -> bool {
        self.loading && self.since.is_some_and(|t| t.elapsed() >= SLOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prepared diff carrying nothing: the sequence is what these test.
    fn empty() -> RenderedDiff {
        RenderedDiff::default()
    }

    /// A request that asks git for nothing.
    fn nothing() -> (String, DiffContext) {
        (String::new(), DiffContext::default())
    }

    #[test]
    fn an_answer_to_a_row_you_have_left_is_dropped() {
        // The whole point of the sequence: a held `j` starts a load per row it
        // passes, and the slow one must not land on the row you stopped on.
        let mut feed = DiffFeed::default();
        let (first, second) = (feed.tx.clone(), feed.tx.clone());
        feed.request(nothing); // seq 1
        feed.request(nothing); // seq 2, supersedes it

        let _ = first.send((1, empty()));
        assert!(feed.take().is_none(), "the answer to seq 1 is not wanted");

        let _ = second.send((2, empty()));
        assert!(feed.take().is_some(), "the answer to seq 2 is");
        assert!(!feed.loading(), "and the pane has stopped waiting");
    }

    #[test]
    fn a_row_with_no_diff_stops_the_waiting() {
        let mut feed = DiffFeed::default();
        feed.request(nothing);
        assert!(feed.loading());
        feed.idle();
        assert!(
            !feed.loading(),
            "nothing is coming, so nothing is waited for"
        );
        assert!(!feed.slow());
    }
}
