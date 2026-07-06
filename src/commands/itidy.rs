//! `slu itidy` — interactive branch tidier (TUI) for the current repo.
//!
//! Lists local branches whose upstream is **gone** (the remote branch they
//! tracked was deleted — e.g. a merged PR that the remote auto-deleted), which
//! are the genuinely-finished branches worth cleaning up. It deliberately does
//! *not* list branches still alive on the remote (like a reserved `v0.2.21`) or
//! ones that never had an upstream.
//!
//! `j`/`k` (or arrows) move; `Enter` opens a Yes/No confirm popup (default
//! **No**), where `←`/`→` (or `h`/`l`) toggle and `Enter` acts on the highlight
//! — Yes deletes (`git branch -D`, force, since these are done on the remote and
//! often aren't recognised as locally merged), No cancels. `y`/`n` are shortcuts;
//! `Esc` cancels the popup, or quits when none is open. `q` / `Ctrl-C` quit.
//!
//! For the non-interactive, multi-repo view, use `slu tidy`.

use crate::git::{git_capture, git_run};
use crate::tui::{
    is_back, is_down, is_left, is_right, is_up, list_scrollbar, pane_block,
    pop_keyboard_enhancement, push_keyboard_enhancement, X_MOVE, Y_MOVE, SEP,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {}

/// A local branch whose upstream tracking branch has been deleted.
struct Gone {
    name: String,
    age: String,
    author: String,
}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu itidy needs an interactive terminal — use `slu tidy` for the scriptable view");
        return;
    }

    let branches = load_gone();

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, branches);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu itidy: {e}");
    }
}

fn event_loop(terminal: &mut DefaultTerminal, mut branches: Vec<Gone>) -> io::Result<()> {
    let mut sel = 0usize;
    let mut state = ListState::default();
    state.select(Some(0));
    // The delete-confirmation popup: None when closed, Some(yes) when open, where
    // `yes` is whether the Yes button (left) is highlighted. Opens on No.
    let mut confirm: Option<bool> = None;
    // Last action's outcome, shown in the footer (ok?, message).
    let mut msg: Option<(bool, String)> = None;

    loop {
        terminal.draw(|frame| draw(frame, &branches, sel, &mut state, &msg, confirm))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let code = key.code;

        // Ctrl-C always quits, even with the popup open.
        if ctrl && code == KeyCode::Char('c') {
            break;
        }

        if let Some(yes) = confirm {
            // Popup open. Yes is on the left, No on the right.
            if is_left(code) {
                confirm = Some(true);
            } else if is_right(code) {
                confirm = Some(false);
            } else if code == KeyCode::Char('y') {
                msg = delete(&mut branches, &mut sel, &mut state);
                confirm = None;
            } else if code == KeyCode::Enter {
                if yes {
                    msg = delete(&mut branches, &mut sel, &mut state);
                }
                confirm = None;
            } else if is_back(code) || matches!(code, KeyCode::Char('q' | 'n')) {
                confirm = None;
            }
            continue;
        }

        // No popup: navigate / open confirm / quit.
        if matches!(code, KeyCode::Char('q')) || is_back(code) {
            break;
        } else if is_down(code) && sel + 1 < branches.len() {
            sel += 1;
            state.select(Some(sel));
            msg = None;
        } else if is_up(code) && sel > 0 {
            sel -= 1;
            state.select(Some(sel));
            msg = None;
        } else if code == KeyCode::Enter && !branches.is_empty() {
            confirm = Some(false); // default to No
            msg = None;
        }
    }
    Ok(())
}

/// Force-delete the selected branch. On success drop it from the list and keep
/// the selection in range; either way return a footer message. Force (`-D`) is
/// deliberate: the user has explicitly confirmed, and gone-upstream branches
/// often aren't seen as locally merged (squash/rebase merges), so `-d` would
/// just refuse them.
fn delete(branches: &mut Vec<Gone>, sel: &mut usize, state: &mut ListState) -> Option<(bool, String)> {
    let target = branches.get(*sel)?.name.clone();
    let (ok, out) = git_run(".", &["branch", "-D", &target]);

    if ok {
        branches.remove(*sel);
        if *sel >= branches.len() {
            *sel = branches.len().saturating_sub(1);
        }
        state.select((!branches.is_empty()).then_some(*sel));
        Some((true, format!("deleted {target}")))
    } else {
        Some((false, format!("{target}: {}", first_line(&out))))
    }
}

