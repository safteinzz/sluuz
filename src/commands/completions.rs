//! `slu completions <shell>` - print a tab-completion script for your shell.
//!
//! slu is a git superset, so the smart move is to *reuse git's own completion*
//! rather than reinvent it: git's completion already knows your branches, tags,
//! remotes, every subcommand and every flag, and it queries the repo live. Each
//! script here delegates to git's completion (so `slu branch v<Tab>` completes a
//! branch named `v0.2.1`, exactly like `git branch v<Tab>`) and layers slu's own
//! superpower verbs on top.
//!
//! This is the same approach `hub` (the canonical git wrapper) uses; clap's own
//! completion generators can't do it, because the branch lives behind the git
//! passthrough that clap never sees.
//!
//! Install (pick your shell):
//!     bash:  echo 'source <(slu completions bash)'  >> ~/.bashrc
//!     zsh:   echo 'source <(slu completions zsh)'   >> ~/.zshrc
//!     fish:  echo 'slu completions fish | source'   >> ~/.config/fish/config.fish

/// slu's own verbs (everything that isn't git passthrough). Single source of
/// truth, shared by all three scripts.
const VERBS: &[(&str, &str)] = &[
    ("search", "pickaxe a string through history"),
    ("scan", "audit repos for leaked secrets"),
    ("repos", "working-tree state across all repos"),
    ("sync", "fetch (and optionally fast-forward) all repos"),
    ("tidy", "find merged, safe-to-delete branches"),
    ("each", "run a git command in every repo"),
    ("trace", "a prettier history view"),
    ("irepos", "interactive repo explorer (TUI)"),
    ("ibranch", "interactive branch explorer (TUI)"),
    ("ilog", "interactive log explorer (TUI)"),
    ("iscan", "interactive history search (TUI)"),
    ("istatus", "interactive status, stage and review (TUI)"),
    ("itidy", "interactively delete finished branches (TUI)"),
    ("update", "update slu itself"),
    ("completions", "print a shell completion script"),
];

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    /// The completion script for this shell.
    fn script(self) -> String {
        match self {
            Shell::Bash => bash(),
            Shell::Zsh => zsh(),
            Shell::Fish => fish(),
        }
    }

    /// The rc file this shell sources on startup, relative to $HOME.
    fn rc_file(self) -> &'static str {
        match self {
            Shell::Bash => ".bashrc",
            Shell::Zsh => ".zshrc",
            Shell::Fish => ".config/fish/config.fish",
        }
    }

    /// The one line that, in that rc file, loads slu's completion every startup.
    fn rc_line(self) -> &'static str {
        match self {
            Shell::Bash => "source <(slu completions bash)",
            Shell::Zsh => "source <(slu completions zsh)",
            Shell::Fish => "slu completions fish | source",
        }
    }
}

#[derive(clap::Args)]
pub struct Args {
    /// Which shell to print a completion script for
    #[arg(value_enum)]
    pub shell: Shell,

    /// Instead of printing, add the loader line to your shell's rc file
    #[arg(long)]
    pub add: bool,
}

pub fn run(args: Args) {
    if args.add {
        install(args.shell);
    } else {
        print!("{}", args.shell.script());
    }
}

/// Append the loader line to the shell's rc file (idempotently) so completion
/// turns on automatically in every new shell.
fn install(shell: Shell) {
    use colored::Colorize;

    let Some(home) = home_dir() else {
        eprintln!("{}", "slu: could not find your home directory".red());
        std::process::exit(1);
    };
    let rc = home.join(shell.rc_file());
    let line = shell.rc_line();

    // Already there? Don't add it twice.
    if let Ok(existing) = std::fs::read_to_string(&rc)
        && existing.lines().any(|l| l.trim() == line)
    {
        println!(
            "{} {}",
            "✓ already set up in".dimmed(),
            rc.display().to_string().bold()
        );
        return;
    }

    // fish's config lives under ~/.config/fish - make sure the dir exists.
    if let Some(parent) = rc.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("{} {e}", "slu: could not create".red());
        std::process::exit(1);
    }

    let block = format!("\n# sluuz shell completion\n{line}\n");
    match append(&rc, &block) {
        Ok(()) => {
            println!(
                "{} {}",
                "✓ added slu completion to".green(),
                rc.display().to_string().bold()
            );
            println!(
                "{}",
                "  restart your shell (or re-source the file) to use it.".dimmed()
            );
        }
        Err(e) => {
            eprintln!("{} {} - {e}", "slu: could not write".red(), rc.display());
            std::process::exit(1);
        }
    }
}

