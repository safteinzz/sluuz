//! `slu ibranch` — interactive branch explorer (TUI). One level above `ilog`:
//! pick a branch, then browse its commits → a commit's files → a file's diff,
//! exactly like `ilog`.
//!
//! Branches (top, j/k) preview their commits (Ctrl-j/k); `h`/`l` slide the scope
//! between local, all, and remote. Opening a branch shows its commits (top, j/k)
//! with the selected commit's files (Ctrl-j/k); opening a file shows its diff.
//! Esc / Ctrl-[ steps back one level; q / Ctrl-C quits. Arrow keys mirror j/k.
//!
//! Push state is visible at a glance: a branch marked `↑` has no remote yet (or
//! its upstream is gone / it's ahead); commits marked `↑` aren't pushed anywhere.

use crate::git::git_capture;
use crate::tui::{
    clamp_hscroll, clamp_scroll, commit_item, diff_hscrollbar, diff_scrollbar, difftool_commit,
    file_item, half_page, is_back, is_down, is_left, is_open, is_right, is_up, list_scrollbar,
    load_commits, load_diff_raw, load_files, load_unpushed, norm_esc, pane_block, pane_height,
    pane_width, pop_keyboard_enhancement, prepare_diff, push_keyboard_enhancement, render_prepared,
    Commit, FileEntry, RenderedDiff, CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE, SEP,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::collections::HashSet;
use std::io::{self, IsTerminal};

/// How many commits to load per branch.
const COMMITS_PER_BRANCH: usize = 200;

#[derive(clap::Args)]
pub struct Args {
    /// Start in the "all" scope (local + remote-tracking branches)
    #[arg(short, long)]
    pub all: bool,

    /// Start in the "remote" scope (remote-tracking branches only)
    #[arg(short, long)]
    pub remotes: bool,
}

struct Branch {
    is_head: bool,
    remote: bool,
    name: String,
    rel: String,
    author: String,
    has_upstream: bool,
    track: String, // raw %(upstream:track): "", "[gone]", "[ahead 2, behind 1]", …
}

impl Branch {
    /// A local branch that isn't fully on a remote: never pushed (no upstream),
    /// its upstream was deleted (`[gone]`), or it's ahead of its upstream.
    fn unpushed(&self) -> bool {
        !self.remote
            && (!self.has_upstream || self.track.contains("gone") || self.track.contains("ahead"))
    }
}

/// Which slice of branches the top pane shows.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Local,
    All,
    Remote,
}

/// Left→right order for the `h`/`l` slider; `All` in the middle.
const SCOPES: [Scope; 3] = [Scope::Local, Scope::All, Scope::Remote];

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::All => "all",
            Scope::Remote => "remote",
        }
    }
    fn keeps(self, b: &Branch) -> bool {
        match self {
            Scope::Local => !b.remote,
            Scope::Remote => b.remote,
            Scope::All => true,
        }
    }
}

