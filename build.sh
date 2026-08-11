#!/bin/bash
# Build script for lindfors-site
# Processes citations and builds the Zola site

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/scripts/lib.sh"

if [ -z "$SKIP_PDFS" ]; then
    preflight_pdfs "$SCRIPT_DIR" || exit 1
fi

# Install zotero-cite if not available
if ! command -v zotero-cite &> /dev/null; then
    echo "Installing zotero-cite..."
    cargo install --git https://github.com/EmilLindfors/zotero-cite
fi

# Process all markdown files with citations
echo "Processing citations..."
for file in $(find "$SCRIPT_DIR/content" -name "*.md" -type f); do
    if grep -q '@[a-zA-Z]' "$file" 2>/dev/null; then
        echo "  Processing: $file"
        zotero-cite process "$file" --output "$file" 2>&1 || true
    fi
done

# Font paths for Typst. --font-path is recursive, so one path covers inter, literata,
# jetbrains-mono and libertinus. Populate it with scripts/fetch-fonts.sh.
FONT_PATHS="--font-path $SCRIPT_DIR/fonts"

if [ -n "$SKIP_PDFS" ]; then
    echo "Skipping CV and PDF generation (SKIP_PDFS set)"
else
    # Generate CV PDF if needed
    echo "Generating CV..."
    if [ ! -f "$SCRIPT_DIR/static/cv.pdf" ] || [ "$SCRIPT_DIR/cv.typ" -nt "$SCRIPT_DIR/static/cv.pdf" ]; then
        SOURCE_DATE_EPOCH="$(stable_epoch_for "$SCRIPT_DIR/cv.typ")" \
            typst compile $FONT_PATHS "$SCRIPT_DIR/cv.typ" "$SCRIPT_DIR/static/cv.pdf" 2>&1 \
            || echo "  Warning: Failed to generate CV PDF"
        echo "  Generated: cv.pdf"
    else
        echo "  CV up to date"
    fi

    # Generate PDFs for all blog posts
    echo "Generating PDFs..."
    mkdir -p "$SCRIPT_DIR/static/pdf"
    for post in "$SCRIPT_DIR"/content/blog/*/index.md; do
        if [ -f "$post" ]; then
            "$SCRIPT_DIR/scripts/generate-pdf.sh" "$post" 2>&1 || echo "  Warning: Failed to generate PDF for $post"
        fi
    done
fi

# Build with Zola
echo "Building site with Zola..."
run_zola build

echo "Done!"
