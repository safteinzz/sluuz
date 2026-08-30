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
        let width = full.width.saturating_sub(4).clamp(24, 88);
        // Sized to the text, capped at four fifths of the screen.
        let inner_w = width.saturating_sub(4).max(1) as usize;
        let body_h = wrapped_height(&self.body, inner_w) as u16;
        let height = body_h
            .saturating_add(2)
            .clamp(5, (full.height * 4 / 5).max(5));
        let area = popup_area(full, width, height);

        let viewport = area.height.saturating_sub(2);
        self.scroll = clamp_scroll(self.scroll, body_h as usize, viewport);

        frame.render_widget(Clear, area); // wipe whatever's underneath
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .padding(Padding::horizontal(1))
            .title(self.title.clone())
            .title_bottom(Span::styled(
                " esc dismiss ",
                Style::default().fg(Color::DarkGray),
            ));
        let body = Paragraph::new(self.body.clone())
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0));
        frame.render_widget(body, area);
    }
}

/// Rows `text` takes once wrapped to `width` columns.
fn wrapped_height(text: &str, width: usize) -> usize {
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
