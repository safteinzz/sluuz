#!/bin/bash

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
BOLD='\033[1m'
RESET='\033[0m'

RECURSIVE=false
LIMIT=20

usage() {
    echo -e "Usage: git_search.sh [-r] [-l N] \"pattern\""
    echo -e "  -r      search all git repos recursively in current dir"
    echo -e "  -l N    max commits to show per repo (default: 20)"
    exit 1
}

while getopts "rl:" opt; do
    case $opt in
        r) RECURSIVE=true ;;
        l) LIMIT="$OPTARG" ;;
        *) usage ;;
    esac
done
shift $((OPTIND - 1))

PATTERN="$1"
[ -z "$PATTERN" ] && usage

export PATTERN LIMIT RED GREEN YELLOW CYAN GRAY BOLD RESET

echo -e "${GRAY}⏳ Searching for:${RESET} ${BOLD}$PATTERN${RESET}\n"

search_repo() {
    local repo="$1"
    local name
    name=$(basename "$(realpath "$repo")")

    local results
    results=$(LC_ALL=C git -C "$repo" log --all -S "$PATTERN" --pickaxe-all -F \
        --pretty=format:"COMMIT:%h %s" -p 2>/dev/null \
        | grep -F -e "COMMIT:" -e "$PATTERN")

    [ -z "$results" ] && return

    local count
    count=$(echo "$results" | grep -c "^COMMIT:")
    [ "$count" -eq 0 ] && return

    local output="${BOLD}${CYAN}━━━ $name${RESET} ${GRAY}${count} commit(s)${RESET}"
    local shown=0

    while IFS= read -r line; do
        if [[ "$line" == COMMIT:* ]]; then
            [ "$shown" -ge "$LIMIT" ] && break
            local hash msg
            hash=$(echo "$line" | awk '{print $1}' | cut -c8-)
            msg=$(echo "$line" | cut -d' ' -f2-)
            output+="\n\n  ${YELLOW}▸ $hash${RESET} $msg"
            ((shown++))
        elif [[ "$line" == -* ]]; then
            output+="\n    ${RED}$line${RESET}"
        elif [[ "$line" == +* ]]; then
            output+="\n    ${GREEN}$line${RESET}"
        fi
    done <<< "$results"

    if [ "$count" -gt "$LIMIT" ]; then
        output+="\n\n  ${GRAY}↳ $((count - LIMIT)) more — use -l $count to see all${RESET}"
    fi

    echo -e "$output\n"
}

export -f search_repo

if [ "$RECURSIVE" = true ]; then
    find . -name ".git" -type d 2>/dev/null \
        | sed 's|/.git||' \
        | sort \
        | xargs -P "$(nproc)" -I {} bash -c 'search_repo "$@"' _ {}
else
    search_repo "."
fi
