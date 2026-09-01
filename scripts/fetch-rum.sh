#!/bin/bash
# Vendor the OpenObserve browser-rum bundle into static/js/.
#
# The published integration is an npm one, but this repo has no package.json and no
# bundler -- build.sh is shell plus site-tools -- so the SDK arrives as the prebuilt
# IIFE that ships in the same package. It defines window.OO_RUM and nothing else;
# static/js/rum.js does the initialising.
#
# The result is committed. Cloudflare Pages builds from git and runs no npm install,
# so an uncommitted bundle is a 404 on every page, and self-hosting is the same choice
# the fonts made: no third-party fetch on an ordinary page load.
#
# Usage: ./scripts/fetch-rum.sh   (re-run after bumping VERSION and SHA)

set -e

VERSION="0.4.1"
SHA="501517961466676378381cc9cf0ccaa00cb4b6d5e4ae39a0e123da6e48781b0f"
URL="https://cdn.jsdelivr.net/npm/@openobserve/browser-rum@${VERSION}/bundle/openobserve-rum.js"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DEST="$PROJECT_DIR/static/js/openobserve-rum.js"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

echo "Fetching @openobserve/browser-rum ${VERSION}"
curl -fsSL -o "$tmp" "$URL"

got="$(sha256sum "$tmp" | awk '{print $1}')"
if [ "$got" != "$SHA" ]; then
    echo "Error: checksum mismatch for $URL" >&2
    echo "       expected $SHA" >&2
    echo "       got      $got" >&2
    exit 1
fi

# A banner, because the file is minified and otherwise unattributable in a diff.
{
    echo "/*! @openobserve/browser-rum ${VERSION} -- vendored, do not edit."
    echo " *  Source: ${URL}"
    echo " *  Refresh: ./scripts/fetch-rum.sh (bump VERSION and SHA first) */"
    cat "$tmp"
} > "$DEST"

echo "Wrote $DEST ($(wc -c < "$DEST") bytes)"

# Session replay and the profiler are the only features that pull further files, and
# they arrive as lazily imported chunks resolved next to this bundle. Neither is
# vendored, so static/js/rum.js keeps sessionReplaySampleRate at 0 and never calls
# startSessionReplayRecording(). Turning replay on means fetching bundle/chunks/ too.
