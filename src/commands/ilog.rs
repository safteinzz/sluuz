//! `slu ilog` — interactive log explorer (TUI). The "i" prefix marks the
//! interactive views (`ilog`, `ibranch`).
//!
//! Three levels, so navigating is instant and the heavy work is lazy:
//! commits (top, j/k) show the selected commit's files in the bottom pane;
//! Ctrl-j/k pick a file; Enter opens it into a side-by-side, syntax-highlighted
//! diff. `h`/`l` (or `←`/`→`) slide the scope between local (unpushed), all, and
//! pushed. Scroll the diff with Ctrl-j/k, Ctrl-d/u (vim half-page) or PgDn/PgUp.
//! q / Ctrl-C quit; Esc steps back. Arrow keys mirror j/k everywhere.

use crate::git::git_capture;
use crate::tui::{
    clamp_hscroll, clamp_scroll, commit_item, diff_hscrollbar, diff_scrollbar, difftool_commit,
    file_item, half_page, is_back, is_down, is_left, is_open, is_right, is_up, list_scrollbar,
    load_commits, load_diff_raw, load_files, load_unpushed, norm_esc, pane_block, pane_height,
    pane_width, pop_keyboard_enhancement, prepare_diff, push_keyboard_enhancement, render_prepared,
    Commit, FileEntry, RenderedDiff, CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE, Y_MOVE,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::collections::HashSet;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {
    /// Include commits from all branches
    #[arg(short, long)]
    pub all: bool,

    /// Maximum number of commits to load
    #[arg(short = 'n', long, default_value_t = 200)]
    pub number: usize,
}

/// Which level the user is currently navigating.
enum View {
    /// Browsing: j/k move commits (top), Ctrl-j/k move the file selection (bottom).
    Browse,
    /// Viewing one file's side-by-side diff.
    Diff,
}

/// Push-state filter over the loaded commits.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    /// Only commits that are on no remote yet (unpushed).
    Local,
    All,
    /// Only commits that are on some remote (pushed).
    Pushed,
}

