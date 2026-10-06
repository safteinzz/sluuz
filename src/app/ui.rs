//! Rendering. Two panes on every level: the current list on top, the level
//! below it underneath, and the file diff in place of that list at the bottom
//! of the drill.

use super::{App, Level, Pane, Sel, branches, commits, repos, tags};
use crate::git::RepoStatus;
use crate::git::load::{RemoteTags, Tag, TagState};
use crate::tui::input::{CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE};
use crate::tui::widgets::{
    DEL, FILTER_KEYS, FIND, HELP, HelpSection, Modal, QUIT, REFRESH, commit_item, confirm_popup,
    diff_hscrollbar, diff_scrollbar, file_item, key_footer, list_scrollbar, pane_block, scope_tabs,
    typed_popup,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, Paragraph};

/// Width the branch name column is padded to, and the cap on the repo one,
/// which grows to fit the paths actually on screen.
const NAME_W: usize = 28;
const REPO_NAME_MAX: usize = 40;
/// Cap on the origin column, which is the first thing to give when the terminal
/// is narrow: it identifies a repo, it is not what you came to read.
const ORIGIN_MAX: usize = 38;

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let [panes, footer_row] =
        Layout::vertical([Constraint::Min(2), Constraint::Length(1)]).areas(frame.area());
    let [top, bottom] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(panes);

    // Which of the two panes a typed filter is going into. Only one at a time,
    // and it is the pane's own title that shows it, so `/` and `ctrl-f` can
    // never be confused for each other.
    let (edit_top, edit_bot) = (
        app.editing == Some(Pane::Top),
        app.editing == Some(Pane::Bottom),
    );

    match app.level {
        // Repos on top, the selected repo's branches below.
        Level::Repos => {
            let labels = stops(&repos::SCOPES, repos::Scope::label);
            let scope = Some((labels.as_slice(), app.rsel.scope));
            let items = repo_items(app);
            let top_title = title("repos", scope, &app.rsel, false, Pane::Top, edit_top, true);
            list(frame, top, items, top_title, true, &mut app.rsel);

            let items = branch_items(app);
            let slow = app.bfeed.slow();
            let bot_title = title(
                "branches",
                None,
                &app.bsel,
                slow,
                Pane::Bottom,
                edit_bot,
                true,
            );
            list(frame, bottom, items, bot_title, true, &mut app.bsel);
        }

        // Branches on top, the selected branch's commits below.
        Level::Branches => {
            let labels = stops(&branches::SCOPES, branches::Scope::label);
            let scope = Some((labels.as_slice(), app.bsel.scope));
            let items = branch_items(app);
            let slow = app.bfeed.slow();
            let top_title = title(
                "branches",
                scope,
                &app.bsel,
                slow,
                Pane::Top,
                edit_top,
                true,
            );
            list(frame, top, items, top_title, true, &mut app.bsel);

            let items = commit_items(app, bottom.width);
            let slow = app.cfeed.slow();
            let bot_title = title(
                "commits",
                None,
                &app.csel,
                slow,
                Pane::Bottom,
                edit_bot,
                true,
            );
            list(frame, bottom, items, bot_title, true, &mut app.csel);
        }

        // Tags on top, the commits each one added below.
        Level::Tags => {
            let labels = stops(&tags::SCOPES, tags::Scope::label);
            let scope = Some((labels.as_slice(), app.tsel.scope));
            let items = tag_items(app);
            let slow = app.rfeed.slow();
            let (name, status) = tags_label(app);
            let mut top_title = title(&name, scope, &app.tsel, slow, Pane::Top, edit_top, true);
            // What the remote is up to changes while you look, so it goes after
            // everything else and never shifts the tabs along.
            if let Some(status) = status {
                let at = top_title.spans.len() - 1;
                let dim = Style::default().add_modifier(Modifier::DIM);
                top_title
                    .spans
                    .insert(at, Span::styled(format!("   {status}"), dim));
            }
            list(frame, top, items, top_title, true, &mut app.tsel);

            let items = commit_items(app, bottom.width);
            // A tag only the remote has names a commit this clone may not have,
            // and "(none)" would read as a tag with nothing in it.
            let unfetched = app.csel.is_empty()
                && !app.cfeed.loading
                && app
                    .tsel
                    .idx()
                    .is_some_and(|i| app.tags[i].state == TagState::Remote);
            let name = if unfetched {
                "commits · not in this clone until a fetch".to_string()
            } else {
                app.range_label()
            };
            let slow = app.cfeed.slow();
            let bot_title = title(&name, None, &app.csel, slow, Pane::Bottom, edit_bot, true);
            list(frame, bottom, items, bot_title, true, &mut app.csel);
        }

        // Commits on top, the selected commit's files below.
        Level::Commits => {
            let labels = stops(&commits::SCOPES, commits::Scope::label);
            let scope = (!app.stashes).then_some((labels.as_slice(), app.csel.scope));
            let items = commit_items(app, top.width);
            let slow = app.cfeed.slow();
            let top_title = title(
                &commits_label(app),
                scope,
                &app.csel,
                slow,
                Pane::Top,
                edit_top,
                true,
            );
            list(frame, top, items, top_title, true, &mut app.csel);

            let items = file_items(app);
            let slow = app.ffeed.slow();
            let bot_title = title("files", None, &app.fsel, slow, Pane::Bottom, edit_bot, true);
            list(frame, bottom, items, bot_title, true, &mut app.fsel);
        }

        // The commit list stays up as context; the diff takes the bottom pane.
        Level::Diff => {
            let labels = stops(&commits::SCOPES, commits::Scope::label);
            let scope = (!app.stashes).then_some((labels.as_slice(), app.csel.scope));
            let items = commit_items(app, top.width);
            let slow = app.cfeed.slow();
            let top_title = title(
                &commits_label(app),
                scope,
                &app.csel,
                slow,
                Pane::Top,
                false,
                false,
            );
            list(frame, top, items, top_title, false, &mut app.csel);
            app.diff_rows = bottom.height.saturating_sub(2).max(1);
            diff(frame, bottom, app);
        }
    }
    // Every letter but `?` goes into a filter being typed, so its footer offers
    // only keys that are not letters.
    let (keys, help) = match app.editing {
        Some(_) => (FILTER_KEYS.to_vec(), HELP),
        None => (actions(app), HELP),
    };
    frame.render_widget(
        key_footer(
            &keys,
            app.note.as_ref(),
            app.command.as_ref(),
            Some(help),
            footer_row.width,
        ),
        footer_row,
    );

    if let Some(c) = &app.confirm
        && c.target.typed()
    {
        typed_popup(
            frame,
            panes,
            &c.target.title(),
            c.target.note().as_deref(),
            &c.target.name(),
            &c.typed,
            c.target.verb(),
        );
    } else if let Some(c) = &app.confirm {
        confirm_popup(
            frame,
            panes,
            c.target.colour(),
            &c.target.title(),
            &c.target.name(),
            c.target.note().as_deref(),
            c.yes,
        );
    }
    if let Some(modal) = &mut app.modal {
        modal.draw(frame, panes);
    }
}

