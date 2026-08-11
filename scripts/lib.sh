#!/bin/bash
# Shared helpers for build.sh, deploy.sh and scripts/generate-pdf.sh.
# Source this, don't execute it.

ZOLA_VERSION="0.23.2"
ZOLA_IMAGE="ghcr.io/getzola/zola:v$ZOLA_VERSION"

# Run zola, falling back to the pinned docker image.
#
# The templates use Tera v2 (components, no macros), so they need 0.23+. On Windows
# 0.23.x additionally cannot load any templates at all -- it canonicalises the project
# root to a \\?\ UNC path and the templates glob then matches nothing
# (https://github.com/getzola/zola/issues/3229) -- so docker is the only way to build
# here until that is fixed.
run_zola() {
    local v=""
    if command -v zola >/dev/null 2>&1; then
        v="$(zola --version 2>/dev/null | awk '{print $2}')"
    fi

    case "$v" in
        0.23.*)
            zola "$@"
            return $?
            ;;
    esac

    if ! command -v docker >/dev/null 2>&1; then
        echo "Error: this site needs zola 0.23.x (found '${v:-none}') or docker." >&2
        echo "       Install zola 0.23.2, or install Docker and re-run." >&2
        return 1
    fi

    if [ -n "$v" ]; then
        echo "  (local zola is $v, need 0.23.x -- using $ZOLA_IMAGE)"
    else
        echo "  (no local zola -- using $ZOLA_IMAGE)"
    fi

    if [ -n "$MSYSTEM" ]; then
        # Git Bash mangles /site into a Windows path unless this is disabled.
        MSYS_NO_PATHCONV=1 docker run --rm \
            -v "$(pwd -W):/site" -w //site "$ZOLA_IMAGE" "$@"
    else
        docker run --rm -v "$(pwd):/site" -w /site "$ZOLA_IMAGE" "$@"
    fi
}

# Locate a real ImageMagick. Sets IM_CMD on success.
#
# Careful on Windows: C:\Windows\system32\convert.exe is the FAT-to-NTFS volume
# converter and happily shadows ImageMagick on PATH, so the name alone proves nothing.
find_imagemagick() {
    local c
    for c in magick convert; do
        if command -v "$c" >/dev/null 2>&1 && "$c" -version 2>/dev/null | grep -qi imagemagick; then
            IM_CMD="$c"
            return 0
        fi
    done
    IM_CMD=""
    return 1
}

# Typst needs the font sources, which are gitignored and not downloaded automatically.
# Without them every PDF silently renders in fallback fonts.
fonts_present() {
    local root="$1" d
    for d in inter literata; do
        [ -d "$root/fonts/$d" ] || return 1
    done
    return 0
}

# Fail before generating PDFs we would otherwise commit in a degraded state.
# Set SKIP_PDFS=1 to build the site without touching PDFs at all.
preflight_pdfs() {
    local root="$1" ok=0

    if ! fonts_present "$root"; then
        echo "Error: fonts/inter and fonts/literata are missing." >&2
        echo "       Typst would fall back to system fonts and every regenerated PDF" >&2
        echo "       would differ from the committed ones. Restore them, or run SKIP_PDFS=1." >&2
        ok=1
    fi

    if ls "$root"/content/blog/*/*.webp >/dev/null 2>&1 && ! find_imagemagick; then
        echo "Error: no ImageMagick found (checked 'magick' and 'convert')." >&2
        echo "       Posts have .webp images and Typst cannot read WebP, so PDFs would" >&2
        echo "       be generated without their images. Install ImageMagick, or run" >&2
        echo "       SKIP_PDFS=1 to leave the committed PDFs alone." >&2
        ok=1
    fi

    return $ok
}
