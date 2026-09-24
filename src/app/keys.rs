//! Key dispatch, one arm per level.
//!
//! The shape is the same everywhere: plain keys drive the top pane, Ctrl drives
//! the pane below it, `h`/`l` slide that level's scope, Enter drills in and Esc
//! steps back out.

use super::{App, Level, Pane, branches, commits, repos, tags};
use crate::tui::input::{is_back, is_down, is_left, is_open, is_right, is_up, norm_esc};
use crate::tui::widgets::{Command, CommandLine, Modal, Typed};
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
    // So does a delete waiting on its answer. A typed gate goes ahead only once
    // the name is typed exactly, so no reflex can get through it.
    if let Some(confirm) = &mut app.confirm
        && confirm.target.typed()
    {
        match code {
            KeyCode::Enter if confirm.typed == confirm.target.name() => {
                if let Some(confirm) = app.confirm.take() {
                    app.delete(confirm.target);
                }
            }
            KeyCode::Esc => app.confirm = None,
            KeyCode::Backspace => {
                confirm.typed.pop();
            }
            KeyCode::Char(c) => confirm.typed.push(c),
            _ => {}
        }
        return false;
    }
    // A plain gate: Yes is on the left, and it opens on No, so a reflex Enter is
    // never the key that loses something.
    if let Some(confirm) = &mut app.confirm {
        if is_left(code) {
            confirm.yes = true;
        } else if is_right(code) {
            confirm.yes = false;
        } else if code == KeyCode::Char('y') || (is_open(code) && confirm.yes) {
            if let Some(confirm) = app.confirm.take() {
                app.delete(confirm.target);
            }
        } else if is_open(code) || is_back(code) || matches!(code, KeyCode::Char('q' | 'n')) {
            app.confirm = None;
        }
        return false;
    }
    // So does an open `:` line.
    if let Some(cmd) = &mut app.command {
        match cmd.on_key(code) {
            Typed::Open => {}
            Typed::Cancel => app.command = None,
            Typed::Run(run) => {
                app.command = None;
                match run {
                    Command::Help => {
                        app.modal = Some(Modal::reader(app.help_title(), &app.help_rows()))
                    }
                    Command::Quit => return true,
                    Command::Unknown(what) => {
                        app.set_failed(format!("unknown command `{what}` · :help lists the keys"))
                    }
                }
            }
        }
        return false;
    }
    // So does an open query bar - `q` types a letter there, it does not quit.
    if let Some(pane) = app.editing {
        query_key(app, pane, code);
        return false;
    }

    if code == KeyCode::Char('q') {
        return true;
    }
    if code == KeyCode::Char(':') {
        app.command = Some(CommandLine::default());
        return false;
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
    let moving = !ctrl && (is_up(code) || is_down(code) || is_left(code) || is_right(code));
    if app.pending && !moving {
        app.settle();
    }

    if !ctrl && code == KeyCode::Char('K') {
        app.inspect();
        return false;
    }

    let steps = app.accel.steps(code, ctrl);
    match app.level {
        Level::Repos => repos_key(app, code, ctrl, steps),
        Level::Branches => branches_key(app, code, ctrl, steps),
        Level::Tags => tags_key(app, code, ctrl, steps),
        Level::Commits => commits_key(app, code, ctrl, steps),
        Level::Diff => diff_key(app, code, ctrl, steps, terminal),
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
    if sel.query.on_key(code) {
        app.refilter(pane);
    }
}

/// Repos on top, the selected repo's branches below.
fn repos_key(app: &mut App, code: KeyCode, ctrl: bool, steps: usize) -> bool {
    if !ctrl && matches!(code, KeyCode::Char('s' | 'S')) {
        let pull = code == KeyCode::Char('S');
        app.set_status(if pull {
            "pulling every repo…"
        } else {
            "syncing every repo…"
        });
        app.sync_next = Some(pull);
        return false;
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.bsel.step(is_down(code), steps);
    } else if is_down(code) || is_up(code) {
        if app.rsel.step(is_down(code), steps) {
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
fn branches_key(app: &mut App, code: KeyCode, ctrl: bool, steps: usize) -> bool {
    if !ctrl && code == KeyCode::Char('d') {
        app.ask_delete_branch();
        return false;
    }
    if !ctrl && code == KeyCode::Char('u') {
        if app.undo.as_ref().is_some_and(|u| u.branch) {
            app.undo();
        }
        return false;
    }
    if !ctrl && matches!(code, KeyCode::Char('s' | 'S')) {
        let pull = code == KeyCode::Char('S');
        app.set_status(if pull { "pulling…" } else { "syncing…" });
        app.sync_next = Some(pull);
        return false;
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.csel.step(is_down(code), steps);
    } else if is_down(code) || is_up(code) {
        if app.bsel.step(is_down(code), steps) {
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

/// Tags on top, the selected tag's own commits below.
fn tags_key(app: &mut App, code: KeyCode, ctrl: bool, steps: usize) -> bool {
    if !ctrl && code == KeyCode::Char('d') {
        app.ask_delete_tag();
        return false;
    }
    if !ctrl && code == KeyCode::Char('u') {
        if app.undo.as_ref().is_some_and(|u| !u.branch) {
            app.undo();
        }
        return false;
    }
    if !ctrl && code == KeyCode::Char('t') {
        app.offer_tracking();
        return false;
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.csel.step(is_down(code), steps);
    } else if is_down(code) || is_up(code) {
        if app.tsel.step(is_down(code), steps) {
            app.pending = true;
        }
    } else if !ctrl && (is_left(code) || is_right(code)) {
        if app.tsel.slide(is_right(code), tags::SCOPES.len()) {
            app.rescope_tags();
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

/// Commits on top, the selected commit's files below. Under `istash` the
/// commits are stashes, which `a`, `p` and `d` act on, and have no push state
/// to slide between.
fn commits_key(app: &mut App, code: KeyCode, ctrl: bool, steps: usize) -> bool {
    if app.stashes && !ctrl {
        match code {
            KeyCode::Char('a') => app.apply_stash(false),
            KeyCode::Char('p') => app.apply_stash(true),
            KeyCode::Char('d') => app.ask_drop_stash(),
            _ => {}
        }
        if matches!(code, KeyCode::Char('a' | 'p' | 'd')) || is_left(code) || is_right(code) {
            return false;
        }
    }
    if ctrl && (is_down(code) || is_up(code)) {
        app.fsel.step(is_down(code), steps);
    } else if is_down(code) || is_up(code) {
        if app.csel.step(is_down(code), steps) {
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
fn diff_key(
    app: &mut App,
    code: KeyCode,
    ctrl: bool,
    steps: usize,
    terminal: &mut DefaultTerminal,
) -> bool {
    if ctrl && (is_down(code) || is_up(code)) {
        app.scroll_diff(is_down(code), steps);
    } else if ctrl && (is_left(code) || is_right(code)) {
        app.pan_diff(is_right(code), steps);
    } else if is_open(code) {
        app.difftool(terminal);
    } else if is_back(code) {
        return app.back();
    }
    app.clamp_diff();
    false
}
