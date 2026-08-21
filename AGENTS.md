<!--
AI-ONLY DOCUMENT. This file exists to give an AI agent the COMPLETE operating picture for this repo. Optimize for completeness and precision for the agent, not for human readability. Humans read README.md instead. Do not remove detail to make this nicer, err toward more explicit, not less. FORMAT: machine-read, not a formatted human doc. Do NOT hard-wrap lines to a column width for readability; put each rule/point on ONE line, however long.
-->
# AGENTS.md

Working brief for an AI coding agent, not documentation for people (the README covers that): the rules, invariants and gotchas needed to change this project correctly without rediscovering them.

## Hard rules
- Commit, push, and publish only when the user says to ship; a mid-work commit is never the deliverable, because the user tests interactively first.
- Commit messages are short single-line conventional ones (`feat:`, `fix:`, `chore:`, ...), never with a `Co-Authored-By` trailer and never with a verbose body.
- Release flow, in this exact order: ask whether this shipment gets tests and write them only if the user says yes -> bump `version` in `Cargo.toml` -> `cargo clippy-all` clean and `cargo test` green, which is also what refreshes `Cargo.lock` with the new version -> one commit -> `git push origin main` -> `cargo publish` (dry-run first, publishing is irreversible) -> tag only after publish succeeds with `git tag vX.Y.Z && git push origin --tags`; a tag must never point at a version that failed to publish, and the bump comes first because `cargo publish` fails on a `Cargo.lock` that still holds the old version.
- Tests are proposed at ship time and never before: the first step of the release flow is to ask the user, in plain words, whether this shipment gets tests, and they are written only on a yes, so the decision is always theirs but the question is never forgotten.
- Never write a test for behaviour that has not shipped yet, because code that is not in the last release tag is still being designed, and a test pinning a shape that is about to change is how a suite starts lying.
- A test may only assert something the README or `--help` promises, or a pure-logic invariant (parsing, generation, path resolution, validation); never the shape of a private function and never the specific diff that was just made, since those rot on the next refactor and teach nothing about whether the program works.
- Removing a promise from the README removes its tests in the same commit.
- A test may only write inside a temp directory it deletes, never a real config, data, cache or content directory and never a fixed path, so a machine is left exactly as it was before the suite ran.
- Never drive the interface to test it: build it, say what changed and what to look at, and let the user run it, because they see the screen instantly while an agent driving a pty or a tmux pane is slow and wrong about what it looks like; logic that is not visual can still be checked directly from `tests/`.
- Never `cargo install` to test: run the release binary at `./target/release/slu` directly, because installing replaces the binary on PATH with a work-in-progress build; install only when the user asks.
- `main` is protected: no force-push and no history rewrite, so a mistake is fixed with a forward commit.
- No em-dashes anywhere (code, comments, README, `--help`, crate description, commit messages, prose), because they read as AI-generated text; use `-` instead.
- Fix the root cause, and if a workaround must ship say the word "workaround" out loud so a silent patch never passes as a real fix; the same goes for lints, where an `#[allow]` is never the answer and the code it points at gets fixed or deleted.
- `TODO-LIST.md` (gitignored) holds one-line ideas, and the line is deleted when the idea ships.
- **Never shadow a real git command** - passthrough is the whole premise, so an enhanced view always gets a distinct verb (`trace`, not `log`; the `i` prefix for interactive: `irepos`/`ilog`/`ibranch`/`istatus`/`itidy`/`iscan`).
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
- **When behavior doesn't match the code you just wrote:** the debug binary is stale. `cargo clean -p sluuz`, then rebuild.
- `syntect` is pinned to `default-features=false` + `default-fancy` - the pure-Rust regex backend, so there's no C/oniguruma and it builds on Windows. Don't "fix" the feature flags.
- `slu completions` delegates to git's own completion (that's the only way branch names complete). git's completion is **lazy-loaded**, and completing `slu` never trips that loader - so the emitted script force-loads it.
- `slu update` on Windows renames the running `slu.exe` aside first: Windows cannot overwrite a running binary.
- VS Code's integrated terminal cannot distinguish `Ctrl-J` from `Enter` on any OS (not just Windows). The fix is a user `keybindings.json` remap to `Ctrl-Down` - see README Troubleshooting; there is no code-side fix.

## Build / lint / test
- `cargo build --release`, binary at `target/release/slu`.
- `cargo clippy-all` is the lint pass, aliased in `.cargo/config.toml` to `clippy --release --all-targets -- -D warnings`; use it rather than a bare `cargo clippy`, which skips `tests/` and `examples/` and only warns where the release flow wants a failure.
- `cargo test`.
- Run the debug binary directly as `./target/debug/slu <cmd>` (there is no dev.sh).
- Unit tests live in a `#[cfg(test)] mod tests` inside the file they cover (`commands/ilog.rs`, `commands/iscan.rs`), plus `tests/readme.rs`; all of it is pure logic, since the TUIs have none.

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
If anything here contradicts the code, the code wins; fix AGENTS.md in the same session you notice the drift.
