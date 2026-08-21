//! `slu iscan` - interactive history search (TUI). The query lives in the tool,
//! not in flags: you type terms into the bar and it pickaxes them across every
//! branch of every repo under you.
//!
//! Three panes: the query bar, the hits it found, and the full diff of the commit
//! behind the selected hit, so "what actually changed there?" is one keypress
//! away. `Enter` on a hit opens the same comparison in the user's `git difftool`.
//!
//! The bar starts empty and ready to type, showing the usual secret terms as a
//! placeholder: submitting an empty bar runs those, so the classic audit is one
//! keypress without any of it being in your way. Multiple terms are
//! comma-separated, and `h`/`l` then filters the results down to one.
//!
//! Scanning is slow (pickaxe over every commit of every branch of every repo),
//! so it happens on submit, with the screen showing that it is working.

use crate::git::{display_name, find_repos};
use crate::history::{self, CommitMatch};
use crate::tui::{
    clamp_hscroll, clamp_scroll, half_page, pane_height, pane_width, pop_keyboard_enhancement,
    push_keyboard_enhancement,
};
use crate::tui::difftool::difftool_commit;
use crate::tui::highlight::{prepare_diff, render_prepared, RenderedDiff};
use crate::tui::input::{
    is_back, is_down, is_left, is_open, is_right, is_up, norm_esc, CTRL_X_MOVE, CTRL_Y_MOVE,
    X_MOVE, Y_MOVE,
};
use crate::git::load::load_diff_raw;
use crate::tui::widgets::{diff_hscrollbar, diff_scrollbar, list_scrollbar, pane_block};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

/// Rows a Ctrl-j/k moves the diff, columns a Ctrl-h/l pans it, and the
/// PgUp/PgDn jump.
const SCROLL_STEP: u16 = 3;
const PAN_STEP: u16 = 8;
const PAGE_STEP: u16 = 10;

/// Shown as the bar's placeholder, and used when the bar is submitted empty:
/// the terms a secret audit usually wants.
const DEFAULT_TERMS: &str = "password,secret,token,api_key,passwd,credentials";

/// Shown for a file pickaxe flagged but whose diff has no visible +/- lines
/// (binary or encrypted blobs), mirroring what `slu scan` prints.
const BINARY_HIT: &str = "(binary or no visible diff)";

#[derive(clap::Args)]
pub struct Args {
    /// Base directory to search for repos (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// How many directory levels deep to look for repos
    #[arg(short, long, default_value_t = 3)]
    pub depth: usize,
}

/// One finding: a single matched line in a file of a commit in a repo. Same
/// granularity `slu scan` counts, so the totals agree.
struct Hit {
    repo: String,
    repo_path: String,
    short: String,
    full: String,
    date: String,
    subject: String,
    file: String,
    line: String,
    is_add: bool,
    /// Indices into the submitted terms that this line contains. Empty means
    /// "unknown", which happens for binary hits; those show in every scope.
    matched: Vec<usize>,
}

impl Hit {
    fn in_scope(&self, scope_idx: usize) -> bool {
        // scope 0 is "all"; scope n is terms[n - 1].
        scope_idx == 0 || self.matched.is_empty() || self.matched.contains(&(scope_idx - 1))
    }
}

/// Typing in the bar, or moving through what it found.
#[derive(PartialEq)]
enum Mode {
    Editing,
    Browsing,
}

