#!/bin/bash
# Build script for lindfors-site
# Processes citations, regenerates PDFs, and builds the Zola site.
#
# The heavy lifting lives in tools/site-tools (Rust); this script is orchestration.
# This is the single definition of a build -- deploy.sh runs it and then pushes.
# SKIP_PDFS=1 builds the site without touching the CV or post PDFs.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
source "$SCRIPT_DIR/scripts/lib.sh"

SITE_TOOLS="$(site_tools_bin "$SCRIPT_DIR")"

if [ -z "$SKIP_PDFS" ]; then
    preflight_pdfs "$SCRIPT_DIR" || exit 1
fi

# Needs a local Zotero library, so a failure here is a warning rather than fatal.
# site-tools walks content/blog/ itself and honours extra.skip_citations.
echo "Processing citations..."
"$SITE_TOOLS" cite all || echo "  Warning: citation processing failed"

if [ -n "$SKIP_PDFS" ]; then
    echo "Skipping CV and PDF generation (SKIP_PDFS set)"
else
    echo "Generating CV..."
    "$SITE_TOOLS" cv build || echo "  Warning: Failed to generate CV PDF"

    # Drafts are skipped by site-tools, so unpublished posts get no public PDF.
    echo "Generating PDFs..."
    "$SITE_TOOLS" pdf all || echo "  Warning: Some PDFs failed to generate"
fi

echo "Building site with Zola..."
run_zola build

echo "Done!"
