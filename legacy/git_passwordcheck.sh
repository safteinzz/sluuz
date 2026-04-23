#!/bin/bash
# repo-finder.sh
# Usage: ./repo-finder.sh [base_path]
# Searches all git repos under base_path for secret terms.
# Reports the branch and commit hash for each hit so you can scrub it.

# ─── CONFIG ───────────────────────────────────────────────────────────────────
SEARCH_TERMS="passwords|inbetween|pipes"
BASE_PATH="${1:-$(pwd)}"

# ─── COLORS ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

total_repos=0
repos_with_hits=0
total_hits=0

echo -e "\n${BOLD}🔍 REPO FINDER${RESET}"
echo -e "${DIM}base : $BASE_PATH${RESET}"
echo -e "${DIM}terms: $SEARCH_TERMS${RESET}"
echo -e "${DIM}$(date)${RESET}"
echo -e "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"

while IFS= read -r git_dir; do
    repo_dir="$(dirname "$git_dir")"
    repo_name="$(basename "$repo_dir")"
    total_repos=$((total_repos + 1))

    echo -e "${BOLD}📁 $repo_name${RESET}  ${DIM}$repo_dir${RESET}"

    # %H = full hash, %ad = author date (short)
    hits=$(
        git -C "$repo_dir" log --all -p \
            --format="COMMIT:%H|%ad" \
            --date=short \
            2>/dev/null \
        | awk -v pat="$SEARCH_TERMS" '
            /^COMMIT:/ {
                split($0, a, "|")
                hash = substr(a[1], 8)
                date = a[2]
                current_file = ""
                next
            }
            /^diff --git / {
                current_file = $0
                sub(/.*b\//, "", current_file)
                next
            }
            $0 ~ pat {
                key = hash SUBSEP current_file SUBSEP $0
                if (!seen[key]++) {
                    print hash "|" date "|" current_file "|" $0
                }
            }
        '
    )

    if [[ -z "$hits" ]]; then
        echo -e "   ${GREEN}✓ no matches${RESET}\n"
    else
        hit_count=$(echo "$hits" | wc -l | tr -d ' ')
        repos_with_hits=$((repos_with_hits + 1))
        total_hits=$((total_hits + hit_count))
        echo -e "   ${RED}${BOLD}⚠  $hit_count hit(s)${RESET}\n"

        echo "$hits" | while IFS='|' read -r hash date file line; do
            # look up every branch that contains this commit
            branches=$(git -C "$repo_dir" branch -a --contains "$hash" 2>/dev/null \
                | sed 's/^[* ]*//' | tr '\n' '  ')

            echo -e "   ${DIM}$date${RESET}  ${CYAN}${BOLD}$hash${RESET}"
            echo -e "   ${MAGENTA}branch │${RESET} $branches"
            echo -e "   ${DIM}file   │${RESET} $file"
            echo -e "   ${YELLOW}hit    │${RESET} $line"
            echo ""
        done
    fi

done < <(find "$BASE_PATH" -maxdepth 3 -name ".git" -type d | sort)

echo -e "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "${BOLD}SUMMARY${RESET}"
echo -e "  repos scanned  : ${BOLD}$total_repos${RESET}"
if [[ $repos_with_hits -gt 0 ]]; then
    echo -e "  repos with hits : ${RED}${BOLD}$repos_with_hits${RESET}"
    echo -e "  total hits      : ${RED}${BOLD}$total_hits${RESET}"
    echo -e "\n  ${DIM}to remove a commit from history:${RESET}"
    echo -e "  ${DIM}git rebase -i <hash>^${RESET}"
else
    echo -e "  ${GREEN}${BOLD}✓ all clean${RESET}"
fi
echo ""