/// The footer's share of a level's keys: what it can *do*, in the house order
/// (`↵`, the level's own keys, `d del`, then `/ find · r refresh · q quit`), so
/// a narrow window loses the least used. Moving is left out, since motions are
/// tried rather than read, and `? help` lists everything, movement included.
fn actions(app: &App) -> Vec<&'static str> {
    let mut keys = vec![match app.level {
        Level::Commits => "↵ diff",
        Level::Diff => "↵ difftool",
        _ => "↵ open",
    }];
    let undo = |branch: bool| app.undo.as_ref().is_some_and(|u| u.branch == branch);
    let mut del = false;
    match app.level {
        Level::Repos => keys.extend(["s sync all", "S pull all"]),
        Level::Branches => {
            keys.extend(["s sync", "S pull"]);
            if undo(true) {
                keys.push("u undo");
            }
            del = true;
        }
        Level::Tags => {
            if app.can_track() {
                keys.push("t track");
            }
            if undo(false) {
                keys.push("u undo");
            }
            del = true;
        }
        Level::Commits if app.stashes => {
            keys.extend(["a apply", "p pop"]);
            del = true;
        }
        _ => {}
    }
    keys.push("K inspect");
    if del {
        keys.push(DEL);
    }
    // the diff has no list of its own for `/` to narrow
    if app.level != Level::Diff {
        keys.push(FIND);
    }
    keys.extend([REFRESH, QUIT]);
    keys
}

