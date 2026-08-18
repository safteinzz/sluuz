//! Key dispatch, one arm per level.
//!
//! The shape is the same everywhere: plain keys drive the top pane, Ctrl drives
//! the pane below it, `h`/`l` slide that level's scope, Enter drills in and Esc
//! steps back out.

use super::{branches, commits, repos, App, Level};
use crate::tui::input::{is_back, is_down, is_left, is_open, is_right, is_up, norm_esc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::DefaultTerminal;

/// Handle one key press. Returns true when the app should quit.
pub(super) fn on_key(app: &mut App, key: KeyEvent, terminal: &mut DefaultTerminal) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let code = norm_esc(key.code, ctrl);
    app.msg = None; // any keypress clears a stale status message

    if code == KeyCode::Char('q') || (ctrl && code == KeyCode::Char('c')) {
        return true;
    }

    match app.level {
        Level::Repos => repos_key(app, code, ctrl),
        Level::Branches => branches_key(app, code, ctrl),
        Level::Commits => commits_key(app, code, ctrl),
        Level::Diff => diff_key(app, code, ctrl, terminal),
    }
}

/// Repos on top, the selected repo's branches below.
fn repos_key(app: &mut App, code: KeyCode, ctrl: bool) -> bool {
    if ctrl && (is_down(code) || is_up(code)) {
        app.bsel.step(is_down(code));
    } else if is_down(code) || is_up(code) {
        if app.rsel.step(is_down(code)) {
            app.enter_repo();
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.rsel.slide(is_right(code), repos::SCOPES.len()) {
            app.rescope_repos();
            app.enter_repo();
        }
    } else if is_open(code) && !app.bsel.is_empty() {
        app.enter_branch();
        app.level = Level::Branches;
    } else if is_back(code) {
        return app.back();
    }
    false
}

/// Branches on top, the selected branch's commits below.
fn branches_key(app: &mut App, code: KeyCode, ctrl: bool) -> bool {
    if ctrl && (is_down(code) || is_up(code)) {
        app.csel.step(is_down(code));
    } else if is_down(code) || is_up(code) {
        if app.bsel.step(is_down(code)) {
            app.enter_branch();
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.bsel.slide(is_right(code), branches::SCOPES.len()) {
            app.rescope_branches();
            app.enter_branch();
        }
    } else if is_open(code) && !app.csel.is_empty() {
        app.enter_commit();
        app.level = Level::Commits;
    } else if is_back(code) {
        return app.back();
    }
    false
}

/// Commits on top, the selected commit's files below.
fn commits_key(app: &mut App, code: KeyCode, ctrl: bool) -> bool {
    if ctrl && (is_down(code) || is_up(code)) {
        app.fsel.step(is_down(code));
    } else if is_down(code) || is_up(code) {
        if app.csel.step(is_down(code)) {
            app.enter_commit();
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.csel.slide(is_right(code), commits::SCOPES.len()) {
            app.rescope_commits();
            app.enter_commit();
        }
    } else if is_open(code) && !app.fsel.is_empty() {
        app.open_diff();
    } else if is_back(code) {
        return app.back();
    }
    false
}

/// The file's diff, with the commit list kept above it for context.
fn diff_key(app: &mut App, code: KeyCode, ctrl: bool, terminal: &mut DefaultTerminal) -> bool {
    if ctrl && is_down(code) {
        app.scroll_diff(App::STEP);
    } else if ctrl && is_up(code) {
        app.scroll_diff(-App::STEP);
    } else if ctrl && code == KeyCode::Char('d') {
        app.scroll_diff(App::half_page(terminal));
    } else if ctrl && code == KeyCode::Char('u') {
        app.scroll_diff(-App::half_page(terminal));
    } else if code == KeyCode::PageDown {
        app.scroll_diff(App::PAGE);
    } else if code == KeyCode::PageUp {
        app.scroll_diff(-App::PAGE);
    } else if ctrl && (is_left(code) || is_right(code)) {
        app.pan_diff(is_right(code));
    } else if is_open(code) {
        app.difftool(terminal);
    } else if is_back(code) {
        return app.back();
    }
    app.clamp_diff(terminal);
    false
}