/// Everything the view holds: the bar being typed, what the last scan found,
/// and where the cursor sits in it.
struct App {
    base: PathBuf,
    depth: usize,
    enhanced: bool,
    width: u16,
    mode: Mode,
    query: String,
    cursor: usize,
    terms: Vec<String>,
    hits: Vec<Hit>,
    /// Indices into `hits` that the current term scope keeps.
    visible: Vec<usize>,
    sel: usize,
    scope_idx: usize,
    /// Has a scan run yet?
    searched: bool,
    /// A scan is queued for right after this frame.
    pending: bool,
    state: ListState,
    prepared: RenderedDiff,
    diff: Text<'static>,
    diff_scroll: u16,
    diff_hscroll: u16,
    msg: Option<String>,
}

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu iscan needs an interactive terminal - use `slu scan` for plain output");
        return;
    }

    let mut app = App {
        base: args.path,
        depth: args.depth,
        enhanced: false,
        width: 120,
        mode: Mode::Editing,
        query: String::new(),
        cursor: 0,
        terms: Vec::new(),
        hits: Vec::new(),
        visible: Vec::new(),
        sel: 0,
        scope_idx: 0,
        searched: false,
        pending: false,
        state: ListState::default(),
        prepared: RenderedDiff::default(),
        diff: Text::default(),
        diff_scroll: 0,
        diff_hscroll: 0,
        msg: None,
    };

    let mut terminal = ratatui::init();
    app.enhanced = push_keyboard_enhancement();
    app.width = pane_width(&terminal);
    let result = app.event_loop(&mut terminal);
    if app.enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu iscan: {e}");
    }
}

