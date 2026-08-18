//! Key predicates and the hint labels that describe them.
//!
//! Every view reads its keys through these, so `j`/`↓` mean the same thing
//! everywhere and a pane title never hand-writes a key name.

use ratatui::crossterm::event::KeyCode;

/// Key-hint labels shown in pane titles, defined once so every pane reads the
/// same. `Y_MOVE`/`X_MOVE` are plain vertical/horizontal navigation (arrows and
/// hjkl both work everywhere); the `CTRL_` variants are the modifier forms used
/// for the bottom pane — Ctrl-arrows are the terminal-safe way to send them,
/// since some terminals can't send a distinct Ctrl-letter.
pub const Y_MOVE: &str = "↑↓/jk";
pub const CTRL_Y_MOVE: &str = "ctrl-↑↓/jk";
pub const X_MOVE: &str = "←→/hl";
pub const CTRL_X_MOVE: &str = "ctrl-←→/hl";

// ── shared key predicates (arrow keys mirror j/k everywhere) ────────────────

/// Down = j or ↓.
pub fn is_down(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('j') | KeyCode::Down)
}
/// Up = k or ↑.
pub fn is_up(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('k') | KeyCode::Up)
}
/// Open/drill-in = Enter only (so l/→ are free for horizontal diff scroll).
pub fn is_open(c: KeyCode) -> bool {
    matches!(c, KeyCode::Enter)
}
/// Back/step-out = Esc (Ctrl-[ sends Esc too), so h/← are free to scroll.
pub fn is_back(c: KeyCode) -> bool {
    matches!(c, KeyCode::Esc)
}
/// Pan left = h or ←.
pub fn is_left(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('h') | KeyCode::Left)
}
/// Pan right = l or →.
pub fn is_right(c: KeyCode) -> bool {
    matches!(c, KeyCode::Char('l') | KeyCode::Right)
}

/// Fold Ctrl+[ back into Esc. Terminals send Ctrl+[ as the raw ESC byte, but the
/// kitty protocol we push (DISAMBIGUATE_ESCAPE_CODES) turns it into a distinct
/// Ctrl+[ event — so map it back, since Ctrl+[ is Esc in vim muscle memory. Call
/// it once per key event before matching.
pub fn norm_esc(code: KeyCode, ctrl: bool) -> KeyCode {
    if ctrl && matches!(code, KeyCode::Char('[')) {
        KeyCode::Esc
    } else {
        code
    }
}
