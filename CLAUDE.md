# CLAUDE.md

## Overview
`sluuz` is a Rust CLI published on crates.io that acts as a **git superset**: the
binary is `slu` (a 3-letter stand-in for `git`). Any command git understands is
passed straight through to real git; on top of that `slu` adds cross-repo
management, history/secret search, a prettier log, and interactive TUI explorers.
License: AGPL-3.0-only. Crate name `sluuz`, binary name `slu`.

## Tech stack
- Rust 2021 edition.
- `clap` (derive) with `external_subcommand` for git passthrough.
- `ratatui` 0.29 + `crossterm` 0.28 for the TUIs.
- `syntect` with `default-features=false, features=["default-fancy"]` — pure-Rust
  regex backend (no C/oniguruma) so diffs highlight cleanly on Windows.
- `rayon` (parallel multi-repo ops), `walkdir` (repo discovery), `colored`,
  `terminal_size`.

## Layout
- `src/main.rs` — clap `Cli`/`Cmd`, dispatch, `passthrough()` to git, top-level
  `--help` text (per-command options listed one-per-line via `verbatim_doc_comment`).
- `src/commands/*.rs` — one module per superpower verb: `search`, `scan`, `repos`,
  `sync`, `tidy`, `each`, `trace`, `update`, `completions`, and the interactive
  TUIs `ilog`, `ibranch`, `istatus`, `itidy`.
- `src/git.rs` — repo discovery (`find_repos`), `display_name`, `git_capture`/`git_run`.
- `src/history.rs` — pickaxe search (`git log -S`) and branch lookup.
- `src/tui.rs` — shared TUI building blocks: diff parsing + syntect highlighting,
  scrollbars, key predicates, hint-label consts, and `run_difftool`/`difftool_commit`
  (suspend the TUI → run `git difftool` → re-enter).

## Build / test / lint
- Build: `cargo build` (debug) / `cargo build --release`.
- Lint: `cargo clippy` — keep it warning-clean before any release.
- Run the dev binary directly: `./target/debug/slu <cmd>` (there is no dev.sh).
- No automated tests. Scratch test repos go in `test-playground/` (gitignored) —
  build them there and leave them for the user to poke at.

## Release workflow (always, in order)
change → `cargo clippy` → bump `version` in `Cargo.toml` → `cargo build --release`
→ `git commit` → `git push` → `cargo publish` → `git tag -a vX.Y.Z -m "…"` →
`git push origin vX.Y.Z`. Every published version gets a matching `vX.Y.Z` tag.

## Conventions
- Commits: short conventional-tag messages (`feat:`, `fix:`, …). **Never** add
  co-author / `Co-Authored-By` trailers.
- **Never shadow a real git command.** Enhanced views get distinct verbs
  (`trace`, not `log`; the `i` prefix for interactive: `ilog`/`ibranch`/`istatus`/
  `itidy`). Passthrough must stay intact.
- Fix the root cause; call out any workaround explicitly as such.
- **TUI key rule — plain = top pane, Ctrl = the diff.** `j/k` ≡ `↑/↓` and `h/l` ≡
  `←/→` navigate the top pane; `Ctrl-j/k` ≡ `Ctrl-↑/↓` (+ `Ctrl-d/u`) scroll the
  diff and `Ctrl-h/l` ≡ `Ctrl-←/→` pan it. `Esc` = back, `Enter` = open/drill in
  (on a diff: open the user's `git difftool`). Hint labels come from the
  `Y_MOVE`/`CTRL_Y_MOVE`/`X_MOVE`/`CTRL_X_MOVE` consts in `tui.rs`.
- Run git from the **repo root** (`git rev-parse --show-toplevel`) when acting on
  paths: `git status` reports root-relative paths, so running diff/add from a
  subdirectory fails to resolve them.

## Gotchas
- Stale debug binary: `cargo build` sometimes doesn't pick up changes — run
  `cargo clean -p sluuz` then rebuild when behavior seems wrong.
- Diff TUI perf: highlight ONCE via `prepare_diff` (expensive syntect), then
  `render_prepared` re-lays-out per scroll/pan cheaply. Don't re-highlight on keypress.
- `slu` self-update on Windows renames the running `slu.exe` aside before cargo
  overwrites it.
- VS Code's integrated terminal can't distinguish `Ctrl-J` from Enter on any OS;
  the fix is a user `keybindings.json` remap to `Ctrl-Down` (see README Troubleshooting).
- The kitty protocol we push (DISAMBIGUATE_ESCAPE_CODES) makes `Ctrl-[` a distinct
  key instead of ESC — `norm_esc()` in `tui.rs` folds it back to `Esc` (vim parity).
- `git difftool` needs the terminal: suspend the TUI (pop kitty flags →
  `ratatui::restore()`), run it, then `ratatui::init()` + re-push. Check
  `diff.tool`/`merge.tool` first so an unconfigured user gets a message, not a
  torn-down screen.
