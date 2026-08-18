<!--
AI-ONLY DOCUMENT. This file exists to give an AI agent the COMPLETE operating picture for this repo. Optimize for completeness and precision for the agent, not for human readability. Humans read README.md instead. Do not remove detail to make this nicer — err toward more explicit, not less. FORMAT: machine-read, not a formatted human doc. Do NOT hard-wrap lines to a column width for readability; put each rule/point on ONE line, however long.
-->
# AGENTS.md

## Hard rules
- **Commit, push, and publish only when the user says to ship.** They test interactively first; a mid-work commit is never the deliverable.
- Release flow, in this exact order: `cargo clippy` warning-clean (+ `cargo test` if a suite exists) → bump `version` in `Cargo.toml` → one commit (short conventional message, never co-authored) → `git push origin main` → `cargo publish` (dry-run first; publishing is irreversible) → **tag only after publish succeeds**: `git tag vX.Y.Z && git push origin --tags`. A tag must never point at a version that failed to publish.
- Commit messages: short conventional tags (`feat:`, `fix:`, ...). **Never** add a `Co-Authored-By` trailer.
- **No em-dashes** anywhere user-facing (README, --help, crate description, commit messages, prose) - they read as AI-generated text.
- **Never shadow a real git command** - passthrough is the whole premise, so an enhanced view always gets a distinct verb (`trace`, not `log`; the `i` prefix for interactive: `irepos`/`ilog`/`ibranch`/`istatus`/`itidy`/`iscan`).
- Fix the root cause. If a workaround must ship, say the word "workaround" out loud, so a silent patch never passes as a real fix. Same for lints: never `#[allow]` a warning away; delete or fix the code it points at.
- Scratch/test git repos go in `test-playground/` (gitignored). Build them there and **leave them** - the user opens them to test the TUIs by hand.

## Invariants and gotchas
- **When editing TUI keys - plain = top pane, Ctrl = the diff.** `j/k`≡`↑/↓` and `h/l`≡`←/→` drive the top pane; `Ctrl-j/k`≡`Ctrl-↑/↓` (+`Ctrl-d/u`) scroll the diff and `Ctrl-h/l`≡`Ctrl-←/→` pan it. `Esc` = back, `Enter` = open/drill in (on a diff: the user's `git difftool`). Hint labels come from the `Y_MOVE`/`CTRL_Y_MOVE`/`X_MOVE`/`CTRL_X_MOVE` consts in `tui/input.rs` - never hand-write a key hint.
- **When parsing git output where leading whitespace is significant** (notably `git status --porcelain`, whose first column is a space for unstaged files), use `git_capture_raw`, not `git_capture`: the latter `.trim()`s, which ate the leading space of the first record and shifted the whole line ("Cargo.lock" → "argo.lock").
- **When running git against paths it reported** (`git status --porcelain`, `git show --name-status`), anchor at the repo root (`git rev-parse --show-toplevel`): git reports paths root-relative, so `diff`/`add`/`show -- <path>` from a subdirectory silently mis-resolves - istatus showed "Could not access '<path>'", ilog/ibranch a blank diff pane. Every view now resolves the root once and passes it down: **no view calls `set_current_dir`**, and every loader in `tui/load.rs` takes the repo as its first argument, which is what lets `irepos` walk several repos in one session.
- **When a TUI shells out to an interactive program** (e.g. `git difftool`), you must suspend it: pop the kitty flags → `ratatui::restore()` → run → `ratatui::init()` → re-push. Check `diff.tool`/`merge.tool` *first*, so an unconfigured user gets a message instead of a torn-down screen. See `run_difftool` in `tui/difftool.rs`.
- **When handling `Esc`:** the kitty protocol we push (DISAMBIGUATE_ESCAPE_CODES) turns `Ctrl-[` into a distinct key instead of the ESC byte. `norm_esc()` in `tui/input.rs` folds it back - call it on every key event, or vim users lose `Ctrl-[`.
- **When the diff view feels laggy:** highlight ONCE with `prepare_diff` (syntect is the expensive part), then `render_prepared` re-lays-out per scroll/pan. Never re-highlight on a keypress.
- **When deciding which branches are safe to delete** (`tidy`/`itidy`): use "upstream gone" (`%(upstream:track)` contains `[gone]`), never `git branch --merged` - merged-into-HEAD wrongly lists `main` and release branches that are still alive on the remote.
- **When reading a branch name to feed `git branch -d`:** take `%(refname)` and strip `refs/heads/`. `%(refname:short)` returns `heads/v1.2.3` when a tag shares the name, which `git branch -d` then rejects.
- **When adding a level to the drill** (repos → branches → commits → files → diff): it is ONE app in `src/app/`, not one per command. Add the `Level`, its scope slider, its `enter_*` loader, an arm in `app/keys.rs` and one in `app/ui.rs`. `irepos`/`ibranch`/`ilog` are ~40-line commands that only pick the level to start at, and `Esc` quits at whichever level that was (`App::back`).
- **When a level's bottom pane previews the next level down:** load that pane on the selection move, and load the level *below* it only on `Enter`. Loading eagerly all the way down costs a `git log` on every keypress of the repo list.
- **When verifying a TUI change:** drive it in a pty rather than eyeballing it. Spawn `slu` on a `pty.openpty()` sized *before* the child starts (ratatui reads the size once at init; a 0x0 terminal draws nothing), feed keystrokes, then replay the captured bytes through `pyte` to get the final screen as text. Diffing that screen before and after a refactor is what proved `istatus`/`iscan`/`itidy` came through byte-identical. Note `supports_keyboard_enhancement()` blocks ~2s waiting for a reply no fake pty sends, so allow for it.
- **When behavior doesn't match the code you just wrote:** the debug binary is stale. `cargo clean -p sluuz`, then rebuild.
- `syntect` is pinned to `default-features=false` + `default-fancy` - the pure-Rust regex backend, so there's no C/oniguruma and it builds on Windows. Don't "fix" the feature flags.
- `slu completions` delegates to git's own completion (that's the only way branch names complete). git's completion is **lazy-loaded**, and completing `slu` never trips that loader - so the emitted script force-loads it.
- `slu update` on Windows renames the running `slu.exe` aside first: Windows cannot overwrite a running binary.
- VS Code's integrated terminal cannot distinguish `Ctrl-J` from `Enter` on any OS (not just Windows). The fix is a user `keybindings.json` remap to `Ctrl-Down` - see README Troubleshooting; there is no code-side fix.

