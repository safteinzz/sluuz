//! `slu ibranch` — interactive branch explorer (TUI). One level above `ilog`:
//! pick a branch, then browse its commits → a commit's files → a file's diff,
//! exactly like `ilog`.
//!
//! Branches (top, j/k) preview their commits (Ctrl-j/k); opening a branch shows
//! its commits (top, j/k) with the selected commit's files (Ctrl-j/k); opening a
//! file shows its side-by-side, syntax-highlighted diff. Esc / Ctrl-[ steps back
//! one level at a time; q / Ctrl-C quits. Arrow keys mirror j/k throughout.

use crate::git::git_capture;
use crate::tui::{
    clamp_hscroll, clamp_scroll, commit_item, diff_scrollbar, difftool_commit, file_item,
    half_page, is_back, is_down, is_left, is_open, is_right, is_up, load_commits, load_diff_raw,
    load_files, norm_esc, pane_block, pane_height, pane_width, pop_keyboard_enhancement,
    prepare_diff, push_keyboard_enhancement, list_scrollbar, render_prepared, Commit, FileEntry,
    RenderedDiff, CTRL_X_MOVE, CTRL_Y_MOVE, Y_MOVE, SEP,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::io::{self, IsTerminal};

/// How many commits to load per branch.
const COMMITS_PER_BRANCH: usize = 200;

#[derive(clap::Args)]
pub struct Args {
    /// Show local and remote-tracking branches
    #[arg(short, long)]
    pub all: bool,

    /// Show only remote-tracking branches (like `git branch -r`)
    #[arg(short, long)]
    pub remotes: bool,
}

struct Branch {
    is_head: bool,
    name: String,
    rel: String,
    author: String,
}

/// Current level of the drill-down.
enum View {
    /// Top: branches (j/k). Bottom: that branch's commits (Ctrl-j/k).
    Branches,
    /// Top: the branch's commits (j/k). Bottom: the commit's files (Ctrl-j/k).
    Commit,
    /// One file's side-by-side diff.
    Diff,
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu ibranch needs an interactive terminal — use `slu tidy` / `git branch` instead");
        return;
    }

    let branches = load_branches(args.all, args.remotes);
    if branches.is_empty() {
        eprintln!("no branches (or not a git repo)");
        return;
    }

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &branches, enhanced);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu ibranch: {e}");
    }
}

