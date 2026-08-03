# sluuz

**git, but it sleuths.** 🕵️

`sluuz` is a drop-in **git superset**. The command you type is the short `slu`,
and every git command still works exactly as before — `slu commit`, `slu push`,
`slu log` all pass straight through to git. On top of that you get superpowers a
shell alias could never give you: pickaxe a string through every branch of every
repo at once, audit history for leaked secrets, and manage many repos in one go.

The name? It's for people who like **sleuthing** through git history.

## Install

```bash
cargo install sluuz
```

This installs a single command: **`slu`** — three letters, same length as `git`
itself. Update later with `slu update` (or `cargo install sluuz --force`).

## It's just git… until it isn't

Anything git understands is forwarded verbatim, with your editor, pager, prompts,
colors, and exit codes intact:

```bash
slu commit -m "fix"
slu push
slu rebase -i HEAD~3
slu log --oneline
```

Then come the superpowers.

### `slu search` — sleuth a string through history

Uses git's pickaxe (`git log -S`) across all branches, then shows the matching
commit, the **file(s)** the change touched, and the **branches** that contain it.
Being pickaxe-based, it even finds matches inside binary/encrypted blobs.

```bash
slu search "api_key"             # this repo, all branches
slu search -r "password"         # recurse into every repo under the current dir
slu search -r -l 50 "secret"     # up to 50 commits per repo (default 20)
```

Matching is case-sensitive (pickaxe is precise by nature).

### `slu scan` — audit repos for leaked secrets

Sweeps every commit on every branch for a list of sensitive terms
(case-insensitive) and reports each hit with its commit, branch, and file —
including secrets committed in binary/encrypted files.

```bash
slu scan                              # scan repos under the current dir
slu scan /path/to/projects            # a specific path
slu scan -t "aws,bearer,token"        # custom terms (default: password,secret,token,…)
```

### `slu repos` — state of every repo at a glance

A dashboard of every repo under a path: current branch, uncommitted files, and
how far ahead/behind its upstream it is.

```bash
slu repos                # all repos under the current dir
slu repos --dirty        # only repos needing attention
```

Legend: `✚` uncommitted · `↑` unpushed commits · `↓` unpulled commits

### `slu sync` — fetch (and optionally fast-forward) every repo

Fetches and prunes all repos in parallel. With `--pull` it also runs
`git pull --ff-only`, which fast-forwards safely and refuses rather than merging
when it can't — so it never creates merge commits or conflicts.

```bash
slu sync                 # fetch + prune all repos
slu sync --pull          # also fast-forward the current branch where safe
```

### `slu tidy` — find finished branches to delete

Lists local branches whose **upstream is gone** — the remote branch they tracked
was deleted (e.g. a merged PR the remote auto-deleted) — across every repo, with
how long since each was last touched and a ready-to-paste delete command.
Branches still alive on the remote, or that never had an upstream, are left
alone.

```bash
slu tidy                 # repos with cleanup to do
slu tidy --all           # include repos with nothing to clean up
```

### `slu itidy` — interactively delete finished branches (TUI)

The interactive sibling of `slu tidy`, for the current repo: it lists the same
gone-upstream branches, and you delete them in place. Move with `j`/`k` (or
arrows), press `Enter` for a Yes/No confirm (defaults to **No**), toggle with
`←`/`→` (or `h`/`l`), and `Enter` again to act — `y`/`n` are shortcuts. Deletes
are `git branch -D` (force), since a squash/rebase-merged branch often isn't seen
as locally merged.

```bash
slu itidy                # tidy the current repo, interactively
```

### `slu each` — run any git command across all repos

The catch-all multi-repo power tool: whatever you'd type after `git`, run it in
every repo under you, in parallel.

```bash
slu each pull --ff-only
slu each switch main
slu each "log --oneline -1"
```

## The interactive views (`i*`) and their keys

`ilog`, `ibranch`, `istatus` and `itidy` are the interactive TUIs. They all share
one rule, so the keys never surprise you:

- **Plain keys drive the top pane** — `j`/`k` (or `↑`/`↓`) move, `h`/`l` (or
  `←`/`→`) move horizontally.
- **`Ctrl` + the same keys drive the diff** — `Ctrl-↑`/`Ctrl-↓` (or `Ctrl-j`/`Ctrl-k`)
  and `Ctrl-d`/`Ctrl-u` scroll it; `Ctrl-←`/`Ctrl-→` (or `Ctrl-h`/`Ctrl-l`) pan it
  sideways.
- **`Enter` opens / drills in** — and on a diff it hands the file to your
  configured **`git difftool`** (vimdiff, meld, VS Code…), suspending the TUI
  until you close it. If you have no `diff.tool` set, it says so instead.
