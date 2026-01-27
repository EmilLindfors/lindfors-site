#!/bin/bash
# Deploy script for lindfors-site
# Processes citations and PDFs locally, then pushes to GitHub
# Cloudflare Pages will build automatically on push

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Check for required tools
command -v zola >/dev/null 2>&1 || { echo "Error: zola is required"; exit 1; }
command -v typst >/dev/null 2>&1 || { echo "Error: typst is required"; exit 1; }

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

# Generate CV PDF
echo "Generating CV..."
mkdir -p "$SCRIPT_DIR/static"
typst compile "$SCRIPT_DIR/cv.typ" "$SCRIPT_DIR/static/cv.pdf" 2>&1 || echo "  Warning: Failed to generate CV"

# Generate blog post PDFs
echo "Generating blog PDFs..."
mkdir -p "$SCRIPT_DIR/static/pdf"
for post in "$SCRIPT_DIR"/content/blog/*/index.md; do
    if [ -f "$post" ]; then
        "$SCRIPT_DIR/scripts/generate-pdf.sh" "$post" 2>&1 || echo "  Warning: Failed to generate PDF for $post"
    fi
done

# Verify build works
echo "Testing build..."
zola build

# Commit and push
echo "Committing changes..."
git add -A
git commit -m "Build: process citations and generate PDFs" 2>/dev/null || echo "No changes to commit"
git push

echo "Done! Cloudflare Pages will build automatically."