/// The motions every level shares. The stash has no tabs to switch between,
/// so it gets the same group without `h/l`.
const MOVING: HelpSection = (
    "moving",
    &[
        (Y_MOVE, "move a row"),
        (X_MOVE, "the previous, next tab"),
        ("", "a held key speeds up"),
    ],
);
const MOVING_STASH: HelpSection = (
    "moving",
    &[(Y_MOVE, "move a row"), ("", "a held key speeds up")],
);
const BELOW: HelpSection = (
    "the pane below",
    &[(CTRL_Y_MOVE, "move in it"), ("ctrl-f", "find in it")],
);
const EVERYWHERE: HelpSection = (
    "every screen",
    &[
        ("↵", "open"),
        ("esc", "back"),
        ("/", "find"),
        ("K", "inspect the row"),
        ("r", "refresh"),
        (":", "a command (:q quits)"),
        ("?", "this help"),
        ("q", "quit"),
        ("ctrl-c", "quit, or esc in a box or a filter"),
    ],
);
const REPOS: HelpSection = (
    "repos",
    &[
        ("s", "sync every repo (fetch, prune)"),
        ("S", "sync and pull"),
    ],
);
const BRANCHES: HelpSection = (
    "branches",
    &[
        ("s", "sync (fetch, prune)"),
        ("S", "sync and pull this one"),
        ("d", "delete, asking first"),
        ("u", "put back the last delete"),
    ],
);
const TAGS: HelpSection = (
    "tags",
    &[
        ("t", "keep a copy of the remote's tags"),
        ("d", "delete"),
        ("u", "put back the last delete"),
    ],
);
const COMMITS: HelpSection = ("commits", &[("↵", "the selected file's diff")]);
const STASHES: HelpSection = (
    "stashes",
    &[
        ("↵", "the selected file's diff"),
        ("a", "apply"),
        ("p", "pop"),
        ("d", "delete, asking its name first"),
    ],
);
const DIFF: HelpSection = (
    "diff",
    &[
        ("↵", "git difftool"),
        (CTRL_Y_MOVE, "scroll"),
        (CTRL_X_MOVE, "pan"),
    ],
);
const IN_A_BOX: HelpSection = (
    "in a box",
    &[
        ("y n", "answer"),
        (X_MOVE, "pick"),
        ("↵", "select"),
        (Y_MOVE, "scroll a long one"),
        ("esc", "cancel, or close a long one"),
    ],
);
const IN_A_FIND: HelpSection = (
    "in a find",
    &[
        ("type", "filter"),
        ("↵", "keep"),
        ("esc", "back"),
        ("?", "help"),
    ],
);
const IN_THIS_HELP: HelpSection = (
    "in this help",
    &[
        (Y_MOVE, "scroll"),
        ("ctrl-d ctrl-u", "half a page down, up"),
        ("g G", "the top, the bottom"),
        ("esc q ?", "close"),
    ],
);