fn event_loop(terminal: &mut DefaultTerminal, branches: &[Branch], enhanced: bool) -> io::Result<()> {
    let mut width = pane_width(terminal);
    let mut view = View::Branches;
    let mut msg: Option<String> = None; // transient status (e.g. difftool result)

    let mut branch_sel = 0usize;
    let mut commits: Vec<Commit> = load_commits(&[branches[0].name.as_str()], COMMITS_PER_BRANCH);
    let mut commit_sel = 0usize;
    let mut files: Vec<FileEntry> = Vec::new();
    let mut file_sel = 0usize;
    let mut diff = Text::default();
    let mut prepared = RenderedDiff::default();
    let mut diff_scroll = 0u16;
    let mut diff_hscroll = 0u16;

    let mut branch_state = ListState::default();
    branch_state.select(Some(0));
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
                    branch_sel,
                    commits: &commits,
                    commit_sel,
                    files: &files,
                    file_sel,
                    diff: &diff,
                    diff_scroll,
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
                msg = None; // any keypress clears a stale status message

                if matches!(code, KeyCode::Char('q')) || (ctrl && code == KeyCode::Char('c')) {
                    break;
                }

                match view {
                    // ── Level 1: branches (j/k) + commits preview (Ctrl-j/k) ──
                    View::Branches => {
                        if ctrl && is_down(code) {
                            move_sel(&mut commit_sel, commits.len(), 1, &mut commit_state);
                        } else if ctrl && is_up(code) {
                            move_sel(&mut commit_sel, commits.len(), -1, &mut commit_state);
                        } else if is_down(code) && branch_sel + 1 < branches.len() {
                            branch_sel += 1;
                            branch_state.select(Some(branch_sel));
                            commits = load_commits(&[branches[branch_sel].name.as_str()], COMMITS_PER_BRANCH);
                            commit_sel = 0;
                            commit_state.select(Some(0));
                        } else if is_up(code) && branch_sel > 0 {
                            branch_sel -= 1;
                            branch_state.select(Some(branch_sel));
                            commits = load_commits(&[branches[branch_sel].name.as_str()], COMMITS_PER_BRANCH);
                            commit_sel = 0;
                            commit_state.select(Some(0));
                        } else if is_open(code) && !commits.is_empty() {
                            files = load_files(&commits[commit_sel].hash);
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
                            files = load_files(&commits[commit_sel].hash);
                            file_sel = 0;
                            file_state.select(Some(0));
                        } else if is_up(code) && commit_sel > 0 {
                            commit_sel -= 1;
                            commit_state.select(Some(commit_sel));
                            files = load_files(&commits[commit_sel].hash);
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
                                pane_width(terminal) / 2,
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
    branch_sel: usize,
    commits: &'a [Commit],
    commit_sel: usize,
    files: &'a [FileEntry],
    file_sel: usize,
    diff: &'a Text<'static>,
    diff_scroll: u16,
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

    match p.view {
        // Level 1: branches on top, commits preview on the bottom.
        View::Branches => {
            let top = List::new(p.branches.iter().map(branch_item).collect::<Vec<_>>())
                .block(pane_block(
                    format!(" branches  {}/{}   {Y_MOVE} ", p.branch_sel + 1, p.branches.len()),
                    true,
                ))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(top, areas[0], branch_state);
            list_scrollbar(frame, areas[0], p.branches.len(), branch_state.offset());

            let title =
                commits_title(p.commits, p.commit_sel, &format!("{CTRL_Y_MOVE} select · enter open"));
            let bottom = List::new(p.commits.iter().map(commit_item).collect::<Vec<_>>())
                .block(pane_block(title, true))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(bottom, areas[1], commit_state);
            list_scrollbar(frame, areas[1], p.commits.len(), commit_state.offset());
        }

        // Level 2: commits on top, the commit's files on the bottom.
        View::Commit => {
            let title = commits_title(p.commits, p.commit_sel, Y_MOVE);
            let top = List::new(p.commits.iter().map(commit_item).collect::<Vec<_>>())
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
            let top = List::new(p.commits.iter().map(commit_item).collect::<Vec<_>>())
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
    } else {
        Style::default().fg(Color::Cyan)
    };
    ListItem::new(Line::from(vec![
        Span::raw(if b.is_head { "● " } else { "  " }),
        Span::styled(format!("{:<28}", truncate(&b.name, 28)), name_style),
        Span::styled(format!("  {:<16}", b.rel), Style::default().fg(Color::Magenta)),
        Span::styled(format!("  {}", b.author), Style::default().fg(Color::Blue)),
    ]))
}

fn load_branches(all: bool, remotes_only: bool) -> Vec<Branch> {
    let format_arg = format!(
        "--format=%(HEAD){SEP}%(refname:short){SEP}%(committerdate:relative){SEP}%(authorname)"
    );
    let mut args = vec!["for-each-ref", "--sort=-committerdate", &format_arg];
    // default: local; -r: remotes only; -a: both.
    if remotes_only && !all {
        args.push("refs/remotes");
    } else if all {
        args.push("refs/heads");
        args.push("refs/remotes");
    } else {
        args.push("refs/heads");
    }
    git_capture(".", &args)
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    let head = f.next()?;
                    Some(Branch {
                        is_head: head.trim() == "*",
                        name: f.next()?.to_string(),
                        rel: f.next()?.to_string(),
                        author: f.next().unwrap_or("").to_string(),
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