/// Current level of the drill-down.
enum View {
    Branches,
    Commit,
    Diff,
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu ibranch needs an interactive terminal — use `slu tidy` / `git branch` instead");
        return;
    }

    // Anchor at the repo root: git reports file paths root-relative, so
    // `git show -- <path>` (and the difftool) wouldn't resolve from a
    // subdirectory — the diff pane would come up blank.
    if let Some(root) = git_capture(".", &["rev-parse", "--show-toplevel"]) {
        let _ = std::env::set_current_dir(&root);
    }

    let branches = load_branches();
    if branches.is_empty() {
        eprintln!("no branches (or not a git repo)");
        return;
    }
    let unpushed = load_unpushed();
    // The -a/-r flags just pick the starting scope.
    let scope_idx = if args.remotes && !args.all {
        2
    } else if args.all {
        1
    } else {
        0
    };

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &branches, &unpushed, scope_idx, enhanced);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu ibranch: {e}");
    }
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    branches: &[Branch],
    unpushed: &HashSet<String>,
    mut scope_idx: usize,
    enhanced: bool,
) -> io::Result<()> {
    let mut width = pane_width(terminal);
    let mut view = View::Branches;
    let mut msg: Option<String> = None;

    let mut visible = visible_branches(branches, SCOPES[scope_idx]);
    let mut branch_sel = 0usize; // index into `visible`
    let mut commits: Vec<Commit> = commits_for(branches, &visible, branch_sel);
    let mut commit_sel = 0usize;
    let mut files: Vec<FileEntry> = Vec::new();
    let mut file_sel = 0usize;
    let mut diff = Text::default();
    let mut prepared = RenderedDiff::default();
    let mut diff_scroll = 0u16;
    let mut diff_hscroll = 0u16;

    let mut branch_state = ListState::default();
    branch_state.select((!visible.is_empty()).then_some(0));
    let mut commit_state = ListState::default();
    commit_state.select(Some(0));
    let mut file_state = ListState::default();
    file_state.select(Some(0));

    loop {
        terminal.draw(|frame| {
            draw(
                frame,
                Panes {
                    branches,
                    visible: &visible,
                    scope: SCOPES[scope_idx],
                    branch_sel,
                    unpushed,
                    commits: &commits,
                    commit_sel,
                    files: &files,
                    file_sel,
                    diff: &diff,
                    diff_scroll,
                    prepared: &prepared,
                    diff_hscroll,
                    view: &view,
                    msg: msg.as_deref(),
                },
                &mut branch_state,
                &mut commit_state,
                &mut file_state,
            )
        })?;

        match event::read()? {
            Event::Resize(_, _) => {
                let w = pane_width(terminal);
                if w != width {
                    width = w;
                    if matches!(view, View::Diff) {
                        diff = render_prepared(&prepared, width, diff_hscroll);
                    }
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let code = norm_esc(key.code, ctrl);
                msg = None;

                if matches!(code, KeyCode::Char('q')) || (ctrl && code == KeyCode::Char('c')) {
                    break;
                }

                match view {
                    // ── Level 1: branches (j/k) + commits preview (Ctrl-j/k), h/l scope ──
                    View::Branches => {
                        if ctrl && is_down(code) {
                            move_sel(&mut commit_sel, commits.len(), 1, &mut commit_state);
                        } else if ctrl && is_up(code) {
                            move_sel(&mut commit_sel, commits.len(), -1, &mut commit_state);
                        } else if is_down(code) && branch_sel + 1 < visible.len() {
                            branch_sel += 1;
                            branch_state.select(Some(branch_sel));
                            commits = commits_for(branches, &visible, branch_sel);
                            commit_sel = 0;
                            commit_state.select(Some(0));
                        } else if is_up(code) && branch_sel > 0 {
                            branch_sel -= 1;
                            branch_state.select(Some(branch_sel));
                            commits = commits_for(branches, &visible, branch_sel);
                            commit_sel = 0;
                            commit_state.select(Some(0));
                        } else if !ctrl && is_left(code) && scope_idx > 0 {
                            scope_idx -= 1;
                            (visible, branch_sel, commits) =
                                rescope(branches, SCOPES[scope_idx], &mut branch_state);
                            commit_sel = 0;
                            commit_state.select(Some(0));
                        } else if !ctrl && is_right(code) && scope_idx + 1 < SCOPES.len() {
                            scope_idx += 1;
                            (visible, branch_sel, commits) =
                                rescope(branches, SCOPES[scope_idx], &mut branch_state);
                            commit_sel = 0;
                            commit_state.select(Some(0));
                        } else if is_open(code) && !commits.is_empty() {
                            files = load_files(&commits[commit_sel].hash, &[]);
                            file_sel = 0;
                            file_state.select(Some(0));
                            view = View::Commit;
                        } else if code == KeyCode::Esc {
                            break;
                        }
                    }

                    // ── Level 2: commits (j/k) + files (Ctrl-j/k) ──
                    View::Commit => {
                        if ctrl && is_down(code) {
                            move_sel(&mut file_sel, files.len(), 1, &mut file_state);
                        } else if ctrl && is_up(code) {
                            move_sel(&mut file_sel, files.len(), -1, &mut file_state);
                        } else if is_down(code) && commit_sel + 1 < commits.len() {
                            commit_sel += 1;
                            commit_state.select(Some(commit_sel));
                            files = load_files(&commits[commit_sel].hash, &[]);
                            file_sel = 0;
                            file_state.select(Some(0));
                        } else if is_up(code) && commit_sel > 0 {
                            commit_sel -= 1;
                            commit_state.select(Some(commit_sel));
                            files = load_files(&commits[commit_sel].hash, &[]);
                            file_sel = 0;
                            file_state.select(Some(0));
                        } else if is_open(code) && !files.is_empty() {
                            let raw = load_diff_raw(&commits[commit_sel].hash, &files[file_sel].path);
                            prepared = prepare_diff(&raw);
                            diff_hscroll = 0;
                            diff = render_prepared(&prepared, width, 0);
                            diff_scroll = 0;
                            view = View::Diff;
                        } else if is_back(code) {
                            view = View::Branches;
                        }
                    }

                    // ── Level 3: diff ──
                    View::Diff => {
                        let half = half_page(terminal);
                        if ctrl && is_down(code) {
                            diff_scroll = diff_scroll.saturating_add(3);
                        } else if ctrl && is_up(code) {
                            diff_scroll = diff_scroll.saturating_sub(3);
                        } else if ctrl && code == KeyCode::Char('d') {
                            diff_scroll = diff_scroll.saturating_add(half);
                        } else if ctrl && code == KeyCode::Char('u') {
                            diff_scroll = diff_scroll.saturating_sub(half);
                        } else if code == KeyCode::PageDown {
                            diff_scroll = diff_scroll.saturating_add(10);
                        } else if code == KeyCode::PageUp {
                            diff_scroll = diff_scroll.saturating_sub(10);
                        } else if ctrl && is_right(code) {
                            diff_hscroll = clamp_hscroll(
                                diff_hscroll.saturating_add(8),
                                prepared.max_line(),
                                prepared.cell_width(width),
                            );
                            diff = render_prepared(&prepared, width, diff_hscroll);
                        } else if ctrl && is_left(code) {
                            diff_hscroll = diff_hscroll.saturating_sub(8);
                            diff = render_prepared(&prepared, width, diff_hscroll);
                        } else if is_open(code) {
                            let m = difftool_commit(
                                terminal,
                                enhanced,
                                ".",
                                &commits[commit_sel].hash,
                                &files[file_sel].path,
                            );
                            width = pane_width(terminal);
                            if !m.is_empty() {
                                msg = Some(m);
                            }
                        } else if is_back(code) {
                            view = View::Commit;
                        }
                        diff_scroll =
                            clamp_scroll(diff_scroll, diff.lines.len(), pane_height(terminal));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Indices of `branches` in the given scope.
fn visible_branches(branches: &[Branch], scope: Scope) -> Vec<usize> {
    branches
        .iter()
        .enumerate()
        .filter(|(_, b)| scope.keeps(b))
        .map(|(i, _)| i)
        .collect()
}

/// Commits of the branch currently selected in `visible`, or empty.
fn commits_for(branches: &[Branch], visible: &[usize], branch_sel: usize) -> Vec<Commit> {
    match visible.get(branch_sel) {
        Some(&i) => load_commits(&[branches[i].name.as_str()], COMMITS_PER_BRANCH),
        None => Vec::new(),
    }
}

/// Recompute the visible branches for a new scope, resetting to the top.
fn rescope(
    branches: &[Branch],
    scope: Scope,
    branch_state: &mut ListState,
) -> (Vec<usize>, usize, Vec<Commit>) {
    let visible = visible_branches(branches, scope);
    branch_state.select((!visible.is_empty()).then_some(0));
    let commits = commits_for(branches, &visible, 0);
    (visible, 0, commits)
}

/// Move a selection index by +1/-1 within bounds and update its list state.
fn move_sel(sel: &mut usize, len: usize, delta: i32, state: &mut ListState) {
    if delta > 0 && *sel + 1 < len {
        *sel += 1;
    } else if delta < 0 && *sel > 0 {
        *sel -= 1;
    }
    state.select(Some(*sel));
}

struct Panes<'a> {
    branches: &'a [Branch],
    visible: &'a [usize],
    scope: Scope,
    branch_sel: usize,
    unpushed: &'a HashSet<String>,
    commits: &'a [Commit],
    commit_sel: usize,
    files: &'a [FileEntry],
    file_sel: usize,
    diff: &'a Text<'static>,
    diff_scroll: u16,
    prepared: &'a RenderedDiff,
    diff_hscroll: u16,
    view: &'a View,
    msg: Option<&'a str>,
}

fn draw(
    frame: &mut ratatui::Frame,
    p: Panes,
    branch_state: &mut ListState,
    commit_state: &mut ListState,
    file_state: &mut ListState,
) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());

    let commit_items = |commits: &[Commit]| -> Vec<ListItem<'static>> {
        commits
            .iter()
            .map(|c| commit_item(c, p.unpushed.contains(&c.hash)))
            .collect()
    };

    match p.view {
        // Level 1: branches on top, commits preview on the bottom.
        View::Branches => {
            let items: Vec<ListItem> =
                p.visible.iter().map(|&i| branch_item(&p.branches[i])).collect();
            let btitle = if p.visible.is_empty() {
                format!(" branches · {}  (none)   {X_MOVE} scope ", p.scope.label())
            } else {
                format!(
                    " branches · {}  {}/{}   {Y_MOVE} · {X_MOVE} scope · ↑=unpushed ",
                    p.scope.label(),
                    p.branch_sel + 1,
                    p.visible.len()
                )
            };
            let top = List::new(items)
                .block(pane_block(btitle, true))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(top, areas[0], branch_state);
            list_scrollbar(frame, areas[0], p.visible.len(), branch_state.offset());

            let title =
                commits_title(p.commits, p.commit_sel, &format!("{CTRL_Y_MOVE} select · enter open"));
            let bottom = List::new(commit_items(p.commits))
                .block(pane_block(title, true))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(bottom, areas[1], commit_state);
            list_scrollbar(frame, areas[1], p.commits.len(), commit_state.offset());
        }

        // Level 2: commits on top, the commit's files on the bottom.
        View::Commit => {
            let title = commits_title(p.commits, p.commit_sel, Y_MOVE);
            let top = List::new(commit_items(p.commits))
                .block(pane_block(title, true))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(top, areas[0], commit_state);
            list_scrollbar(frame, areas[0], p.commits.len(), commit_state.offset());

            let files_title = if p.files.is_empty() {
                " files  (none) ".to_string()
            } else {
                format!(
                    " files  {}/{}   {CTRL_Y_MOVE} select · enter open · esc back ",
                    p.file_sel + 1,
                    p.files.len()
                )
            };
            let bottom = List::new(p.files.iter().map(file_item).collect::<Vec<_>>())
                .block(pane_block(files_title, true))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(bottom, areas[1], file_state);
            list_scrollbar(frame, areas[1], p.files.len(), file_state.offset());
        }

        // Level 3: commits on top (context), the file diff on the bottom.
        View::Diff => {
            let title = commits_title(p.commits, p.commit_sel, "");
            let top = List::new(commit_items(p.commits))
                .block(pane_block(title, false))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(top, areas[0], commit_state);
            list_scrollbar(frame, areas[0], p.commits.len(), commit_state.offset());

            let path = p.files.get(p.file_sel).map(|f| f.path.as_str()).unwrap_or("");
            let title = match p.msg {
                Some(m) => format!(" {path}   ⚠ {m} "),
                None => format!(
                    " {path}   enter difftool · {CTRL_Y_MOVE}·ctrl-d/u scroll · {CTRL_X_MOVE} pan · esc back · q quit "
                ),
            };
            let view = Paragraph::new(p.diff.clone())
                .block(pane_block(title, true))
                .scroll((p.diff_scroll, 0));
            frame.render_widget(view, areas[1]);
            diff_scrollbar(frame, areas[1], p.diff.lines.len(), p.diff_scroll);
            let cell = p.prepared.cell_width(areas[1].width.saturating_sub(2));
            diff_hscrollbar(frame, areas[1], p.prepared.max_line(), cell, p.diff_hscroll);
        }
    }
}

fn commits_title(commits: &[Commit], sel: usize, hint: &str) -> String {
    if commits.is_empty() {
        return " commits  (none) ".to_string();
    }
    if hint.is_empty() {
        format!(" commits  {}/{} ", sel + 1, commits.len())
    } else {
        format!(" commits  {}/{}   {hint} ", sel + 1, commits.len())
    }
}

fn branch_item(b: &Branch) -> ListItem<'static> {
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
        Span::styled(format!("{:<28}", truncate(&b.name, 28)), name_style),
        Span::styled(format!("  {:<10}", branch_status(b)), mark_style),
        Span::styled(format!("  {:<14}", b.rel), Style::default().fg(Color::Magenta)),
        Span::styled(format!("  {}", b.author), Style::default().fg(Color::Blue)),
    ]))
}