impl App {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|frame| draw(frame, self))?;

            // The scan blocks, so it only runs once the frame above has told
            // the user it is working.
            if self.pending {
                self.scan();
                continue;
            }

            let Event::Key(key) = event::read()? else {
                let w = pane_width(terminal);
                if w != self.width {
                    self.width = w;
                    self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
                }
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let code = norm_esc(key.code, ctrl);
            self.msg = None;

            if ctrl && code == KeyCode::Char('c') {
                break;
            }

            let quit = if self.mode == Mode::Editing {
                self.edit_key(code, ctrl)
            } else {
                self.browse_key(code, ctrl, terminal)
            };
            if quit {
                break;
            }
            if self.mode == Mode::Browsing {
                self.diff_scroll =
                    clamp_scroll(self.diff_scroll, self.diff.lines.len(), pane_height(terminal));
            }
        }
        Ok(())
    }

    /// Pickaxe every repo under `base` for what the bar says, then show what it
    /// found. An empty bar means the usual secret terms, matching the
    /// placeholder.
    fn scan(&mut self) {
        self.terms = parse_terms(if self.query.trim().is_empty() {
            DEFAULT_TERMS
        } else {
            &self.query
        });
        self.hits = collect(&self.base, self.depth, &self.terms);
        self.scope_idx = 0;
        self.sel = 0;
        self.rescope();
        self.searched = true;
        self.pending = false;
        self.mode = Mode::Browsing;
    }

    /// Re-filter for the current term scope, keep the cursor in range, and
    /// refresh the diff under it.
    fn rescope(&mut self) {
        self.visible = self
            .hits
            .iter()
            .enumerate()
            .filter(|(_, h)| h.in_scope(self.scope_idx))
            .map(|(i, _)| i)
            .collect();
        if self.sel >= self.visible.len() {
            self.sel = self.visible.len().saturating_sub(1);
        }
        self.state
            .select((!self.visible.is_empty()).then_some(self.sel));
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.refresh_diff();
    }

    /// Re-highlight the diff for the selected hit (the expensive syntect pass,
    /// done once per selection change rather than per keypress).
    fn refresh_diff(&mut self) {
        match self.visible.get(self.sel).map(|&i| &self.hits[i]) {
            Some(h) => {
                let raw = load_diff_raw(&h.repo_path, &h.full, &h.file);
                self.prepared = prepare_diff(&raw);
                self.diff = render_prepared(&self.prepared, self.width, 0);
            }
            None => {
                self.prepared = RenderedDiff::default();
                self.diff = Text::default();
            }
        }
    }

    /// The query bar has focus: a plain text field until Enter runs it.
    fn edit_key(&mut self, code: KeyCode, ctrl: bool) -> bool {
        match code {
            KeyCode::Enter => self.pending = true,
            KeyCode::Char(c) if !ctrl => {
                self.query.insert(char_to_byte(&self.query, self.cursor), c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.query.remove(char_to_byte(&self.query, self.cursor - 1));
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < self.query.chars().count() => {
                self.query.remove(char_to_byte(&self.query, self.cursor));
            }
            KeyCode::Left if self.cursor > 0 => self.cursor -= 1,
            KeyCode::Right if self.cursor < self.query.chars().count() => self.cursor += 1,
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.query.chars().count(),
            // Esc leaves the bar only when there are results to go back to.
            KeyCode::Esc if self.searched => self.mode = Mode::Browsing,
            KeyCode::Esc => return true,
            _ => {}
        }
        false
    }

    /// Moving through what the scan found, with the hit's diff below it.
    fn browse_key(&mut self, code: KeyCode, ctrl: bool, terminal: &mut DefaultTerminal) -> bool {
        let half = half_page(terminal);
        let mut moved = false;

        if matches!(code, KeyCode::Char('q')) || is_back(code) {
            return true;
        } else if matches!(code, KeyCode::Char('/') | KeyCode::Char('i')) {
            self.mode = Mode::Editing;
        } else if ctrl && is_down(code) {
            self.diff_scroll = self.diff_scroll.saturating_add(SCROLL_STEP);
        } else if ctrl && is_up(code) {
            self.diff_scroll = self.diff_scroll.saturating_sub(SCROLL_STEP);
        } else if ctrl && code == KeyCode::Char('d') {
            self.diff_scroll = self.diff_scroll.saturating_add(half);
        } else if ctrl && code == KeyCode::Char('u') {
            self.diff_scroll = self.diff_scroll.saturating_sub(half);
        } else if ctrl && is_right(code) {
            self.diff_hscroll = clamp_hscroll(
                self.diff_hscroll.saturating_add(PAN_STEP),
                self.prepared.max_line(),
                self.prepared.cell_width(self.width),
            );
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
        } else if ctrl && is_left(code) {
            self.diff_hscroll = self.diff_hscroll.saturating_sub(PAN_STEP);
            self.diff = render_prepared(&self.prepared, self.width, self.diff_hscroll);
        } else if code == KeyCode::PageDown {
            self.diff_scroll = self.diff_scroll.saturating_add(PAGE_STEP);
        } else if code == KeyCode::PageUp {
            self.diff_scroll = self.diff_scroll.saturating_sub(PAGE_STEP);
        } else if is_down(code) && self.sel + 1 < self.visible.len() {
            self.sel += 1;
            moved = true;
        } else if is_up(code) && self.sel > 0 {
            self.sel -= 1;
            moved = true;
        } else if !ctrl && is_left(code) && self.scope_idx > 0 {
            self.scope_idx -= 1;
            self.sel = 0;
            moved = true;
        } else if !ctrl && is_right(code) && self.scope_idx + 1 < self.terms.len() + 1 {
            self.scope_idx += 1;
            self.sel = 0;
            moved = true;
        } else if is_open(code)
            && let Some(h) = self.visible.get(self.sel).map(|&i| &self.hits[i])
        {
            let (repo, full, file) = (h.repo_path.clone(), h.full.clone(), h.file.clone());
            let m = difftool_commit(terminal, self.enhanced, &repo, &full, &file);
            self.width = pane_width(terminal);
            if !m.is_empty() {
                self.msg = Some(m);
            }
        }

        if moved {
            self.rescope();
        }
        false
    }
}

/// Byte offset of character index `idx`, so inserts and deletes stay safe on
/// multi-byte input.
fn char_to_byte(s: &str, idx: usize) -> usize {
    s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len())
}

