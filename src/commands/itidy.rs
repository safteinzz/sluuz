//! `slu itidy` - interactive branch tidier (TUI) for the current repo.
//!
//! Lists local branches whose upstream is **gone** (the remote branch they
//! tracked was deleted - e.g. a merged PR that the remote auto-deleted), which
//! are the genuinely-finished branches worth cleaning up. It deliberately does
//! *not* list branches still alive on the remote (like a reserved `v0.2.21`) or
//! ones that never had an upstream.
//!
//! `p` prunes and reloads: git only marks a branch `[gone]` once the
//! remote-tracking ref is really absent, and a plain `git fetch` never removes
//! one, so a repo that does not prune hides the very branches this lists.
//!
//! `j`/`k` (or arrows) move; `Enter` opens a Yes/No confirm popup (default
//! **No**), where `←`/`→` (or `h`/`l`) toggle and `Enter` acts on the highlight:
//! Yes deletes (`git branch -D`, force, since these are done on the remote and
//! often aren't recognised as locally merged), No cancels. `y`/`n` are shortcuts;
//! `Esc` cancels the popup, or quits when none is open. `q` / `Ctrl-C` quit.
//!
//! For the non-interactive, multi-repo view, use `slu tidy`.

use crate::git::{SEP, first_line, git_capture, git_run, prunes_on_fetch};
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
use std::time::{Duration, Instant};

/// How long a success note sits in the footer before the key hints come back.
/// A failure is not given one: it stays until a keypress, because it is the
/// only place git's own words are shown.
const NOTE: Duration = Duration::from_secs(4);

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
    /// Last action's outcome, shown in the footer (ok?, message), and when it
    /// was put there.
    msg: Option<(bool, String)>,
    msg_at: Instant,
    /// Whether a fetch in this repo prunes on its own. When it does not, the
    /// list can be missing branches and the view says so.
    prunes: bool,
    /// A prune has happened in this session, so the list is current whatever
    /// the config says.
    pruned: bool,
    /// `p` was pressed: the frame that says so is drawn before git is run, or
    /// the screen would simply freeze for the length of a network round trip.
    pruning: bool,
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
        msg_at: Instant::now(),
        prunes: prunes_on_fetch("."),
        pruned: false,
        pruning: false,
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

            // Run the prune after the frame that announced it, never before.
            if self.pruning {
                self.pruning = false;
                self.prune();
                continue;
            }

            // A success note gives the footer back to the key hints on its own.
            // Waiting for a keypress to clear it would leave the one line that
            // says which keys exist covered by an answer already read.
            if let Some(left) = self.note_left()
                && !event::poll(left)?
            {
                self.msg = None;
                continue;
            }

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

    /// How long the footer note has left, or None when there is nothing on a
    /// timer: no note at all, or a failure, which waits for a keypress.
    fn note_left(&self) -> Option<Duration> {
        match &self.msg {
            Some((true, _)) => Some(NOTE.saturating_sub(self.msg_at.elapsed())),
            _ => None,
        }
    }

    /// Put a note in the footer, starting its clock.
    fn note(&mut self, ok: bool, text: String) {
        self.msg = Some((ok, text));
        self.msg_at = Instant::now();
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
        } else if code == KeyCode::Char('p') {
            self.pruning = true;
            self.note(true, "pruning…".to_string());
        }
        false
    }

    /// Drop the remote-tracking refs the remote no longer has and read the list
    /// again, which is the only way a branch whose upstream went away can show
    /// up here at all. `git remote prune` rather than `git fetch --prune`: the
    /// ref list is the whole question here, and the objects are `slu sync`'s job.
    fn prune(&mut self) {
        let remotes = git_capture(".", &["remote"]).unwrap_or_default();
        for remote in remotes.lines().filter(|r| !r.is_empty()) {
            let (ok, out) = git_run(".", &["remote", "prune", remote]);
            if !ok {
                self.note(false, first_line(&out));
                return;
            }
        }
        self.pruned = true;
        self.branches = load_gone();
        self.sel = 0;
        self.state.select((!self.branches.is_empty()).then_some(0));
        self.note(true, "pruned".to_string());
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

        if ok {
            self.branches.remove(self.sel);
            if self.sel >= self.branches.len() {
                self.sel = self.branches.len().saturating_sub(1);
            }
            self.state
                .select((!self.branches.is_empty()).then_some(self.sel));
            self.note(true, format!("deleted {target}"));
        } else {
            self.note(false, format!("{target}: {}", first_line(&out)));
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    // The staleness note only takes a row while it has something to say, and
    // once a prune has run in this session it has nothing.
    let note = u16::from(!app.prunes && !app.pruned);
    let areas = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(note),
        Constraint::Length(1),
    ])
    .split(frame.area());

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

    if note == 1 {
        frame.render_widget(stale_note(), areas[1]);
    }
    frame.render_widget(footer(&app.msg), areas[2]);

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
            format!(" {Y_MOVE} move · enter delete · p prune · q quit"),
            Style::default().fg(Color::DarkGray),
        )),
    };
    Paragraph::new(line)
}

/// Said out loud rather than assumed: without pruning, an empty list is not the
/// same as nothing to tidy.
fn stale_note() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        " refs are only as fresh as your last prune - `p` to prune now, or set `git config --global fetch.prune true`",
        Style::default().add_modifier(Modifier::DIM),
    )))
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
