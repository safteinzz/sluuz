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

/// The diff `r` was pressed on, waited for while the commits stream back in.
pub(super) struct Reopen {
    /// The commit it was of, and how a note names it.
    hash: String,
    label: String,
    path: String,
    /// Where its row stood, which is the row that replaces it if it is gone.
    row: usize,
    /// It never came back, so the row now in its place is what opens.
    gone: bool,
}

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

    /// What `r` on this diff has to find again once the commits are read.
    pub(super) fn reopen_point(&self) -> Option<Reopen> {
        let i = self.csel.idx()?;
        Some(Reopen {
            hash: self.commits[i].hash.clone(),
            label: self.row_label(i),
            path: self.file_path()?.to_string(),
            row: self.csel.cur,
            gone: false,
        })
    }

    /// A row as a note names it: its short sha, or under `istash` its message,
    /// since `stash@{0}` names whichever stash is newest now.
    fn row_label(&self, i: usize) -> String {
        let c = &self.commits[i];
        if self.stashes {
            format!("`{}`", c.subject)
        } else {
            c.short.clone()
        }
    }

    /// `r` on a diff, called every frame until it is done: once the commit it
    /// was of has streamed in and its files are loaded, open the same file,
    /// scrolled where it was. A commit that never turns up (amended, rebased,
    /// a stash dropped elsewhere) is replaced by the row now in its place, and
    /// the note says so, rather than showing another commit under the old one's
    /// name.
    pub(super) fn settle_reopen(&mut self) {
        let Some(r) = &self.reopen else {
            return;
        };
        if self.csel.restore.is_some() {
            if self.cfeed.loading {
                return;
            }
            self.csel.restore = None;
            if self.csel.is_empty() {
                let label = r.label.clone();
                self.reopen = None;
                self.level = Level::Commits;
                self.set_failed(format!("{label} is gone"));
                return;
            }
            let at = r.row.min(self.csel.len() - 1);
            self.csel.restored_at(at);
            if let Some(r) = &mut self.reopen {
                r.gone = true;
            }
            self.sync_files();
            return;
        }
        let (Some(i), Some(hash)) = (self.csel.idx(), self.commit_hash()) else {
            return;
        };
        // The cursor sits on the first row until the restore finds its commit,
        // and that row's files are not the ones to open.
        let waiting = !r.gone && hash != r.hash;
        if waiting || self.files_for != hash || self.ffeed.loading {
            return;
        }
        let Some(r) = self.reopen.take() else {
            return;
        };
        let now = self.row_label(i);
        let found = self
            .fsel
            .visible
            .iter()
            .position(|&f| self.files[f].path == r.path);
        let Some(at) = found else {
            self.level = Level::Commits;
            self.set_failed(if r.gone {
                format!("{} is gone, and `{}` is not in {now}", r.label, r.path)
            } else {
                format!("`{}` is not in {now} any more", r.path)
            });
            return;
        };
        self.fsel.restored_at(at);
        let (scroll, pan) = (self.diff_scroll, self.diff_hscroll);
        self.open_diff();
        if r.gone {
            self.set_failed(format!("{} is gone, so this is {now}", r.label));
        } else {
            self.diff_scroll = scroll;
            self.diff_hscroll = pan;
        }
    }

    /// Keep the pan inside the widest line, which a reloaded diff may have made
    /// narrower.
    pub(super) fn clamp_pan(&mut self) {
        self.diff_hscroll = clamp_hscroll(
            self.diff_hscroll,
            self.prepared.max_line(),
            self.prepared.cell_width(self.width),
        );
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
