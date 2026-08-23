#!/bin/bash
# Build script for lindfors-site
# Processes citations, regenerates PDFs, and builds the Zola site.
#
# The heavy lifting lives in tools/site-tools (Rust); this script is orchestration.
# This is the single definition of a build -- deploy.sh runs it and then pushes.
# SKIP_PDFS=1 builds the site without touching the CV or post PDFs.
# SKIP_AUDIO=1 builds without synthesising audio (it is skipped anyway when the TTS
# endpoint is unreachable or nothing changed).

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

# Plain-markdown representations served by content negotiation. Cheap and dependency
# free, so it runs even when PDFs are skipped. Must happen after `cite`, so the
# committed markdown carries the same rendered citations the HTML does.
echo "Generating markdown representations..."
"$SITE_TOOLS" markdown all || echo "  Warning: markdown generation failed"

# Spoken scripts for the audio versions. Pure text derivation, no network, so it runs
# on every build -- the script is what gets reviewed when the audio sounds wrong. Must
# happen after `cite` for the same reason `markdown` does.
echo "Generating speech scripts..."
"$SITE_TOOLS" speech all || echo "  Warning: speech script generation failed"

# Synthesis is gated on the script hash, so an unchanged post costs nothing and needs
# no endpoint. SKIP_AUDIO=1 skips it outright.
if [ -z "$SKIP_AUDIO" ] && audio_ready "$SCRIPT_DIR"; then
    echo "Generating audio..."
    "$SITE_TOOLS" audio all || echo "  Warning: audio generation failed"
fi

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
