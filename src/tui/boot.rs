//! The rows drawn while a tree of repos is read, before the terminal is taken
//! over: a braille wordmark decoding above a progress line.

use crate::git::{Progress, Stage};
use ratatui::crossterm::terminal;
use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

/// How long a read may take before it starts explaining itself, so a small tree
/// opens in silence.
const GRACE: Duration = Duration::from_millis(150);
const REDRAW: Duration = Duration::from_millis(60);
const BAR: usize = 20;

/// The wordmark, two rows tall.
const MARK: [&str; 2] = ["⡎⠉ ⡇⠀ ⡇⢸ ⡇⢸ ⠉⡹", "⣈⡱ ⣇⣀ ⢇⡸ ⢇⡸ ⣎⣀"];
const SCRAMBLED: &str = "\x1b[2;90m";
const LOCKED: &str = "\x1b[1;96m";
const OFF: &str = "\x1b[0m";

/// Run `work`, drawing how it goes on stderr once it has outlasted `GRACE`, and
/// wipe the rows before returning so the terminal is as it was found. Draws from
/// a timer rather than from `work`'s reports, because the rows must keep moving
/// while a slow repo holds every report back. Draws nothing unless stderr is a
/// terminal.
pub fn while_loading<T>(work: impl FnOnce(&Progress) -> T) -> T {
    let progress = Progress::default();
    if !io::stderr().is_terminal() {
        return work(&progress);
    }
    let (stop, stopped) = mpsc::channel::<()>();
    thread::scope(|scope| {
        let shared = &progress;
        scope.spawn(move || draw_until(&stopped, shared));
        let out = work(shared);
        drop(stop);
        out
    })
}

fn draw_until(stopped: &Receiver<()>, progress: &Progress) {
    if !matches!(stopped.recv_timeout(GRACE), Err(RecvTimeoutError::Timeout)) {
        return;
    }
    let mut err = io::stderr();
    let mut noise = 0x2545_F491_4F6C_DD1D_u64;
    let mut up = "";
    loop {
        let (line, share) = progress_line(progress.stage());
        let mark = wordmark(share, &mut noise);
        // Back up over the wordmark, so every frame overwrites the last in place.
        write!(err, "{up}\r{mark}\r\x1b[K{line}").ok();
        err.flush().ok();
        up = "\x1b[2A";
        if !matches!(stopped.recv_timeout(REDRAW), Err(RecvTimeoutError::Timeout)) {
            break;
        }
    }
    write!(err, "\x1b[2A\r\x1b[J").ok();
    err.flush().ok();
}

/// The line under the wordmark, cut to the window since a line that wraps is one
/// `\r` can no longer rewrite, and the share of the wordmark to lock in.
fn progress_line(stage: Stage) -> (String, f64) {
    let (line, share) = match stage {
        Stage::Finding(found) => (format!("finding repos ... {found}"), 0.0),
        Stage::Working(done, total) => {
            let full = (done * BAR).checked_div(total).unwrap_or(BAR);
            let bar: String = (0..BAR).map(|i| if i < full { '#' } else { '-' }).collect();
            (
                format!("reading repos [{bar}] {done}/{total}"),
                full as f64 / BAR as f64,
            )
        }
    };
    let cols = terminal::size().map_or(80, |(cols, _)| usize::from(cols));
    (line.chars().take(cols.saturating_sub(1)).collect(), share)
}

/// The wordmark with the leftmost `share` of its columns locked in and the rest
/// noise, ending each row with a newline.
fn wordmark(share: f64, noise: &mut u64) -> String {
    let cols = MARK[0].chars().count();
    let locked = (share * cols as f64).round() as usize;
    let mut out = String::new();
    for row in MARK {
        for (i, ch) in row.chars().enumerate() {
            // A gap stays a gap, or the wordmark's letters would run together.
            if ch == ' ' || i < locked {
                out.push_str(LOCKED);
                out.push(ch);
            } else {
                out.push_str(SCRAMBLED);
                out.push(scramble(noise));
            }
            out.push_str(OFF);
        }
        out.push_str("\x1b[K\n");
    }
    out
}

/// One braille cell of noise from the six dot patterns the wordmark mostly uses.
fn scramble(noise: &mut u64) -> char {
    *noise ^= *noise << 13;
    *noise ^= *noise >> 7;
    *noise ^= *noise << 17;
    char::from_u32(0x2800 + (*noise % 64) as u32).unwrap_or('⠿')
}