/// Every key the app answers to, grouped by screen, for the reader `?` opens:
/// the keys every level shares, then each level this app can reach from where
/// it was entered, so `ilog` does not list the repos it never shows. The panel
/// scrolls, so a new row costs nothing but its line.
fn help(app: &App) -> Vec<HelpSection> {
    let moving = match app.stashes {
        true => MOVING_STASH,
        false => MOVING,
    };
    let mut sections = vec![moving, BELOW, EVERYWHERE];
    let reaches = |level: Level| match level {
        Level::Tags => app.start == Level::Tags,
        Level::Commits => !app.stashes,
        level => app.start <= level,
    };
    if reaches(Level::Repos) {
        sections.push(REPOS);
    }
    if reaches(Level::Branches) {
        sections.push(BRANCHES);
    }
    if reaches(Level::Tags) {
        sections.push(TAGS);
    }
    if reaches(Level::Commits) {
        sections.push(COMMITS);
    }
    if app.stashes {
        sections.push(STASHES);
    }
    sections.extend([DIFF, IN_A_BOX, IN_A_FIND, IN_THIS_HELP]);
    sections
}

/// Render one list pane with its scrollbar.
fn list(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem<'static>>,
    title: Line<'static>,
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

/// Render the diff pane.
fn diff(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!(" {} ", app.file_path().unwrap_or(""));
    // An empty diff and one still being highlighted look identical, so say
    // which it is once the load has run long enough to be noticed.
    let body = if app.diff.lines.is_empty() && app.dfeed.slow() {
        Text::from(Line::from(Span::styled(
            "  loading…",
            Style::default().add_modifier(Modifier::DIM),
        )))
    } else {
        app.diff.clone()
    };
    let view = Paragraph::new(body)
        .block(pane_block(title, true))
        .scroll((app.diff_scroll, 0));
    frame.render_widget(view, area);
    diff_scrollbar(frame, area, app.diff.lines.len(), app.diff_scroll);
    let cell = app.prepared.cell_width(area.width.saturating_sub(2));
    diff_hscrollbar(frame, area, app.prepared.max_line(), cell, app.diff_hscroll);
}

/// `" <name>  <scope tabs>  <i>/<n>   <filter> "`, the title every pane wears. A pane
/// still being streamed into says so with a trailing `…`, so a list that is
/// merely short is never mistaken for one that has finished arriving - but only
/// once the load has run long enough to be noticed, or every keypress on a fast
/// repo would blink it on and off.
fn title(
    name: &str,
    scope: Option<(&[&str], usize)>,
    sel: &Sel,
    loading: bool,
    pane: Pane,
    editing: bool,
    filterable: bool,
) -> Line<'static> {
    // Each tab carries a space either side, so the gaps around the tabs are one
    // narrower than the ones around a title without them.
    let mut spans = vec![Span::raw(format!(" {name} "))];
    if let Some((labels, picked)) = scope {
        spans.extend(scope_tabs(labels, picked));
    }
    spans.push(Span::raw(" "));
    let count = if sel.is_empty() {
        let body = if loading { "…" } else { "(none)" };
        body.to_string()
    } else {
        let dots = if loading { "…" } else { "" };
        format!("{}/{}{dots}", sel.cur + 1, sel.len())
    };
    spans.push(Span::raw(count));
    // The key that would open a filter on this pane sits where the filter will
    // show once typed, rather than in the footer, which cannot say which pane.
    spans.extend(sel.query.title_span(pane.sigil(), editing, filterable));
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// A level's scope stops by name, for its tabs.
fn stops<T: Copy>(scopes: &[T], label: fn(T) -> &'static str) -> Vec<&'static str> {
    scopes.iter().map(|&s| label(s)).collect()
}

/// The tags pane's name, which says what the marks are read against and so
/// never changes while it is open, and what that remote is doing, which does.
fn tags_label(app: &App) -> (String, Option<String>) {
    let name = match &app.tag_remote {
        Some(remote) => format!("tags vs {remote}"),
        None => "tags".to_string(),
    };
    let status = match &app.remote {
        None => Some("asking the remote…".to_string()),
        Some(RemoteTags::NoRemote) => Some("no remote".to_string()),
        Some(RemoteTags::Unreachable(_)) => Some("unreachable".to_string()),
        Some(RemoteTags::Answered { fresh: true, .. }) => None,
        // git's copy, shown while the remote is asked again, or kept because it
        // could not be.
        Some(RemoteTags::Answered { .. }) if app.rfeed.loading => Some("refreshing…".to_string()),
        Some(RemoteTags::Answered { .. }) => Some("as of last fetch".to_string()),
    };
    (name, status)
}

/// The commits pane names the paths it was filtered to, or the tag range it
/// holds.
fn commits_label(app: &App) -> String {
    if app.stashes {
        "stashes".to_string()
    } else if app.start == Level::Tags {
        app.range_label()
    } else if app.paths.is_empty() {
        "commits".to_string()
    } else {
        format!("commits · {}", app.paths.join(" "))
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
        cell(format!("{mark:<2}"), mark_style),
        Span::styled(
            format!("{:<NAME_W$}", truncate(&b.name, NAME_W)),
            name_style,
        ),
        cell(format!("  {:<10}", b.status()), mark_style),
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

fn tag_items(app: &App) -> Vec<ListItem<'static>> {
    let name_w = app
        .tsel
        .visible
        .iter()
        .map(|&i| app.tags[i].name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(1, NAME_W);
    let asking = app.remote.is_none();
    app.tsel
        .visible
        .iter()
        .map(|&i| tag_item(&app.tags[i], name_w, asking))
        .collect()
}

/// Mark, name, date, where it stands, its kind, and its message. A tag only the
/// remote has has no date or message here to show.
fn tag_item(t: &Tag, name_w: usize, asking: bool) -> ListItem<'static> {
    let (mark, mark_style) = tag_mark(t.state);
    let name_color = if t.state == TagState::Remote {
        Color::Magenta
    } else {
        Color::Cyan
    };
    let kind = if t.annotated {
        "annotated"
    } else {
        "lightweight"
    };
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        cell(format!("{mark:<2}"), mark_style),
        Span::styled(
            format!("{:<name_w$}", truncate(&t.name, name_w)),
            Style::default().fg(name_color),
        ),
        cell(
            format!("  {:<16}", t.date),
            Style::default().fg(Color::Green),
        ),
        state_cell(t.state, asking, mark_style),
        Span::styled(
            format!("  {kind:<11}"),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::raw(format!("  {}", t.subject)),
    ]))
}

/// Where a tag stands, or, while the remote is still being asked, that it is:
/// a blank there reads as "nothing to say" when the answer is simply not in.
fn state_cell(state: TagState, asking: bool, style: Style) -> Span<'static> {
    if asking && state == TagState::Unknown {
        return Span::styled(
            format!("  {:<11}", "checking…"),
            Style::default().add_modifier(Modifier::DIM),
        );
    }
    Span::styled(format!("  {:<11}", state.label()), style)
}

