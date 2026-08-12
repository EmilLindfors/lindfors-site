#!/bin/bash
# Deploy script for lindfors-site
# Runs the normal build, then commits and pushes. Cloudflare Pages builds on push.
#
# Everything up to the push is build.sh -- this script deliberately has no pipeline of
# its own. The two used to carry separate copies of the citation and PDF loops, which
# is how they drifted apart (deploy.sh once generated the CV without --font-path).
#
# SKIP_PDFS=1 is passed straight through to build.sh.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# This commits and pushes whatever the build produces, so a failed build must stop it
# rather than push half-regenerated PDFs. build.sh is `set -e` and its preflight exits
# non-zero when the PDF toolchain would produce degraded artifacts.
"$SCRIPT_DIR/build.sh"

echo "Committing changes..."
git add -A
git commit -m "Build: process citations and generate PDFs" 2>/dev/null || echo "No changes to commit"
git push

echo "Done! Cloudflare Pages will build automatically."
