//! Rendering. Two panes on every level: the current list on top, the level
//! below it underneath, and the file diff in place of that list at the bottom
//! of the drill.

use super::{App, Level, Pane, Sel, branches, commits, repos};
use crate::git::RepoStatus;
use crate::tui::input::{CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE, char_to_byte};
use crate::tui::widgets::{
    commit_item, diff_hscrollbar, diff_scrollbar, file_item, list_scrollbar, pane_block,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};

/// Width the branch name column is padded to, and the cap on the repo one,
/// which grows to fit the paths actually on screen.
const NAME_W: usize = 28;
const REPO_NAME_MAX: usize = 40;
/// Cap on the origin column, which is the first thing to give when the terminal
/// is narrow: it identifies a repo, it is not what you came to read.
const ORIGIN_MAX: usize = 38;

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());
    let (top, bottom) = (areas[0], areas[1]);

    let move_hint = format!("{Y_MOVE} · {X_MOVE} scope · / filter · r refresh");
    let pick_hint = format!("{CTRL_Y_MOVE} select · enter open · ? filter");
    // Which of the two panes a typed filter is going into. Only one at a time,
    // and it is the pane's own title that shows it, so `/` and `?` can never be
    // confused for each other.
    let (edit_top, edit_bot) = (
        app.editing == Some(Pane::Top),
        app.editing == Some(Pane::Bottom),
    );

    match app.level {
        // Repos on top, the selected repo's branches below.
        Level::Repos => {
            let scope = repos::SCOPES[app.rsel.scope].label();
            let items = repo_items(app);
            let top_title = title(
                "repos",
                Some(scope),
                &app.rsel,
                false,
                Pane::Top,
                edit_top,
                &move_hint,
            );
            list(frame, top, items, top_title, true, &mut app.rsel);

            let items = branch_items(app);
            let bot_title = title(
                "branches",
                None,
                &app.bsel,
                app.bfeed.slow(),
                Pane::Bottom,
                edit_bot,
                &pick_hint,
            );
            list(frame, bottom, items, bot_title, true, &mut app.bsel);
        }

        // Branches on top, the selected branch's commits below.
        Level::Branches => {
            let scope = branches::SCOPES[app.bsel.scope].label();
            let items = branch_items(app);
            let hint = format!("{move_hint} · ↑=unpushed");
            let top_title = title(
                "branches",
                Some(scope),
                &app.bsel,
                app.bfeed.slow(),
                Pane::Top,
                edit_top,
                &hint,
            );
            list(frame, top, items, top_title, true, &mut app.bsel);

            let items = commit_items(app);
            let bot_title = title(
                "commits",
                None,
                &app.csel,
                app.cfeed.slow(),
                Pane::Bottom,
                edit_bot,
                &pick_hint,
            );
            list(frame, bottom, items, bot_title, true, &mut app.csel);
        }

        // Commits on top, the selected commit's files below.
        Level::Commits => {
            let items = commit_items(app);
            let hint = format!("{move_hint} · ↑=unpushed");
            let top_title = title(
                &commits_label(app),
                None,
                &app.csel,
                app.cfeed.slow(),
                Pane::Top,
                edit_top,
                &hint,
            );
            list(frame, top, items, top_title, true, &mut app.csel);

            let items = file_items(app);
            let hint = format!("{CTRL_Y_MOVE} select · enter open · ? filter · esc back");
            let bot_title = title(
                "files",
                None,
                &app.fsel,
                app.ffeed.slow(),
                Pane::Bottom,
                edit_bot,
                &hint,
            );
            list(frame, bottom, items, bot_title, true, &mut app.fsel);
        }

        // The commit list stays up as context; the diff takes the bottom pane.
        Level::Diff => {
            let items = commit_items(app);
            let top_title = title(
                &commits_label(app),
                None,
                &app.csel,
                app.cfeed.slow(),
                Pane::Top,
                false,
                "",
            );
            list(frame, top, items, top_title, false, &mut app.csel);
            diff(frame, bottom, app);
        }
    }

    if let Some(modal) = &mut app.modal {
        modal.draw(frame);
    }
}

/// Render one list pane with its scrollbar.
fn list(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem<'static>>,
    title: String,
    active: bool,
    sel: &mut Sel,
) {
    let widget = List::new(items)
        .block(pane_block(title, active))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(widget, area, &mut sel.state);
    list_scrollbar(frame, area, sel.len(), sel.state.offset());
}

