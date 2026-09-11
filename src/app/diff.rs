//! The diff level: one file's change, side by side.
//!
//! The raw diff is highlighted once on the way in; scrolling and panning only
//! re-lay-out the spans that pass already produced.

use super::{App, Level};
use crate::git::load::{commit_diff_ctx, load_diff_raw};
use crate::tui::difftool::{DiffTool, difftool_commit};
use crate::tui::highlight::{RenderedDiff, render_prepared};
use crate::tui::{clamp_hscroll, clamp_scroll};
use ratatui::DefaultTerminal;

/// Rows a Ctrl-j/k moves the diff, and columns a Ctrl-h/l pans it, before a held
/// key multiplies them.
const SCROLL_STEP: u16 = 3;
const PAN_STEP: u16 = 8;

impl App {
    /// Open the selected file of the selected commit into the diff level.
    ///
    /// `git show` plus syntect on a thousand-line file is long enough to freeze
    /// a held `j` through the file list, so the work goes to a thread and the
    /// pane opens empty. A move that supersedes this one bumps the sequence,
    /// and the answer to the old one is dropped when it lands.
    pub(super) fn open_diff(&mut self) {
        let (Some(hash), Some(path)) = (
            self.commit_hash().map(str::to_string),
            self.file_path().map(str::to_string),
        ) else {
            return;
        };
        let repo = self.repo.clone();
        self.dfeed.request(move || {
            let raw = load_diff_raw(&repo, &hash, &path);
            (raw, commit_diff_ctx(&repo, &hash, &path))
        });
        self.prepared = RenderedDiff::default();
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.diff = render_prepared(&self.prepared, self.width, 0);
        self.level = Level::Diff;
    }

    /// Re-lay-out the prepared diff at the current width and pan.
    pub(super) fn relayout_diff(&mut self) {
        self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
    }

    pub(super) fn scroll_diff(&mut self, down: bool, steps: usize) {
        let by = SCROLL_STEP.saturating_mul(steps as u16);
        self.diff_scroll = if down {
            self.diff_scroll.saturating_add(by)
        } else {
            self.diff_scroll.saturating_sub(by)
        };
    }

    pub(super) fn pan_diff(&mut self, right: bool, steps: usize) {
        let by = PAN_STEP.saturating_mul(steps as u16);
        self.diff_hscroll = if right {
            clamp_hscroll(
                self.diff_hscroll.saturating_add(by),
                self.prepared.max_line(),
                self.prepared.cell_width(self.width),
            )
        } else {
            self.diff_hscroll.saturating_sub(by)
        };
        self.relayout_diff();
    }

    /// Keep the last line from scrolling up past the top of the viewport.
    pub(super) fn clamp_diff(&mut self) {
        self.diff_scroll = clamp_scroll(self.diff_scroll, self.diff.lines.len(), self.diff_rows);
    }

    /// Hand the file to the user's `git difftool`, then take the terminal back.
    pub(super) fn difftool(&mut self, terminal: &mut DefaultTerminal) {
        let (Some(hash), Some(path)) = (self.commit_hash(), self.file_path()) else {
            return;
        };
        let (hash, path) = (hash.to_string(), path.to_string());
        let outcome = difftool_commit(terminal, self.enhanced, &self.repo, &hash, &path);
        self.width = crate::tui::pane_width(terminal);
        match outcome {
            DiffTool::Quiet => {}
            DiffTool::Note(m) => self.set_failed(m),
            DiffTool::Failed(modal) => self.modal = Some(modal),
        }
    }
}
