#!/bin/bash
# Deploy script for lindfors-site
# Processes citations and PDFs locally, then pushes to GitHub.
# Cloudflare Pages builds automatically on push.
#
# This commits and pushes whatever it produces, so it refuses to run when the PDF
# toolchain would generate degraded artifacts. SKIP_PDFS=1 skips PDFs entirely.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
source "$SCRIPT_DIR/scripts/lib.sh"

command -v typst >/dev/null 2>&1 || { echo "Error: typst is required"; exit 1; }

SITE_TOOLS="$(site_tools_bin "$SCRIPT_DIR")"

if [ -z "$SKIP_PDFS" ]; then
    preflight_pdfs "$SCRIPT_DIR" || exit 1
fi

# Process citations (requires a local Zotero library)
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
    "$SITE_TOOLS" cv build || echo "  Warning: Failed to generate CV"

    echo "Generating blog PDFs..."
    "$SITE_TOOLS" pdf all || echo "  Warning: Some PDFs failed to generate"
fi

# Verify build works before pushing
echo "Testing build..."
run_zola build

echo "Committing changes..."
git add -A
git commit -m "Build: process citations and generate PDFs" 2>/dev/null || echo "No changes to commit"
git push

echo "Done! Cloudflare Pages will build automatically."