- **`Esc`** (or `Ctrl-[`) steps back · **`q`** / `Ctrl-C` quit.

In `ilog` and `ibranch`, a yellow `↑` marks what isn't pushed to any remote (a
commit that exists nowhere remote, or a branch with no upstream / ahead / gone),
and `h`/`l` slides the scope: `ilog` between **local (unpushed) / all / pushed**,
`ibranch` between **local / all / remote**.

### `slu ilog` — interactive log explorer (TUI)

Opens a full-screen browser: scroll the commit list up top, see the selected
commit's diff below. The interactive sibling of `slu trace`.

Diffs are rendered side-by-side and syntax-highlighted in pure Rust (via
`syntect`) — with old/new line-number gutters and scrollbars, no external tools
required.

```bash
slu ilog                 # explore the current branch
slu ilog --all -n 500    # all branches, more history
slu ilog src/main.rs     # only commits touching that file
```

Give it a path (a file or a directory) and the log narrows to commits that
touched it, with each commit's file list narrowed to the same path.

Keys: `j`/`k` move commits · `Ctrl-↑`/`Ctrl-↓` select a file · `Enter` opens its
diff (and `Enter` again opens it in your difftool).

### `slu ibranch` — interactive branch explorer (TUI)

One level above `ilog`: pick a branch (sorted by recent activity, current one
marked), then drill into its commits → a commit's files → a file's side-by-side
diff, exactly like `ilog`.

```bash
slu ibranch              # start in the local scope
slu ibranch -r           # start in the remote scope
slu ibranch -a           # start in the all scope
```

Keys: `j`/`k` move the top pane, `Ctrl-↑`/`Ctrl-↓` the bottom one; `Enter` drills
in one level, `Esc` steps back.

### `slu istatus` — interactive git status (TUI)

`git status` you can act on. The changed files sit up top, the selected file's
diff below. Stage and unstage in place, and slide `h`/`l` (or `←`/`→`) to filter
the list between **staged**, **all**, and **unstaged**.

```bash
slu istatus              # the current repo (works from any subdirectory)
```

Keys: `j`/`k` move files · `h`/`l` change scope · `s` stage · `u` unstage ·
`Space` toggle · `r` refresh · `Enter` opens the file in your difftool
(comparing exactly what the pane shows — worktree, or `--cached` when staged).

The two-column code is git's own: **left = staged** (green), **right = unstaged**
(red). So `M ` is staged, ` M` is unstaged, `MM` is both, `??` is untracked.

### `slu update` — update sluuz itself

```bash
slu update               # cargo install sluuz --force, the easy way
```

### `slu completions` — tab-completion

Because `slu` is a git superset, its completion *reuses git's own* — so
`slu branch v<Tab>` completes a branch named `v0.2.1`, `slu checkout <Tab>` lists
refs, and every git subcommand, alias, and flag completes exactly like `git`.
slu's own verbs (`scan`, `ilog`, …) are added on top.

The easy way — `--add` writes the loader line into your shell's rc file and is
safe to run twice:

```bash
slu completions bash --add     # ~/.bashrc
slu completions zsh  --add     # ~/.zshrc
slu completions fish --add     # ~/.config/fish/config.fish
```

Then restart your shell. Prefer to wire it up yourself? Drop `--add` and
`slu completions <shell>` just prints the script for you to source:

```bash
source <(slu completions bash)      # bash / zsh
slu completions fish | source       # fish
```

This works in Git Bash and WSL on Windows too (both ship git's completion).

## Common options

The multi-repo commands accept a `path` argument (defaults to `.`) and
`-d, --depth <N>` — how many directory levels deep to look for repos (default 3).

## Troubleshooting

### `Ctrl-J` / `Ctrl-K` do nothing in some terminals

A few terminals — VS Code's integrated terminal among them — can't distinguish
`Ctrl-J` from Enter, which leaves the bottom-pane keys in `ilog`/`ibranch` dead.
slu accepts `Ctrl-Down` and `Ctrl-Up` for those same actions, so the fix is to
make the terminal send those instead.

In VS Code, add two keybindings (Command Palette → *Open Keyboard Shortcuts
(JSON)*): bind `ctrl+j` and `ctrl+k`, scoped to `"when": "terminalFocus"`, to the
`workbench.action.terminal.sendSequence` command. The sequence to send is the ESC
character (write it as the JSON escape backslash-u-001b) followed by `[1;5B` for
`Ctrl-Down` and `[1;5A` for `Ctrl-Up`.

## License

AGPL-3.0-only
