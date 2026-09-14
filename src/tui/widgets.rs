//! Row renderers, pane furniture and the modal box: what every view draws
//! with, none of it knowing which view is drawing.

use crate::git::load::{Commit, FileEntry, RefKind, RefLabel, fit_refs};
use crate::tui::clamp_scroll;
use crate::tui::input::{X_MOVE, is_back, is_down, is_up};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, ListItem, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};
use std::time::Duration;

/// Render a commit row `width` columns wide. `unpushed` prepends a yellow `↑`
/// marker (this commit is on no remote yet); pushed commits get an aligning
/// blank so columns line up. Refs take only what leaves the subject
/// `MIN_SUBJECT` columns.
pub fn commit_item(c: &Commit, unpushed: bool, width: usize) -> ListItem<'static> {
    let mark = if unpushed {
        Span::styled(
            "↑ ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let mut spans = vec![
        mark,
        Span::styled(
            format!("{:<7} ", c.short),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(format!("{}  ", c.date), Style::default().fg(Color::Green)),
        Span::styled(
            format!("<{}> ", c.committer),
            Style::default().fg(Color::Blue),
        ),
    ];
    let left: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let room = width.saturating_sub(left + MIN_SUBJECT);
    spans.extend(ref_spans(&c.refs, room));
    spans.push(Span::raw(c.subject.clone()));
    ListItem::new(Line::from(spans))
}

/// Columns a commit's subject keeps however many refs point at it.
pub const MIN_SUBJECT: usize = 20;

/// `(HEAD -> main, origin/main, +2) ` in `git log --decorate`'s colours, as
/// many labels as `room` holds, or nothing when no ref points here.
fn ref_spans(refs: &[RefLabel], room: usize) -> Vec<Span<'static>> {
    if refs.is_empty() {
        return Vec::new();
    }
    let (kept, hidden) = fit_refs(refs, room);
    let mut spans = vec![Span::raw("(")];
    for (i, (kind, text)) in kept.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        let colour = match kind {
            RefKind::Head | RefKind::Detached => Color::Cyan,
            RefKind::Branch => Color::Green,
            RefKind::Remote => Color::Red,
            RefKind::Tag => Color::Yellow,
        };
        spans.push(Span::styled(
            text,
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ));
    }
    if hidden > 0 {
        spans.push(Span::raw(", "));
        spans.push(Span::styled(
            format!("+{hidden}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans.push(Span::raw(") "));
    spans
}

pub fn file_item(f: &FileEntry) -> ListItem<'static> {
    let (color, ch) = status_glyph(f.status);
    ListItem::new(Line::from(vec![
        Span::styled(format!("{ch}  "), Style::default().fg(color)),
        Span::raw(f.path.clone()),
    ]))
}

fn status_glyph(status: char) -> (Color, char) {
    match status {
        'A' => (Color::Green, 'A'),
        'M' => (Color::Yellow, 'M'),
        'D' => (Color::Red, 'D'),
        'R' => (Color::Cyan, 'R'),
        c => (Color::Gray, c),
    }
}

/// A bordered block whose border is bright when the pane is focused.
pub fn pane_block(title: impl Into<Line<'static>>, active: bool) -> Block<'static> {
    let color = if active { Color::Cyan } else { Color::DarkGray };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(title)
}

/// How long a note sits in a view's footer before the key hints come back.
pub const NOTE: Duration = Duration::from_secs(3);

/// What a `:` line asked for.
pub enum Command {
    Help,
    Quit,
    Unknown(String),
}

/// What a key did to an open `:` line.
pub enum Typed {
    Open,
    Cancel,
    Run(Command),
}

/// The `:` line in a view's footer: what has been typed after the colon. It
/// opens showing `help` as a placeholder, and Enter on an empty line runs
/// exactly that, so the one command worth knowing needs nothing typed.
#[derive(Default)]
pub struct CommandLine {
    text: String,
}

impl CommandLine {
    pub fn on_key(&mut self, code: KeyCode) -> Typed {
        match code {
            KeyCode::Esc => Typed::Cancel,
            KeyCode::Enter => Typed::Run(match self.text.trim() {
                "" | "help" | "h" | "keys" => Command::Help,
                "q" | "quit" => Command::Quit,
                other => Command::Unknown(other.to_string()),
            }),
            KeyCode::Backspace if self.text.is_empty() => Typed::Cancel,
            KeyCode::Backspace => {
                self.text.pop();
                Typed::Open
            }
            KeyCode::Char(c) => {
                self.text.push(c);
                Typed::Open
            }
            _ => Typed::Open,
        }
    }
}

/// The row under a view: the keys it answers to, or for `NOTE` what the last
/// action came to, green when it worked and yellow when it did not, or the `:`
/// line while one is open. Keys live here rather than on pane borders, which
/// only have room for what a pane is. `:help`, when `help` says the view takes
/// it, is kept at the right edge however narrow the window, since it is the way
/// to every key that falls off.
pub fn key_footer(
    actions: &[String],
    note: Option<&(bool, String)>,
    command: Option<&CommandLine>,
    help: bool,
    width: u16,
) -> Paragraph<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let line = match (command, note) {
        (Some(cmd), _) => {
            let mut spans = vec![Span::raw(format!(" :{}▏", cmd.text))];
            if cmd.text.is_empty() {
                spans.push(Span::styled("help", dim.add_modifier(Modifier::DIM)));
            }
            Line::from(spans)
        }
        (None, Some((true, text))) => Line::from(Span::styled(
            format!(" ✓ {text}"),
            Style::default().fg(Color::Green),
        )),
        (None, Some((false, text))) => Line::from(Span::styled(
            format!(" ✗ {text}"),
            Style::default().fg(Color::Yellow),
        )),
        (None, None) if !help => Line::from(Span::styled(fit(actions, width), dim)),
        (None, None) => {
            const HELP: &str = ":help ";
            let room = (width as usize).saturating_sub(HELP.len() + 2);
            let left = fit(actions, room as u16);
            let pad = (width as usize).saturating_sub(left.chars().count() + HELP.len());
            Line::from(vec![
                Span::styled(left, dim),
                Span::raw(" ".repeat(pad)),
                Span::styled(HELP, dim),
            ])
        }
    };
    Paragraph::new(line)
}

/// As many whole keys as fit in `width`, in order: a key cut off mid-word says
/// less than one left off.
fn fit(keys: &[String], width: u16) -> String {
    let mut line = String::new();
    for key in keys {
        let next = if line.is_empty() {
            format!(" {key}")
        } else {
            format!("{line} · {key}")
        };
        if next.chars().count() > width as usize {
            break;
        }
        line = next;
    }
    line
}

/// The most tabs a slider shows at once. More than this and a border runs out
/// of room, so a search with a dozen terms shows the ones around where you are.
const MAX_TABS: usize = 5;

/// A scope slider as tabs: `│` between them, and the picked one filled with
/// the border's cyan, the way a picked button is. On a pane border, whose own
/// text is already cyan, a cyan-and-bold stop the way the tab bars in the other
/// crates mark it did not stand out from the rest. Past `MAX_TABS` it shows a
/// window that slides with the picked one, and `‹`/`›` say more are hidden.
pub fn scope_tabs(labels: &[&str], picked: usize) -> Vec<Span<'static>> {
    let first = picked
        .saturating_sub(MAX_TABS / 2)
        .min(labels.len().saturating_sub(MAX_TABS));
    let last = (first + MAX_TABS).min(labels.len());
    let dim = Style::default().fg(Color::DarkGray);
    let mut spans = Vec::new();
    if first > 0 {
        spans.push(Span::styled("‹", dim));
    }
    for (i, label) in labels.iter().enumerate().take(last).skip(first) {
        if i > first {
            spans.push(Span::styled("│", dim));
        }
        let style = if i == picked {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Reset)
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    if last < labels.len() {
        spans.push(Span::styled("›", dim));
    }
    spans
}

/// Draw a vertical scrollbar down the right edge of `area`, with the thumb at
/// `top` of `total` rows. No bar is drawn when everything already fits.
fn render_vscrollbar(frame: &mut Frame, area: Rect, total: usize, top: usize) {
    let viewport = area.height.saturating_sub(2) as usize;
    if total <= viewport {
        return; // everything fits; no scrollbar needed
    }
    let mut state = ScrollbarState::new(total - viewport).position(top);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None);
    frame.render_stateful_widget(bar, area.inner(Margin::new(0, 1)), &mut state);
}

