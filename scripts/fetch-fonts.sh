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

# Static instances only. Typst does not support variable fonts -- it warns
# "variable fonts are not currently supported and may render incorrectly" and hints to
# install a static version -- so the google/fonts builds of Inter and Literata, which
# ship only variable TTFs, are unusable here despite being the obvious source.
INTER_URL="https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip"
INTER_SHA="9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e"

LITERATA_URL="https://github.com/googlefonts/literata/releases/download/3.103/3.103.zip"
LITERATA_SHA="f7fb973cafb26cf785cbebaeaf51c18f87c15a3bcf4d82a7d4857564db5b056d"

JBMONO_URL="https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip"
JBMONO_SHA="6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf"

LIBERTINUS_URL="https://github.com/alerque/libertinus/releases/download/v7.051/Libertinus-7.051.zip"
LIBERTINUS_SHA="4d9be29b5cb380c35af8ba967abcc752ad1e07be1f738a9789c33e0dd7478c92"

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
    fetch "$INTER_URL" "$INTER_SHA" "$CACHE_DIR/Inter-4.1.zip"
    # The statics live under extras/; the archive root holds only the variable build.
    unzip -o -j -q "$CACHE_DIR/Inter-4.1.zip" \
        "extras/ttf/Inter-Regular.ttf" "extras/ttf/Inter-Italic.ttf" \
        "extras/ttf/Inter-SemiBold.ttf" "extras/ttf/Inter-Bold.ttf" \
        "extras/ttf/Inter-BoldItalic.ttf" -d "$FONTS_DIR/inter"
    echo "  inter: ok"
fi

# --- Literata (body serif, used for post PDFs) ---
if have_family literata; then
    echo "  literata: already present"
else
    mkdir -p "$FONTS_DIR/literata"
    fetch "$LITERATA_URL" "$LITERATA_SHA" "$CACHE_DIR/Literata-3.103.zip"
    unzip -o -j -q "$CACHE_DIR/Literata-3.103.zip" \
        "fonts/ttf/Literata-Regular.ttf" "fonts/ttf/Literata-Italic.ttf" \
        "fonts/ttf/Literata-Bold.ttf" "fonts/ttf/Literata-BoldItalic.ttf" \
        -d "$FONTS_DIR/literata"
    echo "  literata: ok"
fi

# --- JetBrains Mono (code blocks) ---
if have_family jetbrains-mono; then
    echo "  jetbrains-mono: already present"
else
    mkdir -p "$FONTS_DIR/jetbrains-mono"
    fetch "$JBMONO_URL" "$JBMONO_SHA" "$CACHE_DIR/JetBrainsMono-2.304.zip"
    unzip -o -j -q "$CACHE_DIR/JetBrainsMono-2.304.zip" \
        "fonts/ttf/JetBrainsMono-Regular.ttf" "fonts/ttf/JetBrainsMono-Italic.ttf" \
        "fonts/ttf/JetBrainsMono-Bold.ttf" "fonts/ttf/JetBrainsMono-BoldItalic.ttf" \
        -d "$FONTS_DIR/jetbrains-mono"
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
