//! Shared building blocks for the interactive TUIs: terminal setup, key
//! predicates, widgets, diff rendering, and the difftool handoff. Reading git is
//! not terminal work, so the queries the views run live in `git::load`.
//!
//! Nothing in here knows about any one view. `app/` builds the repos → branches
//! → commits → diff drill on top of it, and `iscan`/`istatus` use the
//! same pieces to draw their own screens.

/// How long a frame waits for a key while something is still arriving: a
/// background load, a diff being highlighted. With nothing in flight the loops
/// block on the key instead, so an idle TUI costs nothing.
pub const FRAME: std::time::Duration = std::time::Duration::from_millis(33);

pub mod boot;
pub mod difffeed;
pub mod difftool;
pub mod highlight;
pub mod input;
pub mod widgets;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::crossterm::{ExecutableCommand, execute};
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

// ── taking the terminal ─────────────────────────────────────────────────────

/// Take the terminal for a TUI as `ratatui::init` does, first putting back the
/// mode the first `init` found, on a Windows console one crossterm can read
/// keys in.
pub fn init() -> DefaultTerminal {
    #[cfg(unix)]
    tty::take();
    #[cfg(windows)]
    console::take();
    ratatui::init()
}

/// Give the terminal back as the first `init` found it.
pub fn restore() {
    ratatui::restore();
    #[cfg(unix)]
    tty::give_back();
    #[cfg(windows)]
    console::give_back();
}

/// The terminal settings the first `init` found, which every later `init`
/// starts from and `restore` hands back. crossterm saves whatever settings it
/// finds each time raw mode goes on, so without this a difftool that turns
/// echo off, or dies in raw mode, leaves the shell that way once the TUI quits.
#[cfg(unix)]
mod tty {
    use rustix::termios::{OptionalActions, Termios, tcgetattr, tcsetattr};
    use std::fs::File;
    use std::sync::OnceLock;

    static FOUND: OnceLock<Termios> = OnceLock::new();

    /// Apply `found` to the terminal crossterm puts in raw mode: `/dev/tty`,
    /// else stdin.
    fn set(found: &Termios) {
        match File::open("/dev/tty") {
            Ok(tty) => {
                let _ = tcsetattr(&tty, OptionalActions::Now, found);
            }
            Err(_) => {
                let _ = tcsetattr(std::io::stdin(), OptionalActions::Now, found);
            }
        }
    }

    pub fn take() {
        match FOUND.get() {
            Some(found) => set(found),
            None => {
                let now = match File::open("/dev/tty") {
                    Ok(tty) => tcgetattr(&tty),
                    Err(_) => tcgetattr(std::io::stdin()),
                };
                if let Ok(now) = now {
                    let _ = FOUND.set(now);
                }
            }
        }
    }

    pub fn give_back() {
        if let Some(found) = FOUND.get() {
            set(found);
        }
    }
}

/// The console input mode the first `init` found, which `restore` hands to
/// whoever gets the terminal next and `init` starts from again, never from what
/// a child left: a difftool ended by Ctrl-C can leave
/// `ENABLE_VIRTUAL_TERMINAL_INPUT` on, under which crossterm reads Enter, Esc
/// and arrows as loose characters.
#[cfg(windows)]
mod console {
    use crossterm_winapi::{ConsoleMode, Handle};
    use std::sync::{Mutex, OnceLock, PoisonError};
    use std::thread;
    use std::time::{Duration, Instant};

    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

    /// How long the console has to be left alone before `guard` stops
    /// watching it.
    const QUIET: Duration = Duration::from_secs(2);

    static FOUND: OnceLock<u32> = OnceLock::new();

    /// The raw mode the TUI holds the console in, `None` while it has handed
    /// the console over.
    static HELD: Mutex<Option<u32>> = Mutex::new(None);

    fn input() -> Option<ConsoleMode> {
        Handle::current_in_handle().ok().map(ConsoleMode::from)
    }

    pub fn take() {
        let Some(input) = input() else {
            return;
        };
        let again = FOUND.get().is_some();
        if !again && let Ok(mode) = input.mode() {
            let _ = FOUND.set(mode);
        }
        let Some(found) = FOUND.get() else {
            return;
        };
        let _ = input.set_mode(found & !ENABLE_VIRTUAL_TERMINAL_INPUT);
        let raw = found
            & !(ENABLE_VIRTUAL_TERMINAL_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT);
        *HELD.lock().unwrap_or_else(PoisonError::into_inner) = Some(raw);
        if again {
            thread::spawn(move || guard(raw));
        }
    }

    pub fn give_back() {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        *held = None;
        if let (Some(input), Some(found)) = (input(), FOUND.get()) {
            let _ = input.set_mode(*found);
        }
    }

