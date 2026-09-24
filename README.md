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

Then come the parts it does not. Each one below is an interactive tool and the
plain command that prints the same answer, for scripts and pipes.

## Every repo at once, and a way into them

![slu irepos drilling from a repository through its branches and commits into a diff, narrowing each list by typing a filter](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/repos.gif)

Legend: `✚` uncommitted · `↑` unpushed commits · `↓` unpulled commits

```bash
slu irepos               # every repo under here, with a way in; `s` syncs them all, `S` also pulls
slu irepos ~/projects    # or under somewhere else
slu repos                # the same list, printed
slu repos --dirty        # only the ones needing attention
```

![slu repos listing six repositories and their state, then only the dirty ones](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/repos-plain.png)

One app, five doors: `slu ibranch` opens it at the branches level, `slu itag`
at the tags level, `slu ilog` at the commits level and `slu istash` at the
stashes of the repo you are standing in.

## Find a leaked string in every repo's history

![slu iscan searching every repository's history for a password and browsing the hits with the diff of each](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/sleuth.gif)

```bash
slu iscan                             # type the terms, browse the hits
slu search -r "1337-let-me-in"        # one string, every repo, printed
slu scan                              # the usual terms, printed
slu scan -t "AKIA,BEGIN RSA PRIVATE KEY"
```

![slu scan's report over six repositories, ending in a summary of seven hits](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/sleuth-plain.png)

`notifications-worker` is the case a file listing cannot find: `.env` was
untracked afterwards, so the secret survives only in history. The `origin/` refs
say it survives on the server too.

Terms are case-insensitive and default to
`password,secret,token,api_key,passwd,credentials`; `-t` replaces that list.
Being pickaxe-based, all three also read binary and encrypted blobs.

## Read history in a real diff view

![slu ilog showing a history with a side-by-side diff below it, scrolled and panned](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/history.gif)

```bash
slu ilog                     # this branch
slu ilog --all -n 500        # every branch, more history
slu ilog src/main.rs         # only commits touching that path
slu trace [-a] [-g] [-n N]   # the aligned log, printed; every branch, with a graph
```

![slu ilog with the selected commit's diff side by side below it](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/ilog-diff.png)

![slu trace -a listing every branch's history in aligned columns](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/history-plain.png)

Highlighted in pure Rust, so no external diff tool is required - though `Enter`
hands the file to yours if you want it.

## Stage and review in one place

![slu istatus staging and unstaging a file with its diff below, and switching between its tabs](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/status.gif)

Legend: left column staged (green) · right column unstaged (red) · `MM` both ·
`??` untracked

```bash
slu istatus              # the two columns, with the diffs
slu status -sb           # real git, passed straight through
```

![slu status -sb and slu log as git prints them](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/status-plain.png)

`git status` you can act on, in git's own two-column code. `s` stages the file
under the cursor, `u` unstages it and `space` flips it; `S` and `U` do the same
to every file the list shows, filter included. The diff pane shows the side the
tab you are on is about. Works from any subdirectory.

## See what a stash holds

![slu istash listing two stashes with the files each one holds, opening their diffs, then asking for a stash's name before dropping it](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/stash.gif)

```bash
slu istash               # every stash, its files and their diffs
slu stash list           # real git, passed straight through
```

![slu stash list and slu stash show as git prints them](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/stash-plain.png)

No more `git stash show -p stash@{2}` to find out what you parked. The diff is
the stash against the commit it was made on, which is what `git stash show -p`
prints. `a` applies the stash under the cursor, `p` pops it (applies it, then
drops it) and `d` drops it, which asks for its name first, since its changes
are kept nowhere else. A pop that conflicts keeps the stash, as git always does.
Untracked files stashed with `-u` are not listed.

## Know what you have not pushed, and delete what is finished

![slu ibranch listing branches with their push state across its tabs, filtering them, asking for an unpushed branch's name before it would delete it, then deleting a finished one](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/branches.gif)

Legend: `↑N` ahead of upstream · `no remote` never pushed · `⚑ gone` upstream
was deleted · `synced` in step

```bash
slu ibranch [-r|-g]      # push state of your branches; `d` deletes, `s` syncs, `S` also pulls
                         # -r opens on the remote's, -g on the finished ones (upstream gone)
slu tidy [path] [-a]     # finished branches across every repo, with a delete command to paste
slu tidy -p              # the same, after dropping remote branches that are gone
```

![slu tidy -a reporting the finished branches across six repositories](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/branches-plain.png)

"Finished" means **upstream gone**, not "merged", so a branch still alive on the
remote is never suggested for deletion. A gone branch is only offered once its
changes are on the remote's main line, however they got there; one holding work
the remote does not have is listed apart and left for you to look at.

Git only marks a branch gone once the remote-tracking ref is really absent, and
a plain `git fetch` never removes one - so on a repo that does not prune, `tidy`
can be missing branches and says so. `tidy -p`, or `s` in `ibranch`, prunes
first; `git config --global fetch.prune true` fixes it for good.

## Know which tags the remote has

![slu itag marking three tags against the remote, asking for a pushed one's name before it would delete it everywhere, opening another's commits and a diff, and deleting that one](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/tags.gif)

Legend: `↑` not pushed · `⚑` the remote's tag is not the same as yours · `↓` only
the remote has it

```bash
slu itag                 # every tag against the remote, and what went into each
slu tag -n               # real git, passed straight through
```

![slu tag -n as git prints it](https://gitlab.com/safteinzz/sluuz/-/raw/main/readme-assets/tags-plain.png)

git keeps no record of which tags a remote has, so `slu itag` asks it with `git
ls-remote` once the list is up, and never stops to prompt for a password: a
remote that wants one reads as unreachable. Below each tag are the commits since
the tag before it on the same line of history, which is what went into that
release.

Branches do not have that wait because every clone has git keep a copy of the
remote's branches (`origin/main`). `t` offers the same for tags: one fetch rule
in this repo's `.git/config`, after which git keeps a copy of the remote's tags
on every fetch and push, and `slu itag` shows the marks at once while it checks
for anything newer. The copy lives in `refs/remote-tags/`, outside
`refs/remotes/`, so tags never show up as remote branches.

## Commands

```bash
slu sync [path] [--pull]       # fetch and prune every repo, optionally fast-forward
slu each <git args>            # run any git command in every repo, in parallel
```

Multi-repo commands take a `path` (default `.`) and `-d, --depth <N>` (default
3), and `slu <command> --help` lists any command's full flags.

## Keys

| key | does |
| --- | --- |
| `j` `k` / `↑` `↓` | move, faster the longer you hold it; with `Ctrl`, move the pane below or scroll a diff |
| `h` `l` / `←` `→` | switch tab; with `Ctrl`, pan a diff sideways |
| `/` `?` | filter the top pane / the pane below; every space-separated term has to match |
| `Enter` | open what is under the cursor |
| `Esc` | step back out |
| `r` | read it again from git, keeping the cursor on what it was on |
| `q` / `:q` | quit |

Each screen's own keys are on its bottom row, and `:help` lists every key it
answers to.

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

## Compatibility

Linux, macOS and Windows. Everything is passed through to your own `git`, so
anywhere git runs, `slu` runs.

## License

AGPL-3.0-only