/// Quick-scan glyph: `↑` (unpushed/ahead, yellow), `⚑` (upstream gone, red),
/// nothing for a remote branch or an in-sync local one.
fn branch_mark(b: &Branch) -> (&'static str, Style) {
    if b.remote {
        return ("", Style::default().fg(Color::DarkGray));
    }
    if b.track.contains("gone") {
        return ("⚑", Style::default().fg(Color::Red));
    }
    if b.unpushed() {
        return ("↑", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    }
    ("", Style::default().fg(Color::DarkGray))
}

/// Short push-state text: "no remote", "gone", "↑N ↓M", or "synced".
fn branch_status(b: &Branch) -> String {
    if b.remote {
        return String::new();
    }
    if !b.has_upstream {
        return "no remote".to_string();
    }
    if b.track.contains("gone") {
        return "gone".to_string();
    }
    let ahead = count(&b.track, "ahead");
    let behind = count(&b.track, "behind");
    match (ahead, behind) {
        (Some(a), Some(bh)) => format!("↑{a} ↓{bh}"),
        (Some(a), None) => format!("↑{a}"),
        (None, Some(bh)) => format!("↓{bh}"),
        (None, None) => "synced".to_string(),
    }
}

/// Pull the number after `key` out of a `%(upstream:track)` string like
/// `[ahead 2, behind 1]`.
fn count(track: &str, key: &str) -> Option<u32> {
    let rest = &track[track.find(key)? + key.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .find(|t| !t.is_empty())
        .and_then(|t| t.parse().ok())
}

/// Load every branch (local + remote-tracking) with its push state, newest first.
fn load_branches() -> Vec<Branch> {
    let fmt = format!(
        "--format=%(HEAD){SEP}%(refname){SEP}%(refname:short){SEP}%(committerdate:relative){SEP}%(authorname){SEP}%(upstream){SEP}%(upstream:track)"
    );
    git_capture(
        ".",
        &["for-each-ref", "--sort=-committerdate", &fmt, "refs/heads", "refs/remotes"],
    )
    .map(|out| {
        out.lines()
            .filter_map(|line| {
                let mut f = line.split(SEP);
                let head = f.next()?;
                let refname = f.next()?;
                let short = f.next()?;
                let rel = f.next().unwrap_or("").to_string();
                let author = f.next().unwrap_or("").to_string();
                let upstream = f.next().unwrap_or("");
                let track = f.next().unwrap_or("").to_string();
                // Skip the symbolic `refs/remotes/*/HEAD` alias — it's noise.
                if refname.ends_with("/HEAD") {
                    return None;
                }
                Some(Branch {
                    is_head: head.trim() == "*",
                    remote: refname.starts_with("refs/remotes/"),
                    name: short.to_string(),
                    rel,
                    author,
                    has_upstream: !upstream.is_empty(),
                    track,
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
