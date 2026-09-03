//! Shared building blocks for the interactive TUIs: terminal setup, key
//! predicates, widgets, diff rendering, and the difftool handoff. Reading git is
//! not terminal work, so the queries the views run live in `git::load`.
//!
//! Nothing in here knows about any one view. `app/` builds the repos → branches
//! → commits → diff drill on top of it, and `iscan`/`istatus`/`itidy` use the
//! same pieces to draw their own screens.

pub mod difftool;
pub mod highlight;
pub mod input;
pub mod widgets;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::crossterm::{ExecutableCommand, execute};
use std::io::stdout;

// ── keyboard enhancement (kitty protocol) ───────────────────────────────────

/// Request DISAMBIGUATE_ESCAPE_CODES if the terminal supports the kitty
/// keyboard protocol. Returns true if pushed (caller must call pop).
/// This makes Ctrl+J distinct from Enter in terminals like VS Code.
pub fn push_keyboard_enhancement() -> bool {
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        true
    } else {
        false
    }
}

pub fn pop_keyboard_enhancement() {
    let _ = stdout().execute(PopKeyboardEnhancementFlags);
}

// ── terminal geometry ───────────────────────────────────────────────────────

pub fn pane_width(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| s.width.saturating_sub(2))
        .unwrap_or(120)
}

/// Inner height of the lower (~60%) pane - the diff viewport, in rows.
pub fn pane_height(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| ((s.height as u32 * 6 / 10).saturating_sub(2).max(1)) as u16)
        .unwrap_or(20)
}

/// Inner height of the upper (~40%) pane - the list a level's plain keys drive,
/// and what PageUp/PageDown move it by.
pub fn upper_pane_height(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| ((s.height as u32 * 4 / 10).saturating_sub(2).max(1)) as u16)
        .unwrap_or(14)
}

/// Half the height of the lower (~60%) pane, for vim Ctrl-d/Ctrl-u.
pub fn half_page(terminal: &DefaultTerminal) -> u16 {
    (pane_height(terminal) / 2).max(1)
}

/// Clamp a scroll offset so the last line can't scroll above the viewport -
/// stops you from scrolling off the bottom into empty space.
pub fn clamp_scroll(scroll: u16, total_lines: usize, viewport: u16) -> u16 {
    let max = (total_lines.min(u16::MAX as usize) as u16).saturating_sub(viewport);
    scroll.min(max)
}

/// Clamp horizontal scroll so you can't pan past the longest line. `cell_w` is
/// roughly one side's visible width.
pub fn clamp_hscroll(hscroll: u16, max_line: usize, cell_w: u16) -> u16 {
    let max = (max_line.min(u16::MAX as usize) as u16).saturating_sub(cell_w);
    hscroll.min(max)
}
