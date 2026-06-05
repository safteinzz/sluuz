# sluuz

Fast, colorized CLI for searching git history and managing many git repos at once.

`sluuz` walks every repository under a directory and works across **all branches**
in parallel — so you can hunt a string through history, audit for leaked secrets,
or check the state of 30 repos with a single command.

## Install

```bash
cargo install sluuz
```

Update to the latest version:

```bash
cargo install sluuz --force
```

## Commands

### `search` — find where a string entered or left history

Uses git's pickaxe (`git log -S`) across all branches, then shows the matching
commit, the **file(s)** the change touched, and the **branches** that contain it.
Because it's pickaxe-based, it also finds matches inside binary/encrypted blobs.

```bash
sluuz search "api_key"             # this repo, all branches
sluuz search -r "password"         # recurse into every repo under the current dir
sluuz search -r -l 50 "secret"     # show up to 50 commits per repo (default 20)
```

Matching is case-sensitive (pickaxe is precise by nature).

### `scan` — audit repos for leaked secrets

Sweeps every commit on every branch for a list of sensitive terms
(case-insensitive) and reports each hit with its commit, branch, and file.
Catches secrets committed in binary/encrypted files too.

```bash
sluuz scan                              # scan repos under the current dir
sluuz scan /path/to/projects            # scan a specific path
sluuz scan -t "aws,bearer,token"        # custom terms (default: password,secret,token,…)
sluuz scan -d 5                         # search up to 5 directory levels deep
```

### `status` — working-tree state across all repos

A dashboard of every repo under a path: current branch, uncommitted files,
and how far ahead/behind its upstream it is.

```bash
sluuz status                # all repos under the current dir
sluuz status --dirty        # only repos needing attention
```

Legend: `✚` uncommitted · `↑` unpushed commits · `↓` unpulled commits

### `fetch` — fetch (and optionally fast-forward) every repo

Fetches and prunes all repos in parallel. With `--pull` it additionally runs
`git pull --ff-only`, which fast-forwards safely and refuses rather than merging
when it can't — so it never creates merge commits or conflicts.

```bash
sluuz fetch                 # fetch + prune all repos
sluuz fetch --pull          # also fast-forward the current branch where safe
```

### `branches` — find merged, deletable branches

Lists local branches already merged into your current branch (safe to delete),
with how long since each was last touched and a ready-to-paste delete command.

```bash
sluuz branches              # repos with cleanup to do
sluuz branches --all        # include repos with nothing to clean up
```

## Common options

Most commands accept:

- a `path` argument (defaults to `.`)
- `-d, --depth <N>` — how many directory levels deep to look for repos (default 3)

## License

AGPL-3.0-only
