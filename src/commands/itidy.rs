//! `slu itidy` - interactive branch tidier (TUI) for the current repo.
//!
//! Lists local branches whose upstream is **gone** (the remote branch they
//! tracked was deleted - e.g. a merged PR that the remote auto-deleted), which
//! are the genuinely-finished branches worth cleaning up. It deliberately does
//! *not* list branches still alive on the remote (like a reserved `v0.2.21`) or
//! ones that never had an upstream.
//!
//! `j`/`k` (or arrows) move; `Enter` opens a Yes/No confirm popup (default
//! **No**), where `←`/`→` (or `h`/`l`) toggle and `Enter` acts on the highlight:
//! Yes deletes (`git branch -D`, force, since these are done on the remote and
//! often aren't recognised as locally merged), No cancels. `y`/`n` are shortcuts;
//! `Esc` cancels the popup, or quits when none is open. `q` / `Ctrl-C` quit.
//!
//! For the non-interactive, multi-repo view, use `slu tidy`.

use crate::git::{SEP, git_capture, git_run};
use crate::tui::input::{X_MOVE, Y_MOVE, is_back, is_down, is_left, is_right, is_up, norm_esc};
use crate::tui::widgets::{
    box_block, box_buttons, box_height, box_hint, box_inner_width, box_width, list_scrollbar,
    pane_block, popup_area,
};
use crate::tui::{pop_keyboard_enhancement, push_keyboard_enhancement};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};
use std::io::{self, IsTerminal};

#[derive(clap::Args)]
pub struct Args {}

/// A local branch whose upstream tracking branch has been deleted.
struct Gone {
    name: String,
    age: String,
    author: String,
}

/// Everything the view holds: the branches it found, where the cursor is, and
/// whether the confirm popup is up.
struct App {
    branches: Vec<Gone>,
    sel: usize,
    state: ListState,
    /// The delete-confirmation popup: None when closed, Some(yes) when open,
    /// where `yes` is whether the Yes button (left) is highlighted.
    confirm: Option<bool>,
    /// Last action's outcome, shown in the footer (ok?, message).
    msg: Option<(bool, String)>,
}

pub fn run(_args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!(
            "slu itidy needs an interactive terminal - use `slu tidy` for the scriptable view"
        );
        return;
    }

    let mut state = ListState::default();
    state.select(Some(0));
    let mut app = App {
        branches: load_gone(),
        sel: 0,
        state,
        confirm: None,
        msg: None,
    };

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = app.event_loop(&mut terminal);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu itidy: {e}");
    }
}

impl App {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|frame| draw(frame, self))?;

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let code = norm_esc(key.code, ctrl);

            // Ctrl-C always quits, even with the popup open.
            if ctrl && code == KeyCode::Char('c') {
                break;
            }
            if self.on_key(code) {
                break;
            }
        }
        Ok(())
    }

    /// Handle one key. Returns true when the app should quit.
    fn on_key(&mut self, code: KeyCode) -> bool {
        if let Some(yes) = self.confirm {
            // Popup open. Yes is on the left, No on the right.
            if is_left(code) {
                self.confirm = Some(true);
            } else if is_right(code) {
                self.confirm = Some(false);
            } else if code == KeyCode::Char('y') {
                self.delete();
                self.confirm = None;
            } else if code == KeyCode::Enter {
                if yes {
                    self.delete();
                }
                self.confirm = None;
            } else if is_back(code) || matches!(code, KeyCode::Char('q' | 'n')) {
                self.confirm = None;
            }
            return false;
        }

        // No popup: navigate / open confirm / quit.
        if matches!(code, KeyCode::Char('q')) || is_back(code) {
            return true;
        } else if is_down(code) && self.sel + 1 < self.branches.len() {
            self.sel += 1;
            self.state.select(Some(self.sel));
            self.msg = None;
        } else if is_up(code) && self.sel > 0 {
            self.sel -= 1;
            self.state.select(Some(self.sel));
            self.msg = None;
        } else if code == KeyCode::Enter && !self.branches.is_empty() {
            self.confirm = Some(false); // default to No
            self.msg = None;
        }
        false
    }

    /// Force-delete the selected branch. On success drop it from the list and
    /// keep the selection in range; either way leave a footer message. Force
    /// (`-D`) is deliberate: the user has explicitly confirmed, and
    /// gone-upstream branches often aren't seen as locally merged (squash and
    /// rebase merges), so `-d` would just refuse them.
    fn delete(&mut self) {
        let Some(target) = self.branches.get(self.sel).map(|b| b.name.clone()) else {
            return;
        };
        let (ok, out) = git_run(".", &["branch", "-D", &target]);

        self.msg = if ok {
            self.branches.remove(self.sel);
            if self.sel >= self.branches.len() {
                self.sel = self.branches.len().saturating_sub(1);
            }
            self.state
                .select((!self.branches.is_empty()).then_some(self.sel));
            Some((true, format!("deleted {target}")))
        } else {
            Some((false, format!("{target}: {}", first_line(&out))))
        };
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let areas = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(frame.area());

    let title = if app.branches.is_empty() {
        " no branches with a deleted upstream - nothing to tidy ".to_string()
    } else {
        format!(" gone branches  {}/{} ", app.sel + 1, app.branches.len())
    };
    let list = List::new(app.branches.iter().map(gone_item).collect::<Vec<_>>())
        .block(pane_block(title, true))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[0], &mut app.state);
    list_scrollbar(frame, areas[0], app.branches.len(), app.state.offset());

    frame.render_widget(footer(&app.msg), areas[1]);

    if let Some(yes) = app.confirm
        && let Some(b) = app.branches.get(app.sel)
    {
        confirm_popup(frame, &b.name, yes);
    }
}

/// The gate in front of deleting a branch: red, and it starts on No, because a
/// reflex Enter must never be the key that fires an irreversible thing.
fn confirm_popup(frame: &mut ratatui::Frame, name: &str, yes: bool) {
    let full = frame.area();
    let width = box_width(full.width);
    let lines = vec![
        Line::from(Span::styled(
            truncate(name, box_inner_width(width)),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        box_buttons(Color::Red, yes),
        Line::from(""),
        box_hint(&format!("{X_MOVE} move · enter select · y/n")),
    ];
    // Measured rather than fixed: a long branch name wraps, and a box that was
    // always nine rows tall would put the buttons past its own bottom border.
    let rows = lines.len() as u16;
    let area = popup_area(full, width, box_height(rows, full.height));

    frame.render_widget(Clear, area); // wipe whatever's underneath
    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(box_block(Color::Red, "delete this branch?"));
    frame.render_widget(body, area);
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
        Span::styled(
            "[gone]",
            Style::default().fg(Color::Red).add_modifier(Modifier::DIM),
        ),
        Span::styled(
            format!("  {:<16}", b.age),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(format!("  {}", b.author), Style::default().fg(Color::Blue)),
    ]))
}

/// Local branches whose upstream is gone, newest-activity first. `%(refname)` is
/// stripped to the plain branch name ourselves - `%(refname:short)` would return
/// `heads/v0.2.20` when a tag of the same name exists, which `git branch -D`
/// can't take.
fn load_gone() -> Vec<Gone> {
    let fmt = format!(
        "--format=%(refname){SEP}%(upstream:track){SEP}%(committerdate:relative){SEP}%(authorname)"
    );
    git_capture(
        ".",
        &["for-each-ref", "--sort=-committerdate", &fmt, "refs/heads"],
    )
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
                    name: refname
                        .strip_prefix("refs/heads/")
                        .unwrap_or(refname)
                        .to_string(),
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
