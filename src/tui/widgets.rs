//! Row renderers and pane furniture shared by every view.

use crate::git::load::{Commit, FileEntry};
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState};

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
