# sluuz

> **Canonical:** [gitlab.com/safteinzz/sluuz](https://gitlab.com/safteinzz/sluuz) · **Mirror:** [github.com/safteinzz/sluuz](https://github.com/safteinzz/sluuz)

**git, but it sleuths.** 🕵️

`sluuz` is a drop-in **git superset**. You type `slu` instead of `git` and
everything works exactly as before, because anything git understands is passed
straight through. On top of that it adds what git makes hard: searching history
across every repo you own, auditing for leaked secrets, and interactive views of
your log, branches, and working tree.

## Install

```bash
cargo install sluuz
```

That lands a single command: **`slu`**, three letters like `git` itself. Update
later with `slu update`.

## It is still git

Forwarded verbatim, with your editor, pager, prompts, colors, and exit codes
intact:

```bash
slu commit -m "fix"
slu push
slu rebase -i HEAD~3
```

Then come the parts git does not give you.

## Find a string across every repo at once

git's pickaxe pointed at every branch of every repo under you. Each hit names the
commit, the branches carrying it, the file, and the line that changed.

```bash
slu search -r "1337-let-me-in"
```

![Search results for a leaked password, grouped by repository, showing the commit, branches, file and matching line for five commits across three repos](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/search.png)

One command, three repos. `billing-api` shows the password reached `main` through
`hotfix/staging-db`. `notifications-worker` shows the classic mistake: `.env` was
untracked afterwards, so the secret now lives only in history, which is exactly
where a file listing will not find it.

Being pickaxe-based, it also finds matches inside binary and encrypted blobs.

## Audit history for leaked secrets

Sweeps every commit on every branch for sensitive terms and reports each hit with
its commit, branch, and file. Terms are case-insensitive, and `-t` replaces the
built-in list with your own.

```bash
slu scan                              # default terms
slu scan -t "token,password"          # your own list
slu scan -t "AKIA,BEGIN RSA PRIVATE KEY"
```

![Scan report over three repositories listing eight hits with commit, branch, file and matching lines, ending in a summary and history-rewriting advice](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/scan.png)

The summary closes with the two ways to actually remove a secret, because finding
it is the easy half.

## Or do both interactively

`slu iscan` puts the query inside the tool. Type terms into the bar, press
`Enter`, and it pickaxes them across every repo under you; the diff of the commit
behind the selected hit sits right below, so "what actually changed there?" never
costs you a second command.

```bash
slu iscan
```

![Interactive scanner with a query bar reading password, six hits listed across three repositories, and the side-by-side diff of the selected hit below](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/iscan.png)

The bar starts empty with the usual secret terms as a placeholder, so `Enter` on
an empty bar runs the standard audit and anything you type replaces it. Give it
several comma-separated terms and `h`/`l` narrows the results to one of them
without scanning again. `Enter` on a hit opens it in your difftool.

## See every repo at a glance

Legend: `✚` uncommitted · `↑` unpushed commits · `↓` unpulled commits

```bash
slu repos                # every repo under here
slu repos --dirty        # only the ones needing attention
```

![Dashboard listing six repositories with their branch and state: three clean, two with uncommitted files, one with three unpushed commits](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/repos.png)

Which of your repos have uncommitted or unpushed work, answered in one command
instead of cd-ing through each.

## Read history in a real diff view

`slu ilog` opens a full-screen browser: commits on top, the selected file's diff
below, rendered side by side and syntax-highlighted in pure Rust. No external
diff tool required, though `Enter` hands the file to yours if you want it.

```bash
slu ilog                 # this branch
slu ilog --all -n 500    # every branch, more history
slu ilog src/main.rs     # only commits touching that path
```

![Interactive log with a commit list on top and a side-by-side syntax-highlighted diff below, showing old and new line numbers and red and green change tinting](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/ilog-diff.png)

Old and new line-number gutters, both scrollbars, and horizontal panning for long
lines. Give it a path and the log narrows to commits that touched it, with each
commit's file list narrowed the same way.

## Stage and review in one place

`git status` you can act on. Files on top, the selected file's diff below, and
`h`/`l` slides the list between **staged**, **all**, and **unstaged**.

The two-column code is git's own: **left is staged** (green), **right is
unstaged** (red). So `M ` is staged, ` M` is unstaged, `MM` is both, `??` is
untracked.

```bash
slu istatus
```

![Interactive status showing four changed files with staged and unstaged columns, and the staged diff of Cargo.toml side by side below](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/istatus.png)

`s` stages, `u` unstages, `Space` toggles. The diff pane follows the scope, so a
staged file shows its staged diff. Works from any subdirectory.

## Know what you have not pushed

Legend: `↑N` ahead of upstream · `no remote` never pushed · `⚑ gone` upstream was
deleted · `synced` in step

```bash
slu ibranch              # local scope
slu ibranch -a           # local and remote
```

![Interactive branch list showing five branches with their push state: gone, ahead by one, no remote, and synced, with the current branch marked](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/ibranch.png)

Drill into a branch for its commits, then a commit's files, then a diff. `h`/`l`
slides between **local**, **all**, and **remote**. In `ilog` the same `↑` marks
commits that exist on no remote at all.

## Delete finished branches, carefully

`slu itidy` lists local branches whose upstream was deleted, which is what
"finished" actually means, and deletes them in place.

```bash
slu itidy
```

![Interactive branch tidier with a confirmation dialog over the list, asking to delete a branch, with No selected by default](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/itidy.png)

The confirm defaults to **No**, so a double `Enter` cancels instead of deleting.
`←`/`→` toggles, `y`/`n` are shortcuts. A branch still alive on the remote is
never listed.

## One rule for the interactive views

`ilog`, `ibranch`, `istatus`, `itidy` and `iscan` share it, so the keys never
surprise you:

- **Plain keys drive the top pane.** `j`/`k` (or `↑`/`↓`) move, `h`/`l` (or
  `←`/`→`) change scope.
- **`Ctrl` and the same keys drive the diff.** `Ctrl-↑`/`Ctrl-↓` and
  `Ctrl-d`/`Ctrl-u` scroll it, `Ctrl-←`/`Ctrl-→` pan it sideways.
- **`Enter` opens or drills in.** On a diff it hands the file to your configured
  `git difftool` and suspends until you close it. With no `diff.tool` set it says
  so rather than tearing down the screen.
- **`Esc`** (or `Ctrl-[`) steps back, **`q`** or `Ctrl-C` quits.

## Tab completion

Because `slu` is a git superset, its completion reuses git's own: `slu branch
v<Tab>` completes a real branch name, and every git subcommand, alias, and flag
completes exactly like `git`. slu's own verbs are added on top.

```bash
slu completions bash --add     # writes the loader into ~/.bashrc
slu completions zsh  --add     # ~/.zshrc
slu completions fish --add     # ~/.config/fish/config.fish
```

Restart your shell afterwards. Drop `--add` and it just prints the script for you
to source yourself. Works in Git Bash and WSL on Windows too.

## Everything else

```bash
slu trace [-a] [-g] [-n N]     # a prettier, aligned history view
slu sync [path] [--pull]       # fetch and prune every repo, optionally fast-forward
slu tidy [path] [-a]           # itidy's list across every repo, with a delete command to paste
slu each <git args>            # run any git command in every repo, in parallel
slu update [-y]                # update sluuz itself
```

`tidy` and `itidy` judge by "upstream gone", not by "merged", so a branch still
alive on the remote is never suggested for deletion. Multi-repo commands take a
`path` (default `.`) and `-d, --depth <N>` for how deep to look (default 3).

Run `slu <command> --help` for the full flag surface of any command.

## Troubleshooting

**`Ctrl-J` / `Ctrl-K` do nothing in some terminals.** A few terminals, VS Code's
integrated one among them, cannot tell `Ctrl-J` from Enter, which leaves the
bottom-pane keys dead. slu accepts `Ctrl-Down` and `Ctrl-Up` for the same
actions, so the fix is to make the terminal send those instead. In VS Code, open
*Keyboard Shortcuts (JSON)* and bind `ctrl+j` and `ctrl+k`, scoped to
`"when": "terminalFocus"`, to `workbench.action.terminal.sendSequence`. The
sequence is the ESC character, written as the JSON escape backslash-u-001b,
followed by `[1;5B` for `Ctrl-Down` and `[1;5A` for `Ctrl-Up`.

## License

AGPL-3.0-only