fn draw(
    frame: &mut ratatui::Frame,
    branches: &[Gone],
    sel: usize,
    state: &mut ListState,
    msg: &Option<(bool, String)>,
    confirm: Option<bool>,
) {
    let areas =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(frame.area());

    let title = if branches.is_empty() {
        " no branches with a deleted upstream — nothing to tidy ".to_string()
    } else {
        format!(" gone branches  {}/{} ", sel + 1, branches.len())
    };
    let list = List::new(branches.iter().map(gone_item).collect::<Vec<_>>())
        .block(pane_block(title, true))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[0], state);
    list_scrollbar(frame, areas[0], branches.len(), state.offset());

    frame.render_widget(footer(msg), areas[1]);

    if let Some(yes) = confirm {
        if let Some(b) = branches.get(sel) {
            confirm_popup(frame, &b.name, yes);
        }
    }
}

/// Centered confirmation dialog over the list. `yes` = the Yes button (left) is
/// highlighted; otherwise No (right) is.
fn confirm_popup(frame: &mut ratatui::Frame, name: &str, yes: bool) {
    let area = popup_area(frame.area());
    frame.render_widget(Clear, area); // wipe whatever's underneath

    let picked = Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    let idle = Style::default().fg(Color::DarkGray);
    let buttons = Line::from(vec![
        Span::styled("  Yes  ", if yes { picked } else { idle }),
        Span::raw("     "),
        Span::styled("  No  ", if yes { idle } else { picked }),
    ]);

    let body = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            truncate(name, area.width.saturating_sub(4) as usize),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        buttons,
        Line::from(""),
        Line::from(Span::styled(
            format!("{X_MOVE} toggle · enter select · y/n"),
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" delete this branch? "),
    );
    frame.render_widget(body, area);
}

/// A small box centered in `area`.
fn popup_area(area: Rect) -> Rect {
    let w = area.width.saturating_sub(4).clamp(24, 54);
    let h = 9u16.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// Bottom line: the last action's result if any, otherwise the key hints.
fn footer(msg: &Option<(bool, String)>) -> Paragraph<'static> {
    let line = match msg {
        Some((true, m)) => Line::from(Span::styled(
            format!(" ✓ {m}"),
            Style::default().fg(Color::Green),
        )),
        Some((false, m)) => Line::from(Span::styled(
            format!(" ✗ {m}"),
            Style::default().fg(Color::Red),
        )),
        None => Line::from(Span::styled(
            format!(" {Y_MOVE} move · enter delete · q quit"),
            Style::default().fg(Color::DarkGray),
        )),
    };
    Paragraph::new(line)
}

fn gone_item(b: &Gone) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{:<32}", truncate(&b.name, 32)),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("[gone]", Style::default().fg(Color::Red).add_modifier(Modifier::DIM)),
        Span::styled(format!("  {:<16}", b.age), Style::default().fg(Color::Magenta)),
        Span::styled(format!("  {}", b.author), Style::default().fg(Color::Blue)),
    ]))
}

/// Local branches whose upstream is gone, newest-activity first. `%(refname)` is
/// stripped to the plain branch name ourselves — `%(refname:short)` would return
/// `heads/v0.2.20` when a tag of the same name exists, which `git branch -D`
/// can't take.
fn load_gone() -> Vec<Gone> {
    let fmt = format!(
        "--format=%(refname){SEP}%(upstream:track){SEP}%(committerdate:relative){SEP}%(authorname)"
    );
    git_capture(".", &["for-each-ref", "--sort=-committerdate", &fmt, "refs/heads"])
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut f = line.split(SEP);
                    let refname = f.next()?;
                    let track = f.next()?;
                    // `[gone]` marks a tracking branch whose remote was deleted.
                    if !track.contains("gone") {
                        return None;
                    }
                    Some(Gone {
                        name: refname.strip_prefix("refs/heads/").unwrap_or(refname).to_string(),
                        age: f.next().unwrap_or("").to_string(),
                        author: f.next().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// First non-empty line of git output, for a compact one-line message.
fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