/// Scrollbar for a diff pane: thumb tracks the scroll line.
pub fn diff_scrollbar(frame: &mut Frame, area: Rect, total_lines: usize, scroll: u16) {
    render_vscrollbar(frame, area, total_lines, scroll as usize);
}

/// Scrollbar for a list pane: thumb tracks the visible window (the list's
/// `offset`). Call it right after rendering the list so the offset is current.
pub fn list_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize) {
    render_vscrollbar(frame, area, total, offset);
}

/// Horizontal scrollbar along a diff pane's bottom border. `max_line` is the
/// widest content line, `cell_w` the visible columns per side, `hscroll` the pan
/// offset. Drawn only when the content is wider than one cell (else there's
/// nothing to pan). Kept off the corners with a 1-col horizontal inset.
pub fn diff_hscrollbar(frame: &mut Frame, area: Rect, max_line: usize, cell_w: u16, hscroll: u16) {
    let cell = cell_w as usize;
    if max_line <= cell {
        return;
    }
    let mut state = ScrollbarState::new(max_line - cell).position(hscroll as usize);
    // `■` renders vertically centered and medium-weight - between the too-thin,
    // low-sitting `▬` and the full-cell block `█`.
    let bar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_symbol("■");
    frame.render_stateful_widget(bar, area.inner(Margin::new(1, 0)), &mut state);
}