    /// Put `raw` back whenever something else changes the console's mode,
    /// until it has been left alone for `QUIET` or the TUI hands it over. Git
    /// for Windows runs a difftool through its MSYS2 shell, which on Ctrl-C
    /// outlives git and writes back the mode it started with about 50 ms
    /// after the TUI has taken the console back, turning Ctrl-C into a signal
    /// again.
    fn guard(raw: u32) {
        let Some(input) = input() else {
            return;
        };
        let mut touched = Instant::now();
        while touched.elapsed() < QUIET {
            thread::sleep(Duration::from_millis(10));
            let held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
            if *held != Some(raw) {
                return;
            }
            if input.mode().is_ok_and(|mode| mode != raw) {
                let _ = input.set_mode(raw);
                touched = Instant::now();
            }
        }
    }
}

// ── keyboard enhancement (kitty protocol) ───────────────────────────────────

/// Request DISAMBIGUATE_ESCAPE_CODES if the terminal supports the kitty
/// keyboard protocol. Returns true if pushed (caller must call pop).
/// This makes Ctrl+J distinct from Enter in terminals like VS Code.
pub fn push_keyboard_enhancement() -> bool {
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        true
    } else {
        false
    }
}

pub fn pop_keyboard_enhancement() {
    let _ = stdout().execute(PopKeyboardEnhancementFlags);
}

/// Hand the terminal to `run` (a difftool, which may be an editor), then take
/// it back. Ctrl-C meanwhile reaches the child and not the app, whose screen
/// would otherwise vanish with whatever it was holding.
pub fn suspended<T>(terminal: &mut DefaultTerminal, enhanced: bool, run: impl FnOnce() -> T) -> T {
    if enhanced {
        pop_keyboard_enhancement();
    }
    restore();
    let out = interrupts_to_child(run);
    *terminal = init();
    if enhanced {
        push_keyboard_enhancement();
    }
    out
}

fn interrupts_to_child<T>(run: impl FnOnce() -> T) -> T {
    let ours = interrupts_end_us();
    ours.store(false, Ordering::SeqCst);
    let out = run();
    ours.store(true, Ordering::SeqCst);
    out
}

/// Whether the interrupt keys end the app the way they would with no handler,
/// which `suspended` turns off while a child has the terminal. The handler is
/// installed once and never removed, because removing one through
/// signal-hook leaves the signal ignored for the rest of the process. It is a
/// handler, never "ignore": a child inherits an ignored SIGINT, or a console's
/// Ctrl-C switched off, and could then not be cancelled.
fn interrupts_end_us() -> &'static Arc<AtomicBool> {
    static OURS: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    OURS.get_or_init(|| {
        let ours = Arc::new(AtomicBool::new(true));
        catch_interrupts(&ours);
        ours
    })
}

/// Ctrl-C and Ctrl-\, as SIGINT and SIGQUIT.
#[cfg(unix)]
fn catch_interrupts(ours: &Arc<AtomicBool>) {
    use signal_hook::consts::{SIGINT, SIGQUIT};
    for signal in [SIGINT, SIGQUIT] {
        let _ = signal_hook::flag::register_conditional_default(signal, ours.clone());
    }
}

/// Ctrl-C and Ctrl-Break, which the C runtime turns into SIGINT and SIGBREAK.
/// Ending the app exits with the code Windows' own handler would, since
/// signal-hook can emulate no default for SIGBREAK.
#[cfg(windows)]
fn catch_interrupts(ours: &Arc<AtomicBool>) {
    use signal_hook::consts::{SIGBREAK, SIGINT};
    const STATUS_CONTROL_C_EXIT: i32 = 0xC000_013A_u32.cast_signed();
    for signal in [SIGINT, SIGBREAK] {
        let _ = signal_hook::flag::register_conditional_shutdown(
            signal,
            STATUS_CONTROL_C_EXIT,
            ours.clone(),
        );
    }
}

// ── terminal geometry ───────────────────────────────────────────────────────

pub fn pane_width(terminal: &DefaultTerminal) -> u16 {
    terminal
        .size()
        .map(|s| s.width.saturating_sub(2))
        .unwrap_or(120)
}

/// Clamp a scroll offset so the last line can't scroll above the viewport -
/// stops you from scrolling off the bottom into empty space.
pub fn clamp_scroll(scroll: u16, total_lines: usize, viewport: u16) -> u16 {
    let max = (total_lines.min(u16::MAX as usize) as u16).saturating_sub(viewport);
    scroll.min(max)
}

/// Clamp horizontal scroll so you can't pan past the longest line. `cell_w` is
/// roughly one side's visible width.
pub fn clamp_hscroll(hscroll: u16, max_line: usize, cell_w: u16) -> u16 {
    let max = (max_line.min(u16::MAX as usize) as u16).saturating_sub(cell_w);
    hscroll.min(max)
}
