#!/bin/bash
# Build script for lindfors-site
# Processes citations, regenerates PDFs, and builds the Zola site.
#
# The heavy lifting lives in tools/site-tools (Rust); this script is orchestration.
# SKIP_PDFS=1 builds the site without touching the CV or post PDFs.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
source "$SCRIPT_DIR/scripts/lib.sh"

SITE_TOOLS="$(site_tools_bin "$SCRIPT_DIR")"

if [ -z "$SKIP_PDFS" ]; then
    preflight_pdfs "$SCRIPT_DIR" || exit 1
fi

# Process citations in posts that reference @citekeys.
# Needs a local Zotero library, so a failure here is a warning rather than fatal.
echo "Processing citations..."
for file in "$SCRIPT_DIR"/content/blog/*/index.md; do
    [ -f "$file" ] || continue
    if grep -q '@[a-zA-Z]' "$file" 2>/dev/null; then
        echo "  Processing: $(basename "$(dirname "$file")")"
        "$SITE_TOOLS" cite process "$file" --output "$file" \
            || echo "    Warning: citation processing failed"
    fi
done

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
