# sluuz

CLI tools for searching and managing git repositories.

## Install

```bash
cargo install sluuz
```

## Commands

### search

Search git history for commits that added or removed a string.

```bash
sluuz search "api_key"
sluuz search -r "password"        # recursive across all repos under current dir
sluuz search -r -l 50 "secret"   # show up to 50 commits per repo
```

### scan

Scan repositories for sensitive terms across all branches and commits.

```bash
sluuz scan                              # scan current directory
sluuz scan /path/to/projects            # scan a specific path
sluuz scan -t "aws,bearer,token" .      # custom terms
```

## License

AGPL-3.0-only
