#!/bin/bash
# Generate PDF from markdown blog post using Typst + cmarker
# Usage: ./scripts/generate-pdf.sh content/blog/post-slug/index.md

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TEMPLATE="$PROJECT_DIR/templates/pdf/academic.typ"
OUTPUT_DIR="$PROJECT_DIR/static/pdf"

if [ -z "$1" ]; then
    echo "Usage: $0 <markdown-file>"
    exit 1
fi

INPUT_FILE="$1"

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: File not found: $INPUT_FILE"
    exit 1
fi

# Extract slug from path (e.g., content/blog/example-post/index.md -> example-post)
SLUG=$(basename "$(dirname "$INPUT_FILE")")
OUTPUT_FILE="$OUTPUT_DIR/$SLUG.pdf"

echo "Generating PDF for: $SLUG"

# Create temp directory
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Extract frontmatter and content
awk '
BEGIN { in_front = 0; front_done = 0 }
/^\+\+\+$/ {
    if (in_front == 0) { in_front = 1; next }
    else { front_done = 1; in_front = 0; next }
}
in_front == 1 { print > "'"$TEMP_DIR"'/frontmatter.toml" }
front_done == 1 { print > "'"$TEMP_DIR"'/content.md" }
' "$INPUT_FILE"

# Parse frontmatter for title and date
TITLE=$(grep -E '^title\s*=' "$TEMP_DIR/frontmatter.toml" 2>/dev/null | sed 's/^title\s*=\s*"\(.*\)"/\1/' || echo "Untitled")
DATE=$(grep -E '^date\s*=' "$TEMP_DIR/frontmatter.toml" 2>/dev/null | sed 's/^date\s*=\s*//' || echo "")

# Format date if present
if [ -n "$DATE" ]; then
    DATE=$(date -d "$DATE" "+%B %d, %Y" 2>/dev/null || echo "$DATE")
fi

# Preprocess markdown content:
# 1. Convert internal anchor links [text](#ref-...) to just plain text
# 2. Strip HTML reference paragraphs and convert to plain text
sed -i -E 's/\[([0-9]+)\]\(#ref-[^)]+\)/\1/g' "$TEMP_DIR/content.md"
sed -i -E 's/\[([A-Za-z0-9 ]+)\]\(#ref-[^)]+\)/\1/g' "$TEMP_DIR/content.md"

# Convert HTML reference paragraphs to plain text for PDF
# <p id="ref-..." class="reference">Author (Year). <em>Title</em>. <a href="...">doi:...</a>.</p>
sed -i -E 's/<p[^>]*class="reference"[^>]*>/- /g' "$TEMP_DIR/content.md"
sed -i -E 's/<\/p>//g' "$TEMP_DIR/content.md"
sed -i -E 's/<em>/*/g' "$TEMP_DIR/content.md"
sed -i -E 's/<\/em>/*/g' "$TEMP_DIR/content.md"
sed -i -E 's/<a href="([^"]+)">([^<]+)<\/a>/[\2](\1)/g' "$TEMP_DIR/content.md"

# Copy template to temp directory
cp "$TEMPLATE" "$TEMP_DIR/academic.typ"

# Create the Typst document that uses cmarker to render markdown
cat > "$TEMP_DIR/document.typ" << 'TYPSTEOF'
#import "academic.typ": academic
#import "@preview/cmarker:0.1.8"
#import "@preview/mitex:0.2.6": mitex

#show: academic.with(
TYPSTEOF

# Add the title and date (these need shell variable expansion)
cat >> "$TEMP_DIR/document.typ" << EOF
  title: "$TITLE",
  author: "Emil Lindfors",
  date: "$DATE",
)

EOF

# Add the cmarker render call
cat >> "$TEMP_DIR/document.typ" << 'TYPSTEOF'
#cmarker.render(
  read("content.md"),
  math: mitex,
  smart-punctuation: true,
)
TYPSTEOF

# Generate PDF with custom fonts
FONT_PATHS="--font-path $PROJECT_DIR/fonts/inter --font-path $PROJECT_DIR/fonts/literata"
typst compile $FONT_PATHS "$TEMP_DIR/document.typ" "$OUTPUT_FILE"

echo "Generated: $OUTPUT_FILE"