## Build / lint / test
- `cargo build` (debug) · `cargo build --release`
- `cargo clippy` - must be warning-clean before any release.
- Run the dev binary directly: `./target/debug/slu <cmd>` (there is no dev.sh).
- `cargo test` - unit tests live in `#[cfg(test)] mod tests` inside the file they cover (`commands/ilog.rs`, `commands/iscan.rs`), plus `tests/readme.rs`. **Coverage is thin and all of it is pure logic**; the TUIs themselves have none, so new *pure logic* (parsing, branch classification, scope filtering) should always get a unit test - throwaway manual checks let regressions through (see stowe's `tests/cli.rs` for the pattern).

## Overview
`sluuz` is a Rust CLI on crates.io that is a **git superset**: the binary is `slu` (a 3-letter stand-in for `git`). Anything git understands is passed straight through to real git; on top, `slu` adds cross-repo management, history/secret search, a prettier log, and interactive TUIs. Crate `sluuz`, binary `slu`, AGPL-3.0-only.

Layout:
- `src/main.rs` - the clap `Cmd` enum + `passthrough()`, nothing else.
- `src/git.rs` - repo discovery, `git_capture{,_raw}`/`git_run`, `RepoStatus`, `SEP`.
- `src/history.rs` - the pickaxe shared by `search`, `scan` and `iscan`.
- `src/app/` - **the drill**: one `App` with a `Level` stack (repos → branches → commits → files → diff), split into `repos`/`branches`/`commits`/`diff` state, `keys.rs` for input and `ui.rs` for rendering. `irepos`, `ibranch` and `ilog` are three entry points into it.
- `src/tui/` - primitives with no view knowledge: `input` (key predicates + hint labels), `widgets`, `highlight` (syntect), `difftool`, `load` (git loaders), and terminal setup in `mod.rs`.
- `src/commands/` - one file per command. `iscan`, `istatus` and `itidy` stay standalone (a query bar, a staging area and a confirm popup have no place in a nav stack) but wear the same shape: a state struct, `on_key`, `draw`.

## Self-repair
If this file contradicts the code, **the code wins** - fix AGENTS.md in the same session you notice.
