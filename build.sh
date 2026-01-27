#!/bin/bash
# Build script for lindfors-site
# Processes citations and builds the Zola site

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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

# Font paths for Typst
FONT_PATHS="--font-path $SCRIPT_DIR/fonts/inter --font-path $SCRIPT_DIR/fonts/literata"

# Generate CV PDF if needed
echo "Generating CV..."
if [ ! -f "$SCRIPT_DIR/static/cv.pdf" ] || [ "$SCRIPT_DIR/cv.typ" -nt "$SCRIPT_DIR/static/cv.pdf" ]; then
    typst compile $FONT_PATHS "$SCRIPT_DIR/cv.typ" "$SCRIPT_DIR/static/cv.pdf" 2>&1 || echo "  Warning: Failed to generate CV PDF"
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

# Build with Zola
echo "Building site with Zola..."
zola build

echo "Done!"