/// Split the bar's text into terms: comma-separated, trimmed, empties dropped.
fn parse_terms(query: &str) -> Vec<String> {
    query
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Pickaxe every repo under `base` for `terms`, flattened to one hit per matched
/// line.
fn collect(base: &Path, depth: usize, terms: &[String]) -> Vec<Hit> {
    let mut hits = Vec::new();
    for repo in find_repos(base, depth) {
        let name = display_name(&repo);
        let Some(repo_str) = repo.to_str() else {
            continue;
        };
        for commit in history::pickaxe(repo_str, terms, true) {
            push_hits(&mut hits, &name, repo_str, &commit, terms);
        }
    }
    hits
}

/// Flatten one matching commit into its individual hits.
fn push_hits(out: &mut Vec<Hit>, repo: &str, repo_path: &str, c: &CommitMatch, terms: &[String]) {
    for file in &c.files {
        let mk = |line: String, is_add: bool, matched: Vec<usize>| Hit {
            repo: repo.to_string(),
            repo_path: repo_path.to_string(),
            short: c.short.clone(),
            full: c.full.clone(),
            date: c.date.clone(),
            subject: c.subject.clone(),
            file: file.path.clone(),
            line,
            is_add,
            matched,
        };
        if file.lines.is_empty() {
            out.push(mk(BINARY_HIT.to_string(), false, Vec::new()));
            continue;
        }
        for (is_add, line) in &file.lines {
            let matched = terms_in(line, terms);
            out.push(mk(line.trim().to_string(), *is_add, matched));
        }
    }
}

/// Indices of the terms contained in `line`, case-insensitively. Pickaxe merges
/// per-term results by commit, so which term matched is recovered here.
fn terms_in(line: &str, terms: &[String]) -> Vec<usize> {
    let hay = line.to_lowercase();
    terms
        .iter()
        .enumerate()
        .filter(|(_, t)| hay.contains(&t.to_lowercase()))
        .map(|(i, _)| i)
        .collect()
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(42),
        Constraint::Min(5),
    ])
    .split(frame.area());

    // ── query bar ──
    let editing = app.mode == Mode::Editing;
    let label = "scan terms (comma separated)";
    let bar_hint = if app.pending {
        format!(" {label} · working… ")
    } else if editing {
        format!(" {label} · enter run · esc back ")
    } else {
        format!(" {label} · / edit ")
    };
    // Empty bar shows the defaults dimmed; submitting empty runs exactly those.
    let bar_line = if app.query.is_empty() {
        Line::from(Span::styled(
            format!(" {DEFAULT_TERMS}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
    } else {
        Line::from(Span::styled(
            format!(" {}", app.query),
            Style::default().fg(Color::White),
        ))
    };
    let bar = Paragraph::new(bar_line).block(pane_block(bar_hint, editing));
    frame.render_widget(bar, areas[0]);
    if editing && !app.pending {
        // A real terminal cursor, so it blinks where the user is typing.
        frame.set_cursor_position(Position::new(
            areas[0].x + 2 + app.cursor as u16,
            areas[0].y + 1,
        ));
    }

    // ── hits ──
    let items: Vec<ListItem> = app.visible.iter().map(|&i| hit_item(&app.hits[i])).collect();
    let scope = if app.scope_idx == 0 || app.terms.is_empty() {
        "all terms".to_string()
    } else {
        app.terms[app.scope_idx - 1].clone()
    };
    let title = if app.pending {
        " searching every branch of every repo… ".to_string()
    } else if !app.searched {
        " type terms above, or press enter for the defaults ".to_string()
    } else if app.visible.is_empty() {
        format!(" no hits · {scope} ·  / edit · q quit ")
    } else {
        format!(
            " hits · {scope}  {}/{} of {}   {Y_MOVE} · {X_MOVE} term · / edit · q quit ",
            app.sel + 1,
            app.visible.len(),
            app.hits.len()
        )
    };
    let list = List::new(items)
        .block(pane_block(title, !editing))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[1], &mut app.state);
    list_scrollbar(frame, areas[1], app.visible.len(), app.state.offset());

    // ── diff ──
    let subtitle = match app.visible.get(app.sel).map(|&i| &app.hits[i]) {
        Some(h) => format!("{}  {}  {}", h.short, h.subject, h.file),
        None => String::new(),
    };
    let dtitle = match &app.msg {
        Some(m) => format!(" ⚠ {m} "),
        None if subtitle.is_empty() => " (nothing to show) ".to_string(),
        None => {
            format!(" {subtitle}   enter difftool · {CTRL_Y_MOVE} scroll · {CTRL_X_MOVE} pan ")
        }
    };
    let diff = Paragraph::new(app.diff.clone())
        .block(pane_block(dtitle, !editing))
        .scroll((app.diff_scroll, 0));
    frame.render_widget(diff, areas[2]);
    diff_scrollbar(frame, areas[2], app.diff.lines.len(), app.diff_scroll);
    let cell = app.prepared.cell_width(areas[2].width.saturating_sub(2));
    diff_hscrollbar(frame, areas[2], app.prepared.max_line(), cell, app.diff_hscroll);
}

fn hit_item(h: &Hit) -> ListItem<'static> {
    let line_style = if h.line == BINARY_HIT {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else if h.is_add {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{:<18}", truncate(&h.repo, 18)),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(format!("{:<9}", h.short), Style::default().fg(Color::Yellow)),
        Span::styled(format!("{}  ", h.date), Style::default().fg(Color::Magenta)),
        Span::styled(
            format!("{:<26}", truncate(&h.file, 26)),
            Style::default().fg(Color::Blue),
        ),
        Span::styled(truncate(&h.line, 80), line_style),
    ]))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::{parse_terms, terms_in, Hit};

    fn hit(matched: Vec<usize>) -> Hit {
        Hit {
            repo: "r".into(),
            repo_path: "/r".into(),
            short: "abc1234".into(),
            full: "abc1234full".into(),
            date: "2026-01-01".into(),
            subject: "s".into(),
            file: "f".into(),
            line: "l".into(),
            is_add: true,
            matched,
        }
    }

    #[test]
    fn terms_match_case_insensitively() {
        let terms = vec!["password".to_string(), "token".to_string()];
        assert_eq!(terms_in("+DB_PASSWORD = \"x\"", &terms), vec![0]);
        assert_eq!(terms_in("+api_TOKEN=1", &terms), vec![1]);
    }

    #[test]
    fn a_line_can_match_several_terms() {
        let terms = vec!["password".to_string(), "token".to_string()];
        assert_eq!(terms_in("password and token", &terms), vec![0, 1]);
    }

    #[test]
    fn no_match_is_empty() {
        let terms = vec!["password".to_string()];
        assert!(terms_in("nothing here", &terms).is_empty());
    }

    #[test]
    fn the_bar_splits_on_commas_and_trims() {
        assert_eq!(parse_terms(" password , token "), vec!["password", "token"]);
        assert_eq!(parse_terms("one"), vec!["one"]);
        assert!(parse_terms("  ,  ").is_empty());
    }

    #[test]
    fn a_term_may_contain_spaces() {
        // Only commas split, so a phrase stays one term.
        assert_eq!(
            parse_terms("BEGIN RSA PRIVATE KEY"),
            vec!["BEGIN RSA PRIVATE KEY"]
        );
    }

    #[test]
    fn scope_zero_shows_everything() {
        assert!(hit(vec![1]).in_scope(0));
        assert!(hit(vec![]).in_scope(0));
    }

    #[test]
    fn term_scope_filters_to_that_term() {
        let h = hit(vec![1]); // matched terms[1]
        assert!(h.in_scope(2), "scope 2 is terms[1], should show");
        assert!(!h.in_scope(1), "scope 1 is terms[0], should hide");
    }

    #[test]
    fn binary_hits_show_in_every_scope() {
        // Pickaxe flagged the file but showed no lines, so which term matched is
        // unknown; hiding it in a term scope could hide a real secret.
        let h = hit(vec![]);
        assert!(h.in_scope(1) && h.in_scope(2));
    }
}
