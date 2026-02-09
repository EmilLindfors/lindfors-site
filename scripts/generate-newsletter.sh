#!/usr/bin/env bash
#
# Generate a newsletter-ready HTML email from a blog post.
#
# Usage: ./scripts/generate-newsletter.sh content/blog/my-post/index.md
#        ./scripts/generate-newsletter.sh content/blog/my-post.md
#
# Output: writes to static/newsletter/<slug>.html
#
# The generated HTML is a simple, email-client-safe version of the post
# with inline styles and a link back to the full article for features
# that don't work in email (math, interactive elements, citations).

set -euo pipefail

SITE_URL="https://lindfors.no"
AUTHOR="Emil Lindfors"

if [ $# -lt 1 ]; then
    echo "Usage: $0 <path-to-markdown-post>"
    exit 1
fi

INPUT="$1"

if [ ! -f "$INPUT" ]; then
    echo "Error: File not found: $INPUT"
    exit 1
fi

# Extract frontmatter
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
# - Strip HTML citation links, keep text
BODY=$(echo "$BODY" \
    | sed 's/{{[[:space:]]*figure(.*)}}/[Image - view on site]/g' \
    | sed 's/{{[[:space:]]*katex(.*)}}/[Math equation - view on site]/g' \
    | sed 's/{%[[:space:]]*katex.*%}//g; s/{%[[:space:]]*end.*%}//g' \
    | sed 's/\$\$[^$]*\$\$/[Math equation - view on site]/g' \
)

# Create output directory
mkdir -p "static/newsletter"

# Generate email-safe HTML
cat > "static/newsletter/${SLUG}.html" << EMAILEOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>${TITLE}</title>
</head>
<body style="margin: 0; padding: 0; background-color: #F0EAE0; font-family: Georgia, 'Times New Roman', serif;">
    <div style="max-width: 600px; margin: 0 auto; padding: 32px 24px; background-color: #ffffff;">
        <!-- Header -->
        <div style="border-bottom: 2px solid #2A8F82; padding-bottom: 16px; margin-bottom: 24px;">
            <a href="${SITE_URL}" style="color: #1C3240; text-decoration: none; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 14px; font-weight: 600;">lindfors.no</a>
        </div>

        <!-- Title -->
        <h1 style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 28px; color: #1C3240; margin: 0 0 8px 0; line-height: 1.2;">${TITLE}</h1>

        ${DESCRIPTION:+<p style="color: #5A7078; font-size: 18px; margin: 0 0 16px 0; line-height: 1.5;">$DESCRIPTION</p>}

        <p style="color: #5A7078; font-size: 14px; margin: 0 0 24px 0;">${DATE} &middot; ${AUTHOR}</p>

        <!-- Read on site link -->
        <div style="margin-bottom: 24px; padding: 12px 16px; background-color: #F0EAE0; border-radius: 6px;">
            <a href="${POST_URL}" style="color: #D4706A; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 14px; font-weight: 500;">Read the full post on the site &rarr;</a>
            <span style="color: #5A7078; font-size: 13px; display: block; margin-top: 4px;">For math equations, citations, and interactive features</span>
        </div>

        <!-- Body placeholder - markdown content goes here -->
        <div style="color: #1C3240; font-size: 17px; line-height: 1.75;">
            <p style="color: #5A7078; font-style: italic;">[Post content rendered from markdown below. Process this file through your newsletter service's markdown renderer, or use the pre-rendered version.]</p>
        </div>

        <!-- Footer -->
        <div style="border-top: 2px solid #2A8F82; margin-top: 32px; padding-top: 16px;">
            <p style="color: #5A7078; font-size: 13px; margin: 0 0 8px 0;">You received this because you subscribed to the lindfors.no newsletter.</p>
            <a href="${SITE_URL}" style="color: #D4706A; font-size: 13px;">Visit site</a> &middot;
            <a href="${SITE_URL}/api/unsubscribe" style="color: #D4706A; font-size: 13px;">Unsubscribe</a>
        </div>
    </div>
</body>
</html>
EMAILEOF

# Also save the cleaned markdown body for the newsletter service
cat > "static/newsletter/${SLUG}.md" << MDEOF
---
title: "${TITLE}"
date: "${DATE}"
description: "${DESCRIPTION}"
url: "${POST_URL}"
---

${BODY}

---

*[Read the full post on the site](${POST_URL}) for math equations, citations, and interactive features.*
MDEOF

echo "Newsletter generated:"
echo "  HTML template: static/newsletter/${SLUG}.html"
echo "  Markdown body: static/newsletter/${SLUG}.md"
echo "  Post URL: ${POST_URL}"
