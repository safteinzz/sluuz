//! Rendering. Two panes on every level: the current list on top, the level
//! below it underneath, and the file diff in place of that list at the bottom
//! of the drill.

use super::{branches, commits, repos, App, Level, Sel};
use crate::git::RepoStatus;
use crate::tui::input::{CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE};
use crate::tui::widgets::{
    commit_item, diff_hscrollbar, diff_scrollbar, file_item, list_scrollbar, pane_block,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

/// Width the repo and branch name columns are padded to.
const NAME_W: usize = 28;

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());
    let (top, bottom) = (areas[0], areas[1]);

    let move_hint = format!("{Y_MOVE} · {X_MOVE} scope");
    let pick_hint = format!("{CTRL_Y_MOVE} select · enter open");

    match app.level {
        // Repos on top, the selected repo's branches below.
        Level::Repos => {
            let scope = repos::SCOPES[app.rsel.scope].label();
            let items = repo_items(app);
            let top_title = title("repos", Some(scope), &app.rsel, &move_hint);
            list(frame, top, items, top_title, true, &mut app.rsel);

            let items = branch_items(app);
            let bot_title = title("branches", None, &app.bsel, &pick_hint);
            list(frame, bottom, items, bot_title, true, &mut app.bsel);
        }

        // Branches on top, the selected branch's commits below.
        Level::Branches => {
            let scope = branches::SCOPES[app.bsel.scope].label();
            let items = branch_items(app);
            let hint = format!("{move_hint} · ↑=unpushed");
            let top_title = title("branches", Some(scope), &app.bsel, &hint);
            list(frame, top, items, top_title, true, &mut app.bsel);

            let items = commit_items(app);
            let bot_title = title("commits", None, &app.csel, &pick_hint);
            list(frame, bottom, items, bot_title, true, &mut app.csel);
        }

        // Commits on top, the selected commit's files below.
        Level::Commits => {
            let items = commit_items(app);
            let hint = format!("{move_hint} · ↑=unpushed");
            let top_title = title(&commits_label(app), None, &app.csel, &hint);
            list(frame, top, items, top_title, true, &mut app.csel);

            let items = file_items(app);
            let hint = format!("{CTRL_Y_MOVE} select · enter open · esc back");
            let bot_title = title("files", None, &app.fsel, &hint);
            list(frame, bottom, items, bot_title, true, &mut app.fsel);
        }

        // The commit list stays up as context; the diff takes the bottom pane.
        Level::Diff => {
            let items = commit_items(app);
            let top_title = title(&commits_label(app), None, &app.csel, "");
            list(frame, top, items, top_title, false, &mut app.csel);
            diff(frame, bottom, app);
        }
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
            " {path}   enter difftool · {CTRL_Y_MOVE}·ctrl-d/u scroll · {CTRL_X_MOVE} pan · esc back · q quit "
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

/// `" <name> · <scope>  <i>/<n>   <hint> "`, the title every pane wears.
fn title(name: &str, scope: Option<&str>, sel: &Sel, hint: &str) -> String {
    let name = match scope {
        Some(s) => format!("{name} · {s}"),
        None => name.to_string(),
    };
    if sel.is_empty() {
        return format!(" {name}  (none) ");
    }
    let count = format!("{}/{}", sel.cur + 1, sel.len());
    if hint.is_empty() {
        format!(" {name}  {count} ")
    } else {
        format!(" {name}  {count}   {hint} ")
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
    let branch_w = app
        .rsel
        .visible
        .iter()
        .map(|&i| app.repos[i].branch.chars().count())
        .max()
        .unwrap_or(0)
        .min(24);
    app.rsel
        .visible
        .iter()
        .map(|&i| repo_item(&app.repos[i], branch_w))
        .collect()
}

/// Name, current branch, then the same state flags `slu repos` prints.
fn repo_item(r: &RepoStatus, branch_w: usize) -> ListItem<'static> {
    let mut spans = vec![
        Span::styled(
            format!("  {:<NAME_W$}", truncate(&r.name, NAME_W)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {:<branch_w$}", truncate(&r.branch, branch_w)),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("  "),
    ];
    if r.needs_attention() {
        if r.dirty > 0 {
            spans.push(Span::styled(
                format!("✚{} ", r.dirty),
                Style::default().fg(Color::Yellow),
            ));
        }
        if r.ahead > 0 {
            spans.push(Span::styled(
                format!("↑{} ", r.ahead),
                Style::default().fg(Color::Green),
            ));
        }
        if r.behind > 0 {
            spans.push(Span::styled(
                format!("↓{} ", r.behind),
                Style::default().fg(Color::Red),
            ));
        }
    } else if r.has_upstream {
        spans.push(Span::styled("✓ clean", Style::default().fg(Color::Green)));
    } else {
        spans.push(Span::styled(
            "✓ clean (no upstream)",
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    ListItem::new(Line::from(spans))
}

fn branch_items(app: &App) -> Vec<ListItem<'static>> {
    app.bsel
        .visible
        .iter()
        .map(|&i| branch_item(&app.branches[i]))
        .collect()
}

fn branch_item(b: &branches::Branch) -> ListItem<'static> {
    let name_style = if b.is_head {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else if b.remote {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let (mark, mark_style) = branch_mark(b);
    ListItem::new(Line::from(vec![
        Span::raw(if b.is_head { "● " } else { "  " }),
        Span::styled(format!("{mark:<2}"), mark_style),
        Span::styled(format!("{:<NAME_W$}", truncate(&b.name, NAME_W)), name_style),
        Span::styled(format!("  {:<10}", b.status()), mark_style),
        Span::styled(format!("  {:<14}", b.rel), Style::default().fg(Color::Magenta)),
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
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        );
    }
    ("", Style::default().fg(Color::DarkGray))
}

fn commit_items(app: &App) -> Vec<ListItem<'static>> {
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
