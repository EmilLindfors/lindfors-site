#!/usr/bin/env bash
# Open a pull request against This Week in Rust's current draft for one published post.
#
#   bash scripts/twir-pr.sh <slug> [section]
#
# Run by .github/workflows/twir.yml on the publisher's push when the commit carries the
# `Syndicate: this-week-in-rust` trailer, and by hand from the workstation for a
# back-catalogue post (gh must be logged in). It refuses a draft, since the link has
# to resolve when the editors look at it.
#
#   GH_TOKEN    a token that can push to the fork and open PRs on rust-lang (classic
#               PAT with public_repo); falls back to `gh auth token`
#   TWIR_FORK   owner/name of the fork, default <login>/this-week-in-rust
#   DRY_RUN=1   edit a clone of upstream, print the diff, push and open nothing
#
# Idempotent: a branch already on the fork, or the URL already in the draft, is a
# clean exit, not a second PR.
set -euo pipefail

slug="${1:?usage: twir-pr.sh <slug> [section]}"
section="${2:-Observations/Thoughts}"
upstream="rust-lang/this-week-in-rust"
site="https://lindfors.no"

root="$(cd "$(dirname "$0")/.." && pwd)"
index="$root/content/blog/$slug/index.md"
[ -f "$index" ] || { echo "no such post: content/blog/$slug/index.md" >&2; exit 1; }

# python3 on the runner and on Linux; the workstation's miniforge only has python.
py="$(command -v python3 || command -v python)" || { echo "python is required" >&2; exit 1; }

# Title, description and draft status out of the frontmatter, as shell assignments.
eval "$("$py" - "$index" <<'EOF'
import shlex, sys, tomllib
text = open(sys.argv[1], encoding="utf-8").read()
table = tomllib.loads(text.split("+++", 2)[1])
print("title=" + shlex.quote(table["title"]))
print("description=" + shlex.quote(table.get("description", "")))
print("is_draft=" + ("1" if table.get("draft") else "0"))
EOF
)"
if [ "$is_draft" = 1 ]; then
    echo "$slug is a draft; This Week in Rust needs a link that resolves" >&2
    exit 1
fi
url="$site/blog/$slug/"
branch="lindfors-$slug"

token="${GH_TOKEN:-$(gh auth token 2>/dev/null || true)}"
[ -n "$token" ] || { echo "no GitHub token: set GH_TOKEN or log in with gh" >&2; exit 1; }
export GH_TOKEN="$token"
login="$(gh api user --jq .login)"
fork="${TWIR_FORK:-$login/this-week-in-rust}"
base="$(gh api "repos/$upstream" --jq .default_branch)"   # main, as of 2026-09
dry="${DRY_RUN:-0}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [ "$dry" = 1 ]; then
    echo "dry run: editing a clone of $upstream, pushing nothing"
    git -c core.autocrlf=false clone --quiet --filter=blob:none --branch "$base" "https://github.com/$upstream.git" "$tmp/twir"
else
    # The fork, current with upstream, so the branch starts from the draft the editors
    # are working on. `fork` is a no-op when it exists; `sync --force` never loses
    # anything because nobody edits the fork's default branch.
    gh repo fork "$upstream" --clone=false >/dev/null 2>&1 || true
    gh repo sync "$fork" --source "$upstream" --branch "$base" --force >/dev/null
    if gh api "repos/$fork/branches/$branch" >/dev/null 2>&1; then
        echo "$fork already has branch $branch; the PR exists or was closed. Nothing to do."
        exit 0
    fi
    git -c core.autocrlf=false clone --quiet --filter=blob:none --branch "$base" "https://github.com/$fork.git" "$tmp/twir"
fi
cd "$tmp/twir"

# Between Wednesday's publish and the next draft there is no file; try again later.
drafts=(draft/*.md)
if [ "${#drafts[@]}" -ne 1 ] || [ ! -f "${drafts[0]}" ]; then
    echo "expected exactly one draft/*.md in $upstream, found: ${drafts[*]:-none}. Re-run later." >&2
    exit 1
fi
draft="${drafts[0]}"

set +e
"$py" "$root/scripts/twir-draft-insert.py" "$draft" "$title" "$url" "$section"
rc=$?
set -e
case "$rc" in
    0) ;;
    3) echo "Nothing to do."; exit 0 ;;
    *) exit "$rc" ;;
esac

git checkout --quiet -b "$branch"
git -c user.name="Emil Lindfors" -c user.email="emil@lindfors.no" commit --quiet -am "Add: $title"

if [ "$dry" = 1 ]; then
    git --no-pager show --stat --format='%s' HEAD
    git --no-pager diff HEAD~1 -- "$draft"
    exit 0
fi

git push --quiet "https://x-access-token:${token}@github.com/$fork.git" "HEAD:$branch"
body="$url

$description

Author submission; feel free to move it to another section."
gh pr create --repo "$upstream" --base "$base" --head "${fork%%/*}:$branch" \
    --title "Blog post: $title" --body "$body"