// ── modal ───────────────────────────────────────────────────────────────────

/// A box over the whole screen with something the user has to read: a title, a
/// body, and no way past it but dismissing it.
///
/// It exists because the alternative is a line in a pane title, which is where
/// a failed `git difftool` used to be reported and where nobody looked: the
/// screen came back unchanged and the run looked like a no-op. A view holds an
/// `Option<Modal>` and hands it the keys first, the way the drill gates on its
/// confirm popup.
pub struct Modal {
    title: String,
    body: String,
    scroll: u16,
    /// Yellow for an alert, cyan for a reader: the one thing that differs.
    colour: Color,
    keys: &'static str,
}

impl Modal {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Modal {
        Modal {
            title: title.into(),
            body: body.into(),
            scroll: 0,
            colour: Color::Yellow,
            keys: "j/k ↑↓ scroll · esc dismiss",
        }
    }

    /// A reader: the same box in cyan, for something read by choice rather
    /// than something that went wrong. `rows` is a key and what it does, one
    /// per line, the keys in one column.
    pub fn reader(title: impl Into<String>, rows: &[(String, String)]) -> Modal {
        let width = rows
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0)
            + 3;
        let body = rows
            .iter()
            .map(|(key, does)| format!("{key:<width$}{does}"))
            .collect::<Vec<_>>()
            .join("\n");
        Modal {
            title: title.into(),
            body,
            scroll: 0,
            colour: Color::Cyan,
            keys: "j/k ↑↓ scroll · esc close",
        }
    }

    /// Free text under a reader's rows, a blank line apart: a message, which
    /// wraps on its own rather than hanging under the rows' second column.
    pub fn with_text(mut self, text: &str) -> Modal {
        if !text.is_empty() {
            self.body = format!("{}\n\n{text}", self.body);
        }
        self
    }

    /// Handle one key while the modal is up. Returns true when it was
    /// dismissed, so the caller drops it. Movement keys scroll a body too long
    /// for the box; anything else is swallowed, so a stray keypress can't
    /// close a message before it is read.
    pub fn on_key(&mut self, code: KeyCode) -> bool {
        if is_down(code) {
            self.scroll = self.scroll.saturating_add(1);
        } else if is_up(code) {
            self.scroll = self.scroll.saturating_sub(1);
        } else if is_back(code) || matches!(code, KeyCode::Enter | KeyCode::Char('q' | ' ')) {
            return true;
        }
        false
    }

    /// Draw it centered over whatever the view already rendered.
    pub fn draw(&mut self, frame: &mut Frame) {
        let full = frame.area();
        let width = box_width(full.width);
        // Measured from the wrapped text, never its line count: counting
        // unwrapped lines is what clips the bottom off a long message.
        let body_h = wrapped_height(&self.body, box_inner_width(width)) as u16;
        // The body, a blank, the keys.
        let area = popup_area(full, width, box_height(body_h + 2, full.height));

        // The viewport is what is left after the borders and the padding row.
        let viewport = area.height.saturating_sub(BOX_CHROME_H);
        self.scroll = clamp_scroll(self.scroll, body_h as usize, viewport);

        frame.render_widget(Clear, area); // wipe whatever's underneath
        let mut lines: Vec<Line> = self
            .body
            .lines()
            .map(|l| Line::raw(l.to_string()))
            .collect();
        lines.push(Line::raw(""));
        lines.push(box_hint(self.keys));

        let body = Paragraph::new(lines)
            .block(box_block(self.colour, &self.title))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0));
        frame.render_widget(body, area);
    }
}