/// Quick-scan glyph: `↑` (not pushed, yellow), `⚑` (the two sides differ,
/// red), `↓` (only the remote has it, magenta), nothing once it is in step.
fn tag_mark(state: TagState) -> (&'static str, Style) {
    match state {
        TagState::Local => (
            "↑",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        TagState::Differs => ("⚑", Style::default().fg(Color::Red)),
        TagState::Remote => ("↓", Style::default().fg(Color::Magenta)),
        TagState::Pushed | TagState::Unknown => ("", Style::default().fg(Color::DarkGray)),
    }
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

/// `width` is the pane's, which a row gives up two borders and the cursor to.
fn commit_items(app: &App, width: u16) -> Vec<ListItem<'static>> {
    if app.csel.is_empty() && app.cfeed.slow() {
        return loading_body();
    }
    let row = (width as usize).saturating_sub(4);
    app.csel
        .visible
        .iter()
        .map(|&i| {
            let c = &app.commits[i];
            commit_item(c, app.unpushed.contains(&c.hash), row)
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

/// A padded column, without its colour when it holds nothing: the highlight
/// reverses each cell's colours, so a coloured blank comes out as a solid bar
/// on the selected row.
fn cell(text: String, style: Style) -> Span<'static> {
    if text.trim().is_empty() {
        Span::raw(text)
    } else {
        Span::styled(text, style)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

impl App {
    /// `?`: every key, in a reader over the panes.
    pub(super) fn help(&mut self) {
        self.modal = Some(Modal::help(&help(self)));
    }
}
