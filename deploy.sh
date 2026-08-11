#!/bin/bash
# Deploy script for lindfors-site
# Processes citations and PDFs locally, then pushes to GitHub
# Cloudflare Pages will build automatically on push

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
source "$SCRIPT_DIR/scripts/lib.sh"

# Check for required tools. zola may come from docker, so run_zola does that check.
command -v typst >/dev/null 2>&1 || { echo "Error: typst is required"; exit 1; }

# This script commits and pushes whatever it produces, so refuse to run if the PDF
# toolchain would silently generate degraded artifacts.
if [ -z "$SKIP_PDFS" ]; then
    preflight_pdfs "$SCRIPT_DIR" || exit 1
fi

# Install zotero-cite if not available
if ! command -v zotero-cite &> /dev/null; then
    echo "Installing zotero-cite..."
    cargo install --git https://github.com/EmilLindfors/zotero-cite
fi

# Process citations (requires local Zotero database)
echo "Processing citations..."
for file in $(find "$SCRIPT_DIR/content" -name "*.md" -type f); do
    if grep -q '@[a-zA-Z]' "$file" 2>/dev/null; then
        echo "  Processing: $file"
        zotero-cite process "$file" --output "$file" 2>&1 || true
    fi
done

if [ -n "$SKIP_PDFS" ]; then
    echo "Skipping CV and PDF generation (SKIP_PDFS set)"
else
    # Generate CV PDF. Note build.sh passes --font-path here and this script does not;
    # they have been out of sync for a while.
    echo "Generating CV..."
    mkdir -p "$SCRIPT_DIR/static"
    typst compile --font-path "$SCRIPT_DIR/fonts" \
        "$SCRIPT_DIR/cv.typ" "$SCRIPT_DIR/static/cv.pdf" 2>&1 || echo "  Warning: Failed to generate CV"

    # Generate blog post PDFs
    echo "Generating blog PDFs..."
    mkdir -p "$SCRIPT_DIR/static/pdf"
    for post in "$SCRIPT_DIR"/content/blog/*/index.md; do
        if [ -f "$post" ]; then
            "$SCRIPT_DIR/scripts/generate-pdf.sh" "$post" 2>&1 || echo "  Warning: Failed to generate PDF for $post"
        fi
    done
fi

# Verify build works
echo "Testing build..."
run_zola build

# Commit and push
echo "Committing changes..."
git add -A
git commit -m "Build: process citations and generate PDFs" 2>/dev/null || echo "No changes to commit"
git push

echo "Done! Cloudflare Pages will build automatically."
