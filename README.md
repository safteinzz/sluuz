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

### `slu tidy` — find merged, deletable branches

Lists local branches already merged into your current branch (safe to delete),
with how long since each was last touched and a ready-to-paste delete command.

```bash
slu tidy                 # repos with cleanup to do
slu tidy --all           # include repos with nothing to clean up
```

### `slu each` — run any git command across all repos

The catch-all multi-repo power tool: whatever you'd type after `git`, run it in
every repo under you, in parallel.

```bash
slu each pull --ff-only
slu each switch main
slu each "log --oneline -1"
```

### `slu ilog` — interactive log explorer (TUI)

Opens a full-screen browser: scroll the commit list up top, see the selected
commit's diff below. The interactive sibling of `slu trace`. (The `i` prefix
marks the interactive views — more to come, like `ibranch`.)

If [`delta`](https://github.com/dandavison/delta) is installed it's used to
render the diff side-by-side and syntax-highlighted; otherwise a plain colorized
diff is shown.

```bash
slu ilog                 # explore the current branch
slu ilog --all -n 500    # all branches, more history
```

Keys: `j`/`k` (or arrows) move between commits · `Ctrl-j`/`Ctrl-k` scroll the diff a few lines · `Ctrl-d`/`Ctrl-u` half-page (vim) · `q` / `Esc` / `Ctrl-C` quit

### `slu ibranch` — interactive branch explorer (TUI)

One level above `ilog`: pick a branch (sorted by recent activity, current one
marked), then drill into its commits → a commit's files → a file's side-by-side
diff, exactly like `ilog`.

```bash
slu ibranch              # local branches
slu ibranch -r           # remote-tracking branches only (like git branch -r)
slu ibranch -a           # local and remote
```

Keys: `j`/`k` (or arrows) move the top pane, `Ctrl-j`/`Ctrl-k` the bottom pane;
`Enter` drills in, `Esc` (or `Ctrl-[`) steps back one level; `q` / `Ctrl-C` quit.

### `slu update` — update sluuz itself

```bash
slu update               # cargo install sluuz --force, the easy way
```

## Common options

The multi-repo commands accept a `path` argument (defaults to `.`) and
`-d, --depth <N>` — how many directory levels deep to look for repos (default 3).

## License

AGPL-3.0-only