/// Render the diff pane, or the message that replaced it.
fn diff(frame: &mut Frame, area: Rect, app: &App) {
    let path = app.file_path().unwrap_or("");
    let title = match &app.msg {
        Some(m) => format!(" {path}   ⚠ {m} "),
        None => format!(
            " {path}   enter difftool · {CTRL_Y_MOVE}·ctrl-d/u scroll · {CTRL_X_MOVE} pan · esc back "
        ),
    };
    let view = Paragraph::new(app.diff.clone())
        .block(pane_block(title, true))
        .scroll((app.diff_scroll, 0));
    frame.render_widget(view, area);
    diff_scrollbar(frame, area, app.diff.lines.len(), app.diff_scroll);
    let cell = app.prepared.cell_width(area.width.saturating_sub(2));
    diff_hscrollbar(frame, area, app.prepared.max_line(), cell, app.diff_hscroll);
}

/// `" <name> · <scope>  <i>/<n>   <hint> "`, the title every pane wears. A pane
/// still being streamed into says so with a trailing `…`, so a list that is
/// merely short is never mistaken for one that has finished arriving - but only
/// once the load has run long enough to be noticed, or every keypress on a fast
/// repo would blink it on and off.
fn title(
    name: &str,
    scope: Option<&str>,
    sel: &Sel,
    loading: bool,
    pane: Pane,
    editing: bool,
    hint: &str,
) -> String {
    let name = match scope {
        Some(s) => format!("{name} · {s}"),
        None => name.to_string(),
    };
    // A filter is shown on the pane it was typed into, which is the only thing
    // that says whether `/` or `?` was the key that opened it. It takes the key
    // hints' place, being the thing now deciding what the list holds.
    let text = &sel.query.text;
    let key = pane.sigil();
    let filter = if editing {
        let at = char_to_byte(text, sel.query.caret);
        Some(format!("{key}{}▏{}", &text[..at], &text[at..]))
    } else if !text.is_empty() {
        Some(format!("{key}{text}"))
    } else {
        None
    };
    if sel.is_empty() {
        let body = if loading { "…" } else { "(none)" };
        return match &filter {
            Some(f) => format!(" {name}  {body}   {f} "),
            None => format!(" {name}  {body} "),
        };
    }
    let dots = if loading { "…" } else { "" };
    let count = format!("{}/{}{dots}", sel.cur + 1, sel.len());
    let tail = filter.unwrap_or_else(|| hint.to_string());
    if tail.is_empty() {
        format!(" {name}  {count} ")
    } else {
        format!(" {name}  {count}   {tail} ")
    }
}

/// The commits pane names its scope, and the paths it was filtered to.
fn commits_label(app: &App) -> String {
    let scope = commits::SCOPES[app.csel.scope].label();
    if app.paths.is_empty() {
        format!("commits · {scope}")
    } else {
        format!("commits · {} {scope}", app.paths.join(" "))
    }
}

// ── row renderers ───────────────────────────────────────────────────────────

fn repo_items(app: &App) -> Vec<ListItem<'static>> {
    let name_w = app
        .rsel
        .visible
        .iter()
        .map(|&i| app.repos[i].name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(1, REPO_NAME_MAX);
    let branch_w = app
        .rsel
        .visible
        .iter()
        .map(|&i| app.repos[i].branch.chars().count())
        .max()
        .unwrap_or(0)
        .min(24);
    // Pad the state column so the origins line up in one readable column.
    let state_w = app
        .rsel
        .visible
        .iter()
        .map(|&i| state_width(&app.repos[i]))
        .max()
        .unwrap_or(0);
    app.rsel
        .visible
        .iter()
        .map(|&i| repo_item(&app.repos[i], name_w, branch_w, state_w))
        .collect()
}

/// Printed width of a repo's state flags, for padding the column.
fn state_width(r: &RepoStatus) -> usize {
    state_spans(r)
        .iter()
        .map(|s| s.content.chars().count())
        .sum()
}

