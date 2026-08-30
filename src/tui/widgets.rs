//! Row renderers, pane furniture and the modal box: what every view draws
//! with, none of it knowing which view is drawing.

use crate::git::load::{Commit, FileEntry};
use crate::tui::clamp_scroll;
use crate::tui::input::{is_back, is_down, is_up};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, ListItem, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};

/// Render a commit row. `unpushed` prepends a yellow `↑` marker (this commit is
/// on no remote yet); pushed commits get an aligning blank so columns line up.
pub fn commit_item(c: &Commit, unpushed: bool) -> ListItem<'static> {
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
    ListItem::new(Line::from(vec![
        mark,
        Span::styled(
            format!("{:<8}", c.short),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(format!("{}  ", c.date), Style::default().fg(Color::Green)),
        Span::styled(
            format!("<{}> ", c.committer),
            Style::default().fg(Color::Blue),
        ),
        Span::raw(c.subject.clone()),
    ]))
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
pub fn pane_block(title: String, active: bool) -> Block<'static> {
    let color = if active { Color::Cyan } else { Color::DarkGray };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(title)
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
/// `Option<Modal>` and hands it the keys first, the way `itidy` gates on its
/// confirm popup.
pub struct Modal {
    title: String,
    body: String,
    scroll: u16,
}

impl Modal {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Modal {
        Modal {
            title: title.into(),
            body: body.into(),
            scroll: 0,
        }
    }

    /// Handle one key while the modal is up. Returns true when it was
    /// dismissed, so the caller drops it. Movement keys scroll a body too long
    /// for the box; anything else is swallowed, so a stray keypress can't
    /// close a message before it is read.
    pub fn on_key(&mut self, code: KeyCode) -> bool {
        if is_down(code) || code == KeyCode::PageDown {
            self.scroll = self.scroll.saturating_add(1);
        } else if is_up(code) || code == KeyCode::PageUp {
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
        lines.push(box_hint("j/k ↑↓ scroll · esc dismiss"));

        let body = Paragraph::new(lines)
            .block(box_block(Color::Yellow, &self.title))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0));
        frame.render_widget(body, area);
    }
}

// ── the house box ───────────────────────────────────────────────────────────
// Every overlay is built from these, so only its colour and its buttons carry
// meaning: gate red, alert yellow, offer/picker/form/reader cyan. The anatomy
// is written down in AGENTS.md under "What every box looks like"; this is that
// paragraph as code, and nothing should draw a bordered overlay without it.

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
