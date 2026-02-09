#!/usr/bin/env bash
#
# Generate a newsletter markdown file from a blog post.
#
# Usage: ./scripts/generate-newsletter.sh content/blog/my-post/index.md
#        ./scripts/generate-newsletter.sh content/blog/my-post.md
#
# Output: static/newsletter/<slug>.md
#
# The Worker renders this markdown to HTML with pulldown-cmark and wraps
# it in the email template at send time. Deploy the site after generating
# so the .md file is available at https://lindfors.no/newsletter/<slug>.md

set -euo pipefail

SITE_URL="https://lindfors.no"

if [ $# -lt 1 ]; then
    echo "Usage: $0 <path-to-markdown-post>"
    exit 1
fi

INPUT="$1"

if [ ! -f "$INPUT" ]; then
    echo "Error: File not found: $INPUT"
    exit 1
fi

# Extract frontmatter (Zola uses +++ delimiters)
TITLE=$(awk '/^\+\+\+$/{n++; next} n==1{print}' "$INPUT" | grep '^title' | head -1 | sed 's/^title *= *"//; s/"$//')
DATE=$(awk '/^\+\+\+$/{n++; next} n==1{print}' "$INPUT" | grep '^date' | head -1 | sed 's/^date *= *//; s/"//g')
DESCRIPTION=$(awk '/^\+\+\+$/{n++; next} n==1{print}' "$INPUT" | grep '^description' | head -1 | sed 's/^description *= *"//; s/"$//')

# Determine slug from filename
BASENAME=$(basename "$(dirname "$INPUT")")
if [ "$BASENAME" = "blog" ] || [ "$BASENAME" = "content" ]; then
    BASENAME=$(basename "$INPUT" .md)
fi
SLUG="$BASENAME"

POST_URL="${SITE_URL}/blog/${SLUG}/"

# Extract body (after second +++)
BODY=$(awk '/^\+\+\+$/{n++; next} n>=2{print}' "$INPUT")

# Clean up the body for email:
# - Remove shortcodes (figure, katex, reference)
# - Replace math blocks with [View math on site] placeholder
BODY=$(echo "$BODY" \
    | sed 's/{{[[:space:]]*figure(.*)}}/[Image - view on site]/g' \
    | sed 's/{{[[:space:]]*katex(.*)}}/[Math equation - view on site]/g' \
    | sed 's/{%[[:space:]]*katex.*%}//g; s/{%[[:space:]]*end.*%}//g' \
    | sed 's/\$\$[^$]*\$\$/[Math equation - view on site]/g' \
)

mkdir -p "static/newsletter"

cat > "static/newsletter/${SLUG}.md" << MDEOF
---
title: "${TITLE}"
date: "${DATE}"
description: "${DESCRIPTION}"
url: "${POST_URL}"
---

${BODY}
MDEOF

echo "Newsletter generated: static/newsletter/${SLUG}.md"
echo "Slug: ${SLUG}"
echo ""
echo "Next steps:"
echo "  1. Deploy site so the .md is available online"
echo "  2. ./scripts/send-newsletter.sh ${SLUG}"