/// `✚2 ↑1 ↓3` when there is something to report, else the clean marker.
fn state_spans(r: &RepoStatus) -> Vec<Span<'static>> {
    if !r.needs_attention() {
        return if r.has_upstream {
            vec![Span::styled("✓ clean", Style::default().fg(Color::Green))]
        } else {
            vec![Span::styled(
                "✓ clean (no upstream)",
                Style::default().add_modifier(Modifier::DIM),
            )]
        };
    }
    let mut spans = Vec::new();
    if r.dirty > 0 {
        spans.push(Span::styled(
            format!("✚{}", r.dirty),
            Style::default().fg(Color::Yellow),
        ));
    }
    if r.ahead > 0 {
        spans.push(Span::styled(
            format!("↑{}", r.ahead),
            Style::default().fg(Color::Green),
        ));
    }
    if r.behind > 0 {
        spans.push(Span::styled(
            format!("↓{}", r.behind),
            Style::default().fg(Color::Red),
        ));
    }
    // One space between flags, kept inside the spans so the width math is the
    // same arithmetic the renderer does.
    let last = spans.len().saturating_sub(1);
    for (i, span) in spans.iter_mut().enumerate() {
        if i != last {
            *span = Span::styled(format!("{} ", span.content), span.style);
        }
    }
    spans
}

/// Name, current branch, the same state flags `slu repos` prints, and where the
/// repo came from.
fn repo_item(r: &RepoStatus, name_w: usize, branch_w: usize, state_w: usize) -> ListItem<'static> {
    let mut spans = vec![
        Span::styled(
            format!("  {:<name_w$}", truncate(&r.name, name_w)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {:<branch_w$}", truncate(&r.branch, branch_w)),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("  "),
    ];

    let state = state_spans(r);
    let pad = state_w.saturating_sub(state.iter().map(|s| s.content.chars().count()).sum());
    spans.extend(state);
    spans.push(Span::raw(" ".repeat(pad)));

    if !r.origin.is_empty() {
        spans.push(Span::styled(
            format!("  {}", truncate(&r.origin, ORIGIN_MAX)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    ListItem::new(Line::from(spans))
}

fn branch_items(app: &App) -> Vec<ListItem<'static>> {
    if app.bsel.is_empty() && app.bfeed.slow() {
        return loading_body();
    }
    app.bsel
        .visible
        .iter()
        .map(|&i| branch_item(&app.branches[i]))
        .collect()
}

fn branch_item(b: &branches::Branch) -> ListItem<'static> {
    let name_style = if b.is_head {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else if b.remote {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let (mark, mark_style) = branch_mark(b);
    ListItem::new(Line::from(vec![
        Span::raw(if b.is_head { "● " } else { "  " }),
        Span::styled(format!("{mark:<2}"), mark_style),
        Span::styled(
            format!("{:<NAME_W$}", truncate(&b.name, NAME_W)),
            name_style,
        ),
        Span::styled(format!("  {:<10}", b.status()), mark_style),
        Span::styled(
            format!("  {:<14}", b.rel),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(format!("  {}", b.author), Style::default().fg(Color::Blue)),
    ]))
}

/// Quick-scan glyph: `↑` (unpushed/ahead, yellow), `⚑` (upstream gone, red),
/// nothing for a remote branch or an in-sync local one.
fn branch_mark(b: &branches::Branch) -> (&'static str, Style) {
    if b.remote {
        return ("", Style::default().fg(Color::DarkGray));
    }
    if b.track.contains("gone") {
        return ("⚑", Style::default().fg(Color::Red));
    }
    if b.unpushed() {
        return (
            "↑",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    ("", Style::default().fg(Color::DarkGray))
}

/// The body of a pane with nothing in it yet. It is a whole row rather than a
/// mark on the border because a blank box reads as "there is nothing here",
/// which is the one thing it does not mean.
fn loading_body() -> Vec<ListItem<'static>> {
    vec![ListItem::new(Line::from(Span::styled(
        "  loading…",
        Style::default().add_modifier(Modifier::DIM),
    )))]
}

fn commit_items(app: &App) -> Vec<ListItem<'static>> {
    if app.csel.is_empty() && app.cfeed.slow() {
        return loading_body();
    }
    app.csel
        .visible
        .iter()
        .map(|&i| {
            let c = &app.commits[i];
            commit_item(c, app.unpushed.contains(&c.hash))
        })
        .collect()
}

fn file_items(app: &App) -> Vec<ListItem<'static>> {
    if app.fsel.is_empty() && app.ffeed.slow() {
        return loading_body();
    }
    app.fsel
        .visible
        .iter()
        .map(|&i| file_item(&app.files[i]))
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