fn append(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(text.as_bytes())
}

/// `$HOME`, falling back to `%USERPROFILE%` on Windows (Git Bash sets HOME).
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

fn verb_names() -> String {
    VERBS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bash: delegate to git's completion (`__git_main`), then append slu's verbs
/// when completing the first word. Requires git's bash completion to be loaded
/// (the `bash-completion` package, or sourcing `git-completion.bash`).
fn bash() -> String {
    format!(
        r#"# slu bash completion - `slu` is a git superset, so this reuses git's own
# completion (branches, tags, remotes, every git subcommand and flag) and adds
# slu's verbs on top.
#   add to ~/.bashrc:  source <(slu completions bash)

# git's bash completion is normally lazy-loaded - it only kicks in the first time
# you complete `git` itself. We complete `slu`, which never triggers that, so
# `__git_main` would never exist and there'd be nothing to delegate to. Force it
# to load up front: ask bash-completion's loader first, then fall back to known
# install paths.
if ! declare -F __git_main >/dev/null 2>&1; then
    declare -F _completion_loader >/dev/null 2>&1 && _completion_loader git >/dev/null 2>&1
fi
if ! declare -F __git_main >/dev/null 2>&1; then
    for __slu_gitc in \
        /usr/share/bash-completion/completions/git \
        /etc/bash_completion.d/git \
        "$HOME/.local/share/bash-completion/completions/git"; do
        [ -r "$__slu_gitc" ] && . "$__slu_gitc" && break
    done
    unset __slu_gitc
fi

# Let git set up its proper completion wrapper for `slu` (handles : and = in
# words correctly); harmless if git's completion still isn't loaded.
if declare -F __git_complete >/dev/null 2>&1; then
    __git_complete slu __git_main >/dev/null 2>&1
fi

_slu() {{
    # Everything git knows: branches, refs, subcommands, flags.
    if declare -F __git_wrap__git_main >/dev/null 2>&1; then
        __git_wrap__git_main
    elif declare -F __git_main >/dev/null 2>&1; then
        __git_main
    fi
    # slu's own verbs, only at the first argument.
    if [ "${{COMP_CWORD:-0}}" -eq 1 ]; then
        COMPREPLY+=( $(compgen -W '{verbs}' -- "${{COMP_WORDS[COMP_CWORD]}}") )
    fi
}}

complete -o bashdefault -o default -o nospace -F _slu slu 2>/dev/null \
    || complete -o default -o nospace -F _slu slu
"#,
        verbs = verb_names()
    )
}

/// Zsh: delegate to zsh's bundled `_git` (forcing the git service), then offer
/// slu's verbs at the first argument via `_describe`.
fn zsh() -> String {
    let descs = VERBS
        .iter()
        .map(|(name, desc)| format!("        '{name}:{desc}'"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"#compdef slu
# slu zsh completion - `slu` is a git superset, so this reuses zsh's bundled
# git completion (branches, tags, remotes, flags) and adds slu's verbs on top.
#   add to ~/.zshrc:  source <(slu completions zsh)

_slu() {{
    # slu's own verbs, only at the first argument.
    if (( CURRENT == 2 )); then
        local -a slu_cmds
        slu_cmds=(
{descs}
        )
        _describe -t slu-commands 'slu command' slu_cmds
    fi
    # Everything git knows: complete as if we were git.
    local service=git
    _git
}}

compdef _slu slu
"#
    )
}

/// Fish: `complete --wraps git` makes slu inherit *all* of git's completions
/// (including live branch names); then we add slu's verbs at the first argument.
fn fish() -> String {
    let verbs = VERBS
        .iter()
        .map(|(name, desc)| {
            format!("complete -c slu -n __fish_use_subcommand -a {name} -d '{desc}'")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"# slu fish completion - `slu` is a git superset, so it inherits all of git's
# completions (branches, tags, remotes, flags) and adds slu's verbs on top.
#   add to ~/.config/fish/config.fish:  slu completions fish | source

complete -c slu -w git
{verbs}
"#
    )
}
