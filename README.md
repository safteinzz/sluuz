# sluuz (`slu`)

> **Canonical:** [gitlab.com/safteinzz/sluuz](https://gitlab.com/safteinzz/sluuz) · **Mirror:** [github.com/safteinzz/sluuz](https://github.com/safteinzz/sluuz)

<!-- desc:start -->
git, but it sleuths - a git superset with cross-repo search, secret scanning, and multi-repo management
<!-- desc:end -->

## Install

```bash
cargo install sluuz
slu self check   # is a newer release out?
slu self update  # install the latest
```

No cargo yet? Rust installs the same way on every distro: [rustup.rs](https://rustup.rs).

## It is still git

Anything git understands is forwarded verbatim, with your editor, pager,
prompts, colors and exit codes intact:

```bash
slu commit -m "fix"
slu push
slu rebase -i HEAD~3
```

Then come the parts it does not. Each one below is a plain command and its
interactive twin, because they answer the same question.

## Every repo at once, and a way into them

Legend: `✚` uncommitted · `↑` unpushed commits · `↓` unpulled commits

```bash
slu repos                # every repo under here
slu repos --dirty        # only the ones needing attention
slu irepos               # the same list, with a way in
slu irepos ~/projects    # or under somewhere else
```

![slu repos listing six repositories with their branch and state, then the same list filtered to the dirty ones, then slu irepos using the filter to navigate instead of the arrow keys - / proxy narrowing six repositories to edge-proxy, / feat narrowing its branches to two, ? Marek narrowing the commits under those to that author's four - then ctrl-down picking one of them and drilling into its diff, before walking back out with Esc](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/repos.gif)

One app, three doors: `slu ibranch` opens it at the branches level and
`slu ilog` at the commits level of the repo you are standing in.

Plain keys drive the list you are on, `Ctrl` the preview under it: `j`/`k` move
and `PageUp`/`PageDown` page the top pane, `Ctrl-j`/`Ctrl-k` move the preview and
`Ctrl-d`/`Ctrl-u` half-page it, `h`/`l` slide the scope, `Enter` drills in and
`Esc` steps back out. `/` filters the top pane and `?` the one below it: terms
are separated by spaces and all of them have to appear somewhere in a row, so
`pablo fix` finds what both words are in, whether that is a sha, an author, a
date or a subject. Lists load in the background and keep filtering as they
arrive, so nothing you press ever waits on git.

## Find a leaked string in every repo's history

```bash
slu search -r "1337-let-me-in"        # one string, every repo
slu scan                              # the usual terms
slu scan -t "AKIA,BEGIN RSA PRIVATE KEY"
slu iscan                             # type the terms, browse the hits
```

![slu search finding a password across three repositories, then slu scan sweeping six of them for the default secret terms, then slu iscan running the same hunt from a query bar with the diff of each hit below](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/sleuth.gif)

`notifications-worker` is the case a file listing cannot find: `.env` was
untracked afterwards, so the secret survives only in history. The `origin/` refs
say it survives on the server too.

Terms are case-insensitive and default to
`password,secret,token,api_key,passwd,credentials`; `-t` replaces that list.
Being pickaxe-based, all three also read binary and encrypted blobs.

![Scan report over six repositories, three of them clean, ending in a summary of seven hits and the two ways to remove a secret from history](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/scan.png)

## Read history in a real diff view

Highlighted in pure Rust, so no external diff tool is required - though `Enter`
hands the file to yours if you want it.

```bash
slu trace [-a] [-g] [-n N]   # aligned log, optionally every branch, with a graph
slu ilog                     # this branch
slu ilog --all -n 500        # every branch, more history
slu ilog src/main.rs         # only commits touching that path
```

![slu trace listing a repository's history in aligned columns, then slu ilog opening the same history with a side-by-side diff below it, scrolling down through the diff and panning it sideways](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/history.gif)

![Interactive log with nine commits on top and the selected commit's diff below, old and new side by side with line-number gutters and red and green change tinting](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/ilog-diff.png)

## Stage and review in one place

`git status` you can act on. The two-column code is git's own: **left is
staged** (green), **right is unstaged** (red), so `M ` is staged, ` M` is
unstaged, `MM` is both and `??` is untracked.

```bash
slu status -sb           # real git, passed straight through
slu istatus              # the same two columns, with the diffs and the keys
```

![slu status -sb and slu log printing real git output, then slu istatus showing the same four files with their diff below, staging one with s so its marker moves to the staged column, unstaging it again with u, and sliding the list between staged, all and unstaged](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/status.gif)

The diff pane follows the scope. Works from any subdirectory.

## Know what you have not pushed, and delete what is finished

Legend: `↑N` ahead of upstream · `no remote` never pushed · `⚑ gone` upstream
was deleted · `synced` in step

```bash
slu ibranch [-a]         # push state of every branch, local or local + remote
slu tidy [path] [-a]     # finished branches across every repo, with a delete command to paste
slu tidy -p              # the same, after dropping remote branches that are gone
slu itidy                # the same list, deleted in place (`p` prunes and reloads)
```

![slu ibranch listing eight branches with their push state, sliding between local, all and remote scope, then narrowing the branch list to the two fix branches by typing / fix and the commits under it to the two health ones with ? health, then slu tidy reporting the finished branches across every repo, then slu itidy deleting one through a confirmation dialog that opens on No](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/branches.gif)

"Finished" means **upstream gone**, not "merged", so a branch still alive on the
remote is never suggested for deletion, and the confirm defaults to **No**.

Git only marks a branch gone once the remote-tracking ref is really absent, and
a plain `git fetch` never removes one - so on a repo that does not prune, this
list can be missing branches and says so. `-p` prunes first; `git config --global
fetch.prune true` fixes it for good.

## Tab completion

Completion reuses git's own, so `slu branch v<Tab>` completes a real branch name
and every git subcommand, alias and flag behaves exactly like `git`.

```bash
slu completions bash --add     # writes the loader into ~/.bashrc
slu completions zsh  --add     # ~/.zshrc
slu completions fish --add     # ~/.config/fish/config.fish
```

Restart your shell afterwards; without `--add` it just prints the script. Works
in Git Bash and WSL too.

## Commands

```bash
slu sync [path] [--pull]       # fetch and prune every repo, optionally fast-forward
slu each <git args>            # run any git command in every repo, in parallel
```

Multi-repo commands take a `path` (default `.`) and `-d, --depth <N>` for how
deep to look (default 3). Run `slu <command> --help` for the full flag surface
of any command.

## Compatibility

Linux, macOS and Windows. Everything is passed through to your own `git`, so
anywhere git runs, `slu` runs.

## License

AGPL-3.0-only
