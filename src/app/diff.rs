//! The diff level: one file's change, side by side.
//!
//! The raw diff is highlighted once on the way in; scrolling and panning only
//! re-lay-out the spans that pass already produced.

use super::{App, Level};
use crate::git::load::load_diff_raw;
use crate::tui::difftool::difftool_commit;
use crate::tui::highlight::{prepare_diff, render_prepared};
use crate::tui::{clamp_hscroll, clamp_scroll, half_page, pane_height};
use ratatui::DefaultTerminal;

/// Rows a Ctrl-j/k moves the diff, and columns a Ctrl-h/l pans it.
const SCROLL_STEP: u16 = 3;
const PAN_STEP: u16 = 8;
const PAGE_STEP: u16 = 10;

impl App {
    /// Open the selected file of the selected commit into the diff level.
    pub(super) fn open_diff(&mut self) {
        let (Some(hash), Some(path)) = (self.commit_hash(), self.file_path()) else {
            return;
        };
        let raw = load_diff_raw(&self.repo, hash, path);
        self.prepared = prepare_diff(&raw);
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.diff = render_prepared(&self.prepared, self.width, 0);
        self.level = Level::Diff;
    }

    /// Re-lay-out the prepared diff at the current width and pan.
    pub(super) fn relayout_diff(&mut self) {
        self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
    }

    pub(super) fn scroll_diff(&mut self, delta: i32) {
        self.diff_scroll = if delta >= 0 {
            self.diff_scroll.saturating_add(delta as u16)
        } else {
            self.diff_scroll.saturating_sub(delta.unsigned_abs() as u16)
        };
    }

    pub(super) fn pan_diff(&mut self, right: bool) {
        self.diff_hscroll = if right {
            clamp_hscroll(
                self.diff_hscroll.saturating_add(PAN_STEP),
                self.prepared.max_line(),
                self.prepared.cell_width(self.width),
            )
        } else {
            self.diff_hscroll.saturating_sub(PAN_STEP)
        };
        self.relayout_diff();
    }

    /// Steps for the keys that move by more than a line.
    pub(super) fn half_page(terminal: &DefaultTerminal) -> i32 {
        half_page(terminal) as i32
    }

    pub(super) const STEP: i32 = SCROLL_STEP as i32;
    pub(super) const PAGE: i32 = PAGE_STEP as i32;

    /// Keep the last line from scrolling up past the top of the viewport.
    pub(super) fn clamp_diff(&mut self, terminal: &DefaultTerminal) {
        self.diff_scroll = clamp_scroll(
            self.diff_scroll,
            self.diff.lines.len(),
            pane_height(terminal),
        );
    }

    /// Hand the file to the user's `git difftool`, then take the terminal back.
    pub(super) fn difftool(&mut self, terminal: &mut DefaultTerminal) {
        let (Some(hash), Some(path)) = (self.commit_hash(), self.file_path()) else {
            return;
        };
        let (hash, path) = (hash.to_string(), path.to_string());
        let m = difftool_commit(terminal, self.enhanced, &self.repo, &hash, &path);
        self.width = crate::tui::pane_width(terminal);
        if !m.is_empty() {
            self.msg = Some(m);
        }
    }
}
