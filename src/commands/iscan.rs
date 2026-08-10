//! `slu iscan` — interactive history search (TUI). The query lives in the tool,
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
    clamp_hscroll, clamp_scroll, diff_hscrollbar, diff_scrollbar, difftool_commit, half_page,
    is_back, is_down, is_left, is_open, is_right, is_up, list_scrollbar, load_diff_raw_in, norm_esc,
    pane_block, pane_height, pane_width, pop_keyboard_enhancement, prepare_diff,
    push_keyboard_enhancement, render_prepared, RenderedDiff, CTRL_X_MOVE, CTRL_Y_MOVE, X_MOVE,
    Y_MOVE,
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

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

pub fn run(args: Args) {
    if !io::stdout().is_terminal() {
        eprintln!("slu iscan needs an interactive terminal — use `slu scan` for plain output");
        return;
    }

    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &args.path, args.depth, enhanced);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();

    if let Err(e) = result {
        eprintln!("slu iscan: {e}");
    }
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    base: &Path,
    depth: usize,
    enhanced: bool,
) -> io::Result<()> {
    let mut width = pane_width(terminal);
    let mut mode = Mode::Editing;
    let mut query = String::new();
    let mut cursor = 0usize;

    let mut terms: Vec<String> = Vec::new();
    let mut hits: Vec<Hit> = Vec::new();
    let mut visible: Vec<usize> = Vec::new();
    let mut sel = 0usize;
    let mut scope_idx = 0usize;
    let mut searched = false; // has a scan run yet?
    let mut pending = false; // a scan is queued for right after this frame

    let mut prepared = RenderedDiff::default();
    let mut diff = Text::default();
    let mut diff_scroll = 0u16;
    let mut diff_hscroll = 0u16;
    let mut msg: Option<String> = None;

    let mut state = ListState::default();

    loop {
        terminal.draw(|frame| {
            draw(
                frame,
                View {
                    query: &query,
                    cursor,
                    mode: &mode,
                    scanning: pending,
                    searched,
                    terms: &terms,
                    scope_idx,
                    hits: &hits,
                    visible: &visible,
                    sel,
                    diff: &diff,
                    diff_scroll,
                    prepared: &prepared,
                    diff_hscroll,
                    msg: msg.as_deref(),
                },
                &mut state,
            )
        })?;

        // The scan blocks, so it only runs once the frame above has told the
        // user it is working.
        if pending {
            // An empty bar means "the usual secret terms", matching the placeholder.
            terms = parse_terms(if query.trim().is_empty() {
                DEFAULT_TERMS
            } else {
                &query
            });
            hits = collect(base, depth, &terms);
            scope_idx = 0;
            visible = visible_hits(&hits, scope_idx);
            sel = 0;
            state.select((!visible.is_empty()).then_some(0));
            diff_scroll = 0;
            diff_hscroll = 0;
            refresh(&hits, &visible, sel, width, &mut prepared, &mut diff);
            searched = true;
            pending = false;
            mode = Mode::Browsing;
            continue;
        }

        let Event::Key(key) = event::read()? else {
            let w = pane_width(terminal);
            if w != width {
                width = w;
                diff = render_prepared(&prepared, width, diff_hscroll);
            }
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let code = norm_esc(key.code, ctrl);
        msg = None;

        if ctrl && code == KeyCode::Char('c') {
            break;
        }

        if mode == Mode::Editing {
            match code {
                KeyCode::Enter => pending = true,
                KeyCode::Char(c) if !ctrl => {
                    query.insert(char_to_byte(&query, cursor), c);
                    cursor += 1;
                }
                KeyCode::Backspace if cursor > 0 => {
                    query.remove(char_to_byte(&query, cursor - 1));
                    cursor -= 1;
                }
                KeyCode::Delete if cursor < query.chars().count() => {
                    query.remove(char_to_byte(&query, cursor));
                }
                KeyCode::Left if cursor > 0 => cursor -= 1,
                KeyCode::Right if cursor < query.chars().count() => cursor += 1,
                KeyCode::Home => cursor = 0,
                KeyCode::End => cursor = query.chars().count(),
                // Esc leaves the bar only when there are results to go back to.
                KeyCode::Esc if searched => mode = Mode::Browsing,
                KeyCode::Esc => break,
                _ => {}
            }
            continue;
        }

        // ── Browsing ────────────────────────────────────────────────────────
        let half = half_page(terminal);
        let mut moved = false;

        if matches!(code, KeyCode::Char('q')) || is_back(code) {
            break;
        } else if matches!(code, KeyCode::Char('/') | KeyCode::Char('i')) {
            mode = Mode::Editing;
        } else if ctrl && is_down(code) {
            diff_scroll = diff_scroll.saturating_add(3);
        } else if ctrl && is_up(code) {
            diff_scroll = diff_scroll.saturating_sub(3);
        } else if ctrl && code == KeyCode::Char('d') {
            diff_scroll = diff_scroll.saturating_add(half);
        } else if ctrl && code == KeyCode::Char('u') {
            diff_scroll = diff_scroll.saturating_sub(half);
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
        } else if code == KeyCode::PageDown {
            diff_scroll = diff_scroll.saturating_add(10);
        } else if code == KeyCode::PageUp {
            diff_scroll = diff_scroll.saturating_sub(10);
        } else if is_down(code) && sel + 1 < visible.len() {
            sel += 1;
            moved = true;
        } else if is_up(code) && sel > 0 {
            sel -= 1;
            moved = true;
        } else if !ctrl && is_left(code) && scope_idx > 0 {
            scope_idx -= 1;
            visible = visible_hits(&hits, scope_idx);
            sel = 0;
            moved = true;
        } else if !ctrl && is_right(code) && scope_idx + 1 < terms.len() + 1 {
            scope_idx += 1;
            visible = visible_hits(&hits, scope_idx);
            sel = 0;
            moved = true;
        } else if is_open(code)
            && let Some(h) = visible.get(sel).map(|&i| &hits[i])
        {
            let m = difftool_commit(terminal, enhanced, &h.repo_path, &h.full, &h.file);
            width = pane_width(terminal);
            if !m.is_empty() {
                msg = Some(m);
            }
        }

        if moved {
            if sel >= visible.len() {
                sel = visible.len().saturating_sub(1);
            }
            state.select((!visible.is_empty()).then_some(sel));
            diff_scroll = 0;
            diff_hscroll = 0;
            refresh(&hits, &visible, sel, width, &mut prepared, &mut diff);
        }
        diff_scroll = clamp_scroll(diff_scroll, diff.lines.len(), pane_height(terminal));
    }
    Ok(())
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

fn visible_hits(hits: &[Hit], scope_idx: usize) -> Vec<usize> {
    hits.iter()
        .enumerate()
        .filter(|(_, h)| h.in_scope(scope_idx))
        .map(|(i, _)| i)
        .collect()
}

/// Re-highlight the diff for the selected hit (the expensive syntect pass, done
/// once per selection change rather than per keypress).
fn refresh(
    hits: &[Hit],
    visible: &[usize],
    sel: usize,
    width: u16,
    prepared: &mut RenderedDiff,
    diff: &mut Text<'static>,
) {
    match visible.get(sel).map(|&i| &hits[i]) {
        Some(h) => {
            let raw = load_diff_raw_in(&h.repo_path, &h.full, &h.file);
            *prepared = prepare_diff(&raw);
            *diff = render_prepared(prepared, width, 0);
        }
        None => {
            *prepared = RenderedDiff::default();
            *diff = Text::default();
        }
    }
}

struct View<'a> {
    query: &'a str,
    cursor: usize,
    mode: &'a Mode,
    scanning: bool,
    searched: bool,
    terms: &'a [String],
    scope_idx: usize,
    hits: &'a [Hit],
    visible: &'a [usize],
    sel: usize,
    diff: &'a Text<'static>,
    diff_scroll: u16,
    prepared: &'a RenderedDiff,
    diff_hscroll: u16,
    msg: Option<&'a str>,
}

fn draw(frame: &mut ratatui::Frame, v: View, state: &mut ListState) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(42),
        Constraint::Min(5),
    ])
    .split(frame.area());

    // ── query bar ──
    let editing = *v.mode == Mode::Editing;
    let label = "scan terms (comma separated)";
    let bar_hint = if v.scanning {
        format!(" {label} · working… ")
    } else if editing {
        format!(" {label} · enter run · esc back ")
    } else {
        format!(" {label} · / edit ")
    };
    // Empty bar shows the defaults dimmed; submitting empty runs exactly those.
    let bar_line = if v.query.is_empty() {
        Line::from(Span::styled(
            format!(" {DEFAULT_TERMS}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
    } else {
        Line::from(Span::styled(
            format!(" {}", v.query),
            Style::default().fg(Color::White),
        ))
    };
    let bar = Paragraph::new(bar_line).block(pane_block(bar_hint, editing));
    frame.render_widget(bar, areas[0]);
    if editing && !v.scanning {
        // A real terminal cursor, so it blinks where the user is typing.
        frame.set_cursor_position(Position::new(
            areas[0].x + 2 + v.cursor as u16,
            areas[0].y + 1,
        ));
    }

    // ── hits ──
    let items: Vec<ListItem> = v.visible.iter().map(|&i| hit_item(&v.hits[i])).collect();
    let scope = if v.scope_idx == 0 || v.terms.is_empty() {
        "all terms".to_string()
    } else {
        v.terms[v.scope_idx - 1].clone()
    };
    let title = if v.scanning {
        " searching every branch of every repo… ".to_string()
    } else if !v.searched {
        " type terms above, or press enter for the defaults ".to_string()
    } else if v.visible.is_empty() {
        format!(" no hits · {scope} ·  / edit · q quit ")
    } else {
        format!(
            " hits · {scope}  {}/{} of {}   {Y_MOVE} · {X_MOVE} term · / edit · q quit ",
            v.sel + 1,
            v.visible.len(),
            v.hits.len()
        )
    };
    let list = List::new(items)
        .block(pane_block(title, !editing))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, areas[1], state);
    list_scrollbar(frame, areas[1], v.visible.len(), state.offset());

    // ── diff ──
    let subtitle = match v.visible.get(v.sel).map(|&i| &v.hits[i]) {
        Some(h) => format!("{}  {}  {}", h.short, h.subject, h.file),
        None => String::new(),
    };
    let dtitle = match v.msg {
        Some(m) => format!(" ⚠ {m} "),
        None if subtitle.is_empty() => " (nothing to show) ".to_string(),
        None => {
            format!(" {subtitle}   enter difftool · {CTRL_Y_MOVE} scroll · {CTRL_X_MOVE} pan ")
        }
    };
    let diff = Paragraph::new(v.diff.clone())
        .block(pane_block(dtitle, !editing))
        .scroll((v.diff_scroll, 0));
    frame.render_widget(diff, areas[2]);
    diff_scrollbar(frame, areas[2], v.diff.lines.len(), v.diff_scroll);
    let cell = v.prepared.cell_width(areas[2].width.saturating_sub(2));
    diff_hscrollbar(frame, areas[2], v.prepared.max_line(), cell, v.diff_hscroll);
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
