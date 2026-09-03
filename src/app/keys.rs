//! Key dispatch, one arm per level.
//!
//! The shape is the same everywhere: plain keys drive the top pane, Ctrl drives
//! the pane below it, `h`/`l` slide that level's scope, Enter drills in and Esc
//! steps back out.

use super::{App, Level, Pane, Sel, branches, commits, repos};
use crate::tui::input::{is_back, is_down, is_left, is_open, is_right, is_up, norm_esc};
use crate::tui::{half_page, upper_pane_height};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handle one key press. Returns true when the app should quit.
pub(super) fn on_key(app: &mut App, key: KeyEvent, terminal: &mut DefaultTerminal) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let code = norm_esc(key.code, ctrl);

    // Ctrl-C quits from anywhere, even out from under a modal.
    if ctrl && code == KeyCode::Char('c') {
        return true;
    }
    // A modal owns every key until it is dismissed.
    if let Some(modal) = &mut app.modal {
        if modal.on_key(code) {
            app.modal = None;
        }
        return false;
    }
    // So does an open query bar - `q` types a letter there, it does not quit.
    if let Some(pane) = app.editing {
        query_key(app, pane, code);
        return false;
    }

    app.msg = None; // any keypress clears a stale status message
    if code == KeyCode::Char('q') {
        return true;
    }

    // `/` narrows the list plain keys drive, `?` the pane below it - the same
    // split as every other key at every level.
    if code == KeyCode::Char('/') {
        app.open_query(Pane::Top);
        return false;
    }
    if code == KeyCode::Char('?') {
        app.open_query(Pane::Bottom);
        return false;
    }

    // `r` reads this level again: another terminal may have committed since it
    // was loaded, and quitting to see that is not an answer.
    if code == KeyCode::Char('r') {
        app.refresh();
        return false;
    }

    // Only another move of the top pane can safely leave a load pending:
    // anything else acts on the pane that load fills, so it has to finish first.
    let moving = !ctrl
        && (is_up(code)
            || is_down(code)
            || is_left(code)
            || is_right(code)
            || matches!(code, KeyCode::PageUp | KeyCode::PageDown));
    if app.pending && !moving {
        app.settle();
    }

    match app.level {
        Level::Repos => repos_key(app, code, ctrl, terminal),
        Level::Branches => branches_key(app, code, ctrl, terminal),
        Level::Commits => commits_key(app, code, ctrl, terminal),
        Level::Diff => diff_key(app, code, ctrl, terminal),
    }
}

/// The query bar has focus: a plain text field over one pane's list, narrowing
/// it as it is typed. Enter keeps what it found and Esc clears it, so a filter
/// is never left on a pane with no way to see it went there.
fn query_key(app: &mut App, pane: Pane, code: KeyCode) {
    if code == KeyCode::Enter {
        app.editing = None;
        return;
    }
    if code == KeyCode::Esc {
        if let Some(sel) = app.pane_sel(pane) {
            sel.query.clear();
        }
        app.editing = None;
        app.refilter(pane);
        return;
    }
    let Some(sel) = app.pane_sel(pane) else {
        app.editing = None;
        return;
    };
    let end = sel.query.text.chars().count();
    match code {
        KeyCode::Char(c) => sel.query.insert(c),
        KeyCode::Backspace => sel.query.backspace(),
        KeyCode::Delete => sel.query.delete(),
        KeyCode::Left => sel.query.caret = sel.query.caret.saturating_sub(1),
        KeyCode::Right if sel.query.caret < end => sel.query.caret += 1,
        KeyCode::Home => sel.query.caret = 0,
        KeyCode::End => sel.query.caret = end,
        _ => return,
    }
    app.refilter(pane);
}

/// The paging keys, the same at every list level: PageUp/PageDown move the list
/// plain keys drive, Ctrl-d/u half-page the pane below it, which is what Ctrl
/// already means everywhere else. Returns whether the top pane moved, or None
/// when the key was not one of these.
fn paging(
    code: KeyCode,
    ctrl: bool,
    terminal: &DefaultTerminal,
    top: &mut Sel,
    bottom: &mut Sel,
) -> Option<bool> {
    let half = half_page(terminal) as usize;
    let page = upper_pane_height(terminal) as usize;
    if ctrl && code == KeyCode::Char('d') {
        bottom.jump(true, half);
        Some(false)
    } else if ctrl && code == KeyCode::Char('u') {
        bottom.jump(false, half);
        Some(false)
    } else if code == KeyCode::PageDown {
        Some(top.jump(true, page))
    } else if code == KeyCode::PageUp {
        Some(top.jump(false, page))
    } else {
        None
    }
}

/// Repos on top, the selected repo's branches below.
fn repos_key(app: &mut App, code: KeyCode, ctrl: bool, terminal: &DefaultTerminal) -> bool {
    if let Some(moved) = paging(code, ctrl, terminal, &mut app.rsel, &mut app.bsel) {
        app.pending |= moved;
        return false;
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.bsel.step(is_down(code));
    } else if is_down(code) || is_up(code) {
        if app.rsel.step(is_down(code)) {
            app.pending = true;
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.rsel.slide(is_right(code), repos::SCOPES.len()) {
            app.rescope_repos();
            app.pending = true;
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
fn branches_key(app: &mut App, code: KeyCode, ctrl: bool, terminal: &DefaultTerminal) -> bool {
    if let Some(moved) = paging(code, ctrl, terminal, &mut app.bsel, &mut app.csel) {
        app.pending |= moved;
        return false;
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.csel.step(is_down(code));
    } else if is_down(code) || is_up(code) {
        if app.bsel.step(is_down(code)) {
            app.pending = true;
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.bsel.slide(is_right(code), branches::SCOPES.len()) {
            app.rescope_branches();
            app.pending = true;
        }
    } else if is_open(code) && !app.csel.is_empty() {
        app.level = Level::Commits;
        app.enter_commit();
    } else if is_back(code) {
        return app.back();
    }
    false
}

/// Commits on top, the selected commit's files below.
fn commits_key(app: &mut App, code: KeyCode, ctrl: bool, terminal: &DefaultTerminal) -> bool {
    if let Some(moved) = paging(code, ctrl, terminal, &mut app.csel, &mut app.fsel) {
        app.pending |= moved;
        return false;
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.fsel.step(is_down(code));
    } else if is_down(code) || is_up(code) {
        if app.csel.step(is_down(code)) {
            app.pending = true;
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.csel.slide(is_right(code), commits::SCOPES.len()) {
            app.rescope_commits();
            app.pending = true;
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