// ── the house box ───────────────────────────────────────────────────────────
// Every overlay is built from these, so only its colour and its buttons carry
// meaning: gate red, alert yellow, offer/picker/form/reader cyan.

/// Narrowest a box may be, so a two-word message still reads as a box.
pub const BOX_MIN_W: u16 = 24;
/// Widest, so one long line does not stretch a box across a 200-column screen.
pub const BOX_MAX_W: u16 = 88;
/// Rows the chrome costs: two borders plus the single top padding row.
pub const BOX_CHROME_H: u16 = 3;
/// Columns the chrome costs: two borders plus two columns of padding a side.
pub const BOX_CHROME_W: u16 = 6;

/// How wide a box is on a screen this wide.
pub fn box_width(screen_w: u16) -> u16 {
    screen_w.saturating_sub(4).clamp(BOX_MIN_W, BOX_MAX_W)
}

/// The columns a body actually gets, which is what it must be wrapped to.
pub fn box_inner_width(width: u16) -> usize {
    width.saturating_sub(BOX_CHROME_W).max(1) as usize
}

/// How tall a box holding `body_rows` *wrapped* rows is, capped at the screen.
/// Pass the wrapped count, never the line count: measuring the unwrapped text
/// is what clips a modal's last row off and makes it look unanswerable.
///
/// The cap is the whole screen and not some fraction of it. A box is drawn over
/// a `Clear`, so it owns the screen while it is up anyway, and a fraction only
/// decides in advance that a long one gets cut off.
pub fn box_height(body_rows: u16, screen_h: u16) -> u16 {
    let floor = BOX_CHROME_H + 1;
    body_rows
        .saturating_add(BOX_CHROME_H)
        .clamp(floor, screen_h.max(floor))
}

/// The bordered block every box wears: a spaced title on the top border, and
/// otherwise an unbroken frame in the colour that says what kind it is.
///
/// Nothing else is written on the border. Keys go in the body, through
/// `box_hint`: a frame with a sentence along the bottom of it stops reading as
/// a frame, and the title then has to compete with it.
pub fn box_block(colour: Color, title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colour))
        // One blank row above the content and none below it: the key line is
        // the last thing in the box, and a blank under it is a wasted row that
        // makes the frame look loose.
        .padding(Padding::new(2, 2, 1, 0))
        .title(format!(" {} ", title.trim()))
}

