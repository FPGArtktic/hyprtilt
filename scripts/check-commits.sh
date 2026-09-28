#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Check the commit messages of BASE..HEAD against the rules in CONTRIBUTING.md:
#
#   * subject "area: imperative description", known area, at most 72
#     characters, no trailing period, followed by an empty line;
#   * body lines at most 75 columns, except trailers and URLs;
#   * a Signed-off-by trailer of the commit author (DCO);
#   * no merge commits (history is rebased).
#
# Usage: scripts/check-commits.sh [BASE]   (default: origin/main)
# A BASE that does not exist (first push) checks every commit of HEAD.

set -euo pipefail

base="${1:-origin/main}"
areas='core|geometry|lua|hyprlang|ipc|apply|tui|cli|docs|ci|pkg|build'

if git rev-parse --verify --quiet "${base}^{commit}" >/dev/null; then
    range="${base}..HEAD"
else
    range="HEAD"
fi

status=0
fail() {
    printf '%s: %s\n' "$1" "$2" >&2
    status=1
}

commits="$(git rev-list --reverse "$range")"
if [[ -z "$commits" ]]; then
    echo "no commits in $range"
    exit 0
fi

for c in $commits; do
    short="$(git rev-parse --short=12 "$c")"
    subject="$(git log -1 --format=%s "$c")"
    author="$(git log -1 --format='%an <%ae>' "$c")"
    message="$(git log -1 --format=%B "$c")"

    if [[ "$(git rev-list --parents -n 1 "$c" | wc -w)" -gt 2 ]]; then
        fail "$short" "merge commit; rebase instead"
    fi
    if ! [[ "$subject" =~ ^($areas):\ [^[:space:]] ]]; then
        fail "$short" "subject must be 'area: description' with area one of ${areas//|/, }: $subject"
    fi
    if [[ ${#subject} -gt 72 ]]; then
        fail "$short" "subject longer than 72 characters (${#subject})"
    fi
    if [[ "$subject" == *. ]]; then
        fail "$short" "subject ends with a period"
    fi
    if [[ -n "$(sed -n 2p <<<"$message")" ]]; then
        fail "$short" "second line must be empty"
    fi
    if ! grep -qxF "Signed-off-by: $author" <<<"$message"; then
        fail "$short" "missing 'Signed-off-by: $author' (use git commit -s)"
    fi
    while IFS= read -r line; do
        [[ ${#line} -le 75 ]] && continue
        [[ "$line" =~ ^[A-Za-z-]+:\  ]] && continue # trailers (Fixes:, Link:, ...)
        [[ "$line" =~ https?:// ]] && continue
        fail "$short" "body line longer than 75 columns: $line"
    done < <(sed -n '3,$p' <<<"$message")
done

if [[ $status -eq 0 ]]; then
    echo "commit messages OK ($(wc -w <<<"$commits") commits in $range)"
fi
exit "$status"
