#!/bin/bash
# Download the font sources Typst needs for CV and blog-post PDFs.
#
# static/fonts/ holds woff2 for the website. Typst cannot read woff2, so the PDF
# toolchain needs TTF/OTF and that is what this fetches into fonts/ (gitignored).
# Without it, Typst silently falls back to system fonts and every regenerated PDF
# differs from the committed ones.
#
# Everything is pinned and checksummed. All four families are OFL-licensed.
#
# Usage: ./scripts/fetch-fonts.sh [--force]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
FONTS_DIR="$PROJECT_DIR/fonts"
CACHE_DIR="$FONTS_DIR/.cache"

FORCE=""
[ "$1" = "--force" ] && FORCE=1

JBMONO_URL="https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip"
JBMONO_SHA="6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf"

LIBERTINUS_URL="https://github.com/alerque/libertinus/releases/download/v7.051/Libertinus-7.051.zip"
LIBERTINUS_SHA="4d9be29b5cb380c35af8ba967abcc752ad1e07be1f738a9789c33e0dd7478c92"

# Inter and Literata come from google/fonts, which has no releases, so pin the commit
# that last touched each directory.
#
# Inter specifically must come from here rather than the upstream rsms/inter release:
# that ships InterVariable.ttf, whose family name is "Inter Variable", and the Typst
# templates ask for "Inter". The google/fonts build registers as plain "Inter".
GF_INTER_COMMIT="0b58fb370093f9a9f4ff785d94405710b79de67c"
INTER_BASE="https://raw.githubusercontent.com/google/fonts/$GF_INTER_COMMIT/ofl/inter"
INTER_ROMAN_SHA="29160a80ff49ddcab2c97711247e08b1fab27a484a329ce8b813d820dc559031"
INTER_ITALIC_SHA="acd98e64795781b2058f07b18475e0ecee2a0fe2b42a49e2f9e37d0d6bf66ce6"

GF_LITERATA_COMMIT="4e5f06dbb274a27ebe71ed54ea706b3ee40eabd9"
LITERATA_BASE="https://raw.githubusercontent.com/google/fonts/$GF_LITERATA_COMMIT/ofl/literata"
LITERATA_ROMAN_SHA="b41138c9373112f32abb589cc22e8674b06ed4048b0c513be922bdd26f274440"
LITERATA_ITALIC_SHA="d483dfaeba9cbf4ce71d32a52ee65df82f7e35b15fff8d1011cdb242d1fcd465"

sha_of() { sha256sum "$1" | awk '{print $1}'; }

fetch() { # url expected_sha dest
    local url="$1" want="$2" dest="$3" got tmp

    if [ -f "$dest" ] && [ "$(sha_of "$dest")" = "$want" ]; then
        return 0
    fi

    echo "  downloading $(basename "$dest")"

    # Download to a plain temp name first. Some of these filenames contain [ and ],
    # which Git Bash mangles when it converts arguments for the native curl.exe.
    tmp="$(mktemp)"
    curl -fsSL -o "$tmp" "$url" || {
        echo "Error: failed to download $url" >&2
        rm -f "$tmp"
        return 1
    }

    got="$(sha_of "$tmp")"
    if [ "$got" != "$want" ]; then
        echo "Error: checksum mismatch for $url" >&2
        echo "       expected $want" >&2
        echo "       got      $got" >&2
        rm -f "$tmp"
        return 1
    fi

    mv -f "$tmp" "$dest"
}

# Skip a family if it already has font files, unless --force.
have_family() {
    [ -z "$FORCE" ] && ls "$FONTS_DIR/$1"/*.[to]tf >/dev/null 2>&1
}

mkdir -p "$CACHE_DIR"

echo "Fetching fonts into $FONTS_DIR"

# --- Inter (body sans) ---
if have_family inter; then
    echo "  inter: already present"
else
    mkdir -p "$FONTS_DIR/inter"
    fetch "$INTER_BASE/Inter%5Bopsz%2Cwght%5D.ttf" "$INTER_ROMAN_SHA" \
        "$FONTS_DIR/inter/Inter[opsz,wght].ttf"
    fetch "$INTER_BASE/Inter-Italic%5Bopsz%2Cwght%5D.ttf" "$INTER_ITALIC_SHA" \
        "$FONTS_DIR/inter/Inter-Italic[opsz,wght].ttf"
    echo "  inter: ok"
fi

# --- Literata (body serif, used for post PDFs) ---
if have_family literata; then
    echo "  literata: already present"
else
    mkdir -p "$FONTS_DIR/literata"
    fetch "$LITERATA_BASE/Literata%5Bopsz%2Cwght%5D.ttf" "$LITERATA_ROMAN_SHA" \
        "$FONTS_DIR/literata/Literata[opsz,wght].ttf"
    fetch "$LITERATA_BASE/Literata-Italic%5Bopsz%2Cwght%5D.ttf" "$LITERATA_ITALIC_SHA" \
        "$FONTS_DIR/literata/Literata-Italic[opsz,wght].ttf"
    echo "  literata: ok"
fi

# --- JetBrains Mono (code blocks) ---
if have_family jetbrains-mono; then
    echo "  jetbrains-mono: already present"
else
    mkdir -p "$FONTS_DIR/jetbrains-mono"
    fetch "$JBMONO_URL" "$JBMONO_SHA" "$CACHE_DIR/JetBrainsMono-2.304.zip"
    # The member names contain [wght], which unzip would read as a glob character
    # class, so match the directory instead -- it holds exactly the two variable TTFs.
    unzip -o -j -q "$CACHE_DIR/JetBrainsMono-2.304.zip" \
        "fonts/variable/*" -d "$FONTS_DIR/jetbrains-mono"
    echo "  jetbrains-mono: ok"
fi

# --- Libertinus Serif (cv.typ body, and the Literata fallback in academic.typ) ---
if have_family libertinus; then
    echo "  libertinus: already present"
else
    mkdir -p "$FONTS_DIR/libertinus"
    fetch "$LIBERTINUS_URL" "$LIBERTINUS_SHA" "$CACHE_DIR/Libertinus-7.051.zip"
    unzip -o -j -q "$CACHE_DIR/Libertinus-7.051.zip" \
        "Libertinus-7.051/static/OTF/LibertinusSerif-Regular.otf" \
        "Libertinus-7.051/static/OTF/LibertinusSerif-Bold.otf" \
        "Libertinus-7.051/static/OTF/LibertinusSerif-Italic.otf" \
        "Libertinus-7.051/static/OTF/LibertinusSerif-BoldItalic.otf" \
        -d "$FONTS_DIR/libertinus"
    echo "  libertinus: ok"
fi

# The archives are only needed to extract from; keeping them would leave ~50 MB of
# zips lying around for no reason.
rm -rf "$CACHE_DIR"

echo "Done. $(find "$FONTS_DIR" -name '*.[to]tf' | wc -l) font files in $FONTS_DIR"