/// The line of keys a box ends with, as the last row of its body. Every kind
/// puts it in the same place, so it is where the eye already is.
///
/// Quieter than anything else in the box, deliberately: an unfocused field
/// label is the default foreground dimmed, so this goes a step below that with
/// `DarkGray` dimmed again. Separation is the blank row above it and its fixed
/// place at the bottom, not brightness. Colouring it only made a guideline look
/// like something worth reading.
pub fn box_hint(keys: &str) -> Line<'static> {
    Line::from(Span::styled(
        keys.to_string(),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    ))
}

/// The Yes/No row a gate and an offer share. The labels carry their keys, so
/// the hint line does not have to teach them twice, and the picked one is
/// filled with the border colour rather than merely reversed: a reversed
/// button reads as "selected", a filled one reads as "this is what Enter does".
pub fn box_buttons(colour: Color, yes: bool) -> Line<'static> {
    let button = |label: &str, picked: bool| {
        let style = if picked {
            Style::default()
                .fg(Color::Black)
                .bg(colour)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        Span::styled(format!(" {label} "), style)
    };
    Line::from(vec![
        button("Yes (y)", yes),
        Span::raw("  "),
        button("No (n)", !yes),
    ])
}

/// A Yes/No box: a gate (red, opening on No, since a reflex Enter must never be
/// the key that fires an irreversible thing) or an offer (cyan, opening on
/// Yes, with nothing at stake). Which it is comes from `colour` and the `yes`
/// the caller opens it with. `note` is what else to know before answering.
pub fn confirm_popup(
    frame: &mut Frame,
    colour: Color,
    title: &str,
    name: &str,
    note: Option<&str>,
    yes: bool,
) {
    let full = frame.area();
    let width = box_width(full.width);
    let mut lines = vec![Line::from(Span::styled(
        name.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    for note in note.into_iter().flat_map(str::lines) {
        lines.push(Line::from(Span::styled(
            note.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    lines.push(Line::from(""));
    lines.push(box_buttons(colour, yes));
    lines.push(Line::from(""));
    lines.push(box_hint(&format!("{X_MOVE} move · enter select · y/n")));
    // Measured from the wrapped text: a long name or note wraps, and a box of
    // fixed height would put the buttons past its own bottom border.
    let inner = box_inner_width(width);
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(inner).max(1)).sum();
    let area = popup_area(full, width, box_height(rows as u16, full.height));

    frame.render_widget(Clear, area);
    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(box_block(colour, title));
    frame.render_widget(body, area);
}

/// The typed gate: red, no Yes/No, and nothing happens on Enter until `typed`
/// is exactly `name`, which the box shows so it is copied rather than guessed.
pub fn typed_popup(frame: &mut Frame, title: &str, note: Option<&str>, name: &str, typed: &str) {
    let full = frame.area();
    let width = box_width(full.width);
    let mut lines: Vec<Line<'static>> = note
        .into_iter()
        .flat_map(str::lines)
        .map(|l| Line::from(l.to_string()))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("type "),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" to confirm:"),
    ]));
    // Armed or not is shown on the thing Enter does, not only on the text: plain
    // text and a dim `enter delete` until the name matches, then bold green
    // text and `enter delete` filled red, the way a picked gate button is.
    let armed = typed == name;
    let field = if armed {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    lines.push(Line::from(Span::styled(format!("{typed}▏"), field)));
    lines.push(Line::from(""));
    let enter = if armed {
        Span::styled(
            " enter delete ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " enter delete ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )
    };
    let esc = Span::styled(
        "  esc cancel",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    );
    lines.push(Line::from(vec![enter, esc]));
    let inner = box_inner_width(width);
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(inner).max(1)).sum();
    let area = popup_area(full, width, box_height(rows as u16, full.height));

    frame.render_widget(Clear, area);
    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(box_block(Color::Red, title));
    frame.render_widget(body, area);
}

/// Rows `text` takes once wrapped to `width` columns.
pub fn wrapped_height(text: &str, width: usize) -> usize {
    text.lines()
        .map(|l| l.chars().count().div_ceil(width).max(1))
        .sum()
}

/// A box of at most `w`×`h`, centered in `area`.
pub fn popup_area(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
