//! `slu ilog` — interactive log explorer (TUI). The "i" prefix marks the
//! interactive views (`ilog`, `ibranch`).
//!
//! Three levels, so navigating is instant and the heavy work is lazy:
//! commits (top, j/k) show the selected commit's files in the bottom pane;
//! Ctrl-j/k pick a file; Enter opens it into a side-by-side, syntax-highlighted
//! diff. Scroll the diff with Ctrl-j/k, Ctrl-d/u (vim half-page) or PgDn/PgUp.
//! q / Ctrl-C quit; Esc steps back. Arrow keys mirror j/k everywhere.

use crate::tui::{
    commit_item, file_item, half_page, is_back, is_down, is_open, is_up, load_commits,
    load_file_diff, load_files, pane_block, pane_width, pop_keyboard_enhancement,
    push_keyboard_enhancement, Commit, FileEntry, CTRL_MOVE, MOVE,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{List, ListState, Paragraph};
use ratatui::DefaultTerminal;
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

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu ilog needs an interactive terminal — use `slu trace` for plain output");
        return;
    }

    let extra: &[&str] = if args.all { &["--all"] } else { &[] };
    let commits = load_commits(extra, args.number);
    if commits.is_empty() {
        eprintln!("no commits (or not a git repo)");
        return;
    }

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &commits);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu ilog: {e}");
    }
}

fn event_loop(terminal: &mut DefaultTerminal, commits: &[Commit]) -> io::Result<()> {
    let mut width = pane_width(terminal);
    let mut commit_sel = 0usize;
    let mut files = load_files(&commits[0].hash);
    let mut file_sel = 0usize;
    let mut view = View::Browse;
    let mut diff = Text::default();
    let mut diff_scroll = 0u16;

    let mut commit_state = ListState::default();
    commit_state.select(Some(0));
    let mut file_state = ListState::default();
    file_state.select(Some(0));

    loop {
        terminal.draw(|frame| {
            draw(
                frame,
                commits,
                commit_sel,
                &mut commit_state,
                &files,
                &mut file_state,
                &view,
                &diff,
                diff_scroll,
                file_sel,
            )
        })?;

        match event::read()? {
            Event::Resize(_, _) => {
                let w = pane_width(terminal);
                if w != width {
                    width = w;
                    if matches!(view, View::Diff) {
                        diff = load_file_diff(&commits[commit_sel].hash, &files[file_sel].path, width);
                    }
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let code = key.code;

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
                            if commit_sel + 1 < commits.len() {
                                commit_sel += 1;
                                commit_state.select(Some(commit_sel));
                                files = load_files(&commits[commit_sel].hash);
                                file_sel = 0;
                                file_state.select(Some(0));
                            }
                        } else if is_up(code) {
                            if commit_sel > 0 {
                                commit_sel -= 1;
                                commit_state.select(Some(commit_sel));
                                files = load_files(&commits[commit_sel].hash);
                                file_sel = 0;
                                file_state.select(Some(0));
                            }
                        } else if is_open(code) && !files.is_empty() {
                            diff = load_file_diff(&commits[commit_sel].hash, &files[file_sel].path, width);
                            diff_scroll = 0;
                            view = View::Diff;
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
                        } else if is_down(code) {
                            diff_scroll = diff_scroll.saturating_add(1);
                        } else if is_up(code) {
                            diff_scroll = diff_scroll.saturating_sub(1);
                        } else if is_back(code) {
                            view = View::Browse;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw(
    frame: &mut ratatui::Frame,
    commits: &[Commit],
    commit_sel: usize,
    commit_state: &mut ListState,
    files: &[FileEntry],
    file_state: &mut ListState,
    view: &View,
    diff: &Text<'static>,
    diff_scroll: u16,
    file_sel: usize,
) {
    let areas = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.area());

    let browsing = matches!(view, View::Browse);

    let commits_list = List::new(commits.iter().map(commit_item).collect::<Vec<_>>())
        .block(pane_block(
            format!(" commits  {}/{}   {MOVE} ", commit_sel + 1, commits.len()),
            browsing,
        ))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(commits_list, areas[0], commit_state);

    match view {
        View::Diff => {
            let path = files.get(file_sel).map(|f| f.path.as_str()).unwrap_or("");
            let view = Paragraph::new(diff.clone())
                .block(pane_block(
                    format!(" {path}   {MOVE}·ctrl-d/u scroll · esc back · q quit "),
                    true,
                ))
                .scroll((diff_scroll, 0));
            frame.render_widget(view, areas[1]);
        }
        _ => {
            let title = if files.is_empty() {
                " files  (none) ".to_string()
            } else {
                format!(
                    " files  {}/{}   {CTRL_MOVE} select · enter open · q quit ",
                    file_sel + 1,
                    files.len()
                )
            };
            let list = List::new(files.iter().map(file_item).collect::<Vec<_>>())
                .block(pane_block(title, browsing))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_stateful_widget(list, areas[1], file_state);
        }
    }
}