/// Left→right order for the `h`/`l` slider; `All` in the middle.
const SCOPES: [Scope; 3] = [Scope::Local, Scope::All, Scope::Pushed];

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::All => "all",
            Scope::Pushed => "pushed",
        }
    }
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu ilog needs an interactive terminal — use `slu trace` for plain output");
        return;
    }

    // Anchor at the repo root: git reports file paths root-relative, so
    // `git show -- <path>` (and the difftool) wouldn't resolve from a
    // subdirectory — the diff pane would come up blank.
    if let Some(root) = git_capture(".", &["rev-parse", "--show-toplevel"]) {
        let _ = std::env::set_current_dir(&root);
    }

    let extra: &[&str] = if args.all { &["--all"] } else { &[] };
    let commits = load_commits(extra, args.number);
    if commits.is_empty() {
        eprintln!("no commits (or not a git repo)");
        return;
    }
    let unpushed = load_unpushed();

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &commits, &unpushed, enhanced);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu ilog: {e}");
    }
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    commits: &[Commit],
    unpushed: &HashSet<String>,
    enhanced: bool,
) -> io::Result<()> {
    let mut width = pane_width(terminal);
    let mut scope_idx = 1usize; // default: All
    let mut visible = visible_commits(commits, unpushed, SCOPES[scope_idx]);
    let mut commit_sel = 0usize; // index into `visible`
    let mut files = files_for(commits, &visible, commit_sel);
    let mut file_sel = 0usize;
    let mut view = View::Browse;
    let mut diff = Text::default();
    let mut prepared = RenderedDiff::default();
    let mut diff_scroll = 0u16;
    let mut diff_hscroll = 0u16;
    let mut msg: Option<String> = None; // transient status (e.g. difftool result)

    let mut commit_state = ListState::default();
    commit_state.select((!visible.is_empty()).then_some(0));
    let mut file_state = ListState::default();
    file_state.select(Some(0));

    loop {
        terminal.draw(|frame| {
            draw(
                frame,
                Panes {
                    commits,
                    visible: &visible,
                    unpushed,
                    scope: SCOPES[scope_idx],
                    commit_sel,
                    files: &files,
                    file_sel,
                    view: &view,
                    diff: &diff,
                    diff_scroll,
                    prepared: &prepared,
                    diff_hscroll,
                    status: msg.as_deref(),
                },
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
                msg = None; // any keypress clears a stale status message

                if matches!(code, KeyCode::Char('q')) || (ctrl && code == KeyCode::Char('c')) {
                    break;
                }

                match view {
                    View::Browse => {
                        if ctrl && is_down(code) {
                            if file_sel + 1 < files.len() {
                                file_sel += 1;
                                file_state.select(Some(file_sel));
                            }
                        } else if ctrl && is_up(code) {
                            if file_sel > 0 {
                                file_sel -= 1;
                                file_state.select(Some(file_sel));
                            }
                        } else if is_down(code) {
                            if commit_sel + 1 < visible.len() {
                                commit_sel += 1;
                                commit_state.select(Some(commit_sel));
                                files = files_for(commits, &visible, commit_sel);
                                file_sel = 0;
                                file_state.select(Some(0));
                            }
                        } else if is_up(code) {
                            if commit_sel > 0 {
                                commit_sel -= 1;
                                commit_state.select(Some(commit_sel));
                                files = files_for(commits, &visible, commit_sel);
                                file_sel = 0;
                                file_state.select(Some(0));
                            }
                        } else if !ctrl && is_left(code) && scope_idx > 0 {
                            scope_idx -= 1;
                            (visible, commit_sel, files) =
                                rescope(commits, unpushed, SCOPES[scope_idx], &mut commit_state);
                            file_sel = 0;
                            file_state.select(Some(0));
                        } else if !ctrl && is_right(code) && scope_idx + 1 < SCOPES.len() {
                            scope_idx += 1;
                            (visible, commit_sel, files) =
                                rescope(commits, unpushed, SCOPES[scope_idx], &mut commit_state);
                            file_sel = 0;
                            file_state.select(Some(0));
                        } else if is_open(code) && !files.is_empty() {
                            if let Some(&i) = visible.get(commit_sel) {
                                let raw = load_diff_raw(&commits[i].hash, &files[file_sel].path);
                                prepared = prepare_diff(&raw);
                                diff_hscroll = 0;
                                diff = render_prepared(&prepared, width, 0);
                                diff_scroll = 0;
                                view = View::Diff;
                            }
                        } else if code == KeyCode::Esc {
                            break;
                        }
                    }
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
                            if let Some(&i) = visible.get(commit_sel) {
                                let m = difftool_commit(
                                    terminal,
                                    enhanced,
                                    ".",
                                    &commits[i].hash,
                                    &files[file_sel].path,
                                );
                                width = pane_width(terminal);
                                if !m.is_empty() {
                                    msg = Some(m);
                                }
                            }
                        } else if is_back(code) {
                            view = View::Browse;
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

/// Indices of `commits` that belong in `scope`.
fn visible_commits(commits: &[Commit], unpushed: &HashSet<String>, scope: Scope) -> Vec<usize> {
    commits
        .iter()
        .enumerate()
        .filter(|(_, c)| match scope {
            Scope::Local => unpushed.contains(&c.hash),
            Scope::Pushed => !unpushed.contains(&c.hash),
            Scope::All => true,
        })
        .map(|(i, _)| i)
        .collect()
}

/// The files of the commit currently selected in `visible`, or empty.
fn files_for(commits: &[Commit], visible: &[usize], commit_sel: usize) -> Vec<FileEntry> {
    match visible.get(commit_sel) {
        Some(&i) => load_files(&commits[i].hash),
        None => Vec::new(),
    }
}

/// Recompute the visible set for a new scope, resetting the selection to the top.
fn rescope(
    commits: &[Commit],
    unpushed: &HashSet<String>,
    scope: Scope,
    commit_state: &mut ListState,
) -> (Vec<usize>, usize, Vec<FileEntry>) {
    let visible = visible_commits(commits, unpushed, scope);
    commit_state.select((!visible.is_empty()).then_some(0));
    let files = files_for(commits, &visible, 0);
    (visible, 0, files)
}

struct Panes<'a> {
    commits: &'a [Commit],
    visible: &'a [usize],
    unpushed: &'a HashSet<String>,
    scope: Scope,
    commit_sel: usize,
    files: &'a [FileEntry],
    file_sel: usize,
    view: &'a View,
    diff: &'a Text<'static>,
    diff_scroll: u16,
    prepared: &'a RenderedDiff,
    diff_hscroll: u16,
    status: Option<&'a str>,
}

fn draw(
    frame: &mut ratatui::Frame,
    p: Panes,
    commit_state: &mut ListState,
    file_state: &mut ListState,
) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());

    let browsing = matches!(p.view, View::Browse);

    let items: Vec<ListItem> = p
        .visible
        .iter()
        .map(|&i| commit_item(&p.commits[i], p.unpushed.contains(&p.commits[i].hash)))
        .collect();
    let commits_title = if p.visible.is_empty() {
        format!(" commits · {}  (none)   {X_MOVE} scope ", p.scope.label())
    } else {
        format!(
            " commits · {}  {}/{}   {Y_MOVE} · {X_MOVE} scope · ↑=unpushed ",
            p.scope.label(),
            p.commit_sel + 1,
            p.visible.len()
        )
    };
    let commits_list = List::new(items)
        .block(pane_block(commits_title, browsing))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(commits_list, areas[0], commit_state);
    list_scrollbar(frame, areas[0], p.visible.len(), commit_state.offset());

    match p.view {
        View::Diff => {
            let path = p.files.get(p.file_sel).map(|f| f.path.as_str()).unwrap_or("");
            let title = match p.status {
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
        _ => {
            let title = if p.files.is_empty() {
                " files  (none) ".to_string()
            } else {
                format!(
                    " files  {}/{}   {CTRL_Y_MOVE} select · enter open · q quit ",
                    p.file_sel + 1,
                    p.files.len()
                )
            };
            let list = List::new(p.files.iter().map(file_item).collect::<Vec<_>>())
                .block(pane_block(title, browsing))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(list, areas[1], file_state);
            list_scrollbar(frame, areas[1], p.files.len(), file_state.offset());
        }
    }
}
