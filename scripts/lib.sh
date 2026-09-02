#!/bin/bash
# Shared helpers for build.sh and deploy.sh. Source this, don't execute it.

ZOLA_VERSION="0.23.4"
ZOLA_IMAGE="ghcr.io/getzola/zola:v$ZOLA_VERSION"

# Run zola, falling back to the pinned docker image.
#
# The templates use Tera v2 (components, no macros), so they need 0.23+. On Windows,
# 0.23.0-0.23.2 could not load any templates at all -- they canonicalised the project
# root to a \\?\ UNC path and the templates glob then matched nothing
# (https://github.com/getzola/zola/issues/3229). 0.23.3 fixed that for local drives,
# so a local 0.23.3+ works here; docker is the fallback for a machine without one.
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
        echo "       Install zola $ZOLA_VERSION, or install Docker and re-run." >&2
        return 1
    fi

    if [ -n "$v" ]; then
        echo "  (local zola is $v, need 0.23.x -- using $ZOLA_IMAGE)"
    else
        echo "  (no local zola -- using $ZOLA_IMAGE)"
    fi

    # `serve` needs the port published and the server bound to something other than
    # 127.0.0.1, which inside a container is only reachable from inside the container.
    # Zola's own default interface makes the site look dead from the host.
    local docker_opts=() zola_args=("$@")
    if [ "$1" = "serve" ]; then
        docker_opts=(-p "${ZOLA_PORT:-1111}:${ZOLA_PORT:-1111}")
        zola_args=("$@" --interface 0.0.0.0 --port "${ZOLA_PORT:-1111}"
                   --base-url localhost)
        echo "  Serving on http://localhost:${ZOLA_PORT:-1111}/"
        # File watching does not survive a Windows bind mount: inotify never sees the
        # host's writes, so an edit will not rebuild. Re-run after changing content.
        [ -n "$MSYSTEM" ] && echo "  (no live reload over a Windows mount -- re-run to pick up edits)"
    fi

    if [ -n "$MSYSTEM" ]; then
        # Git Bash mangles /site into a Windows path unless this is disabled.
        MSYS_NO_PATHCONV=1 docker run --rm "${docker_opts[@]}" \
            -v "$(pwd -W):/site" -w //site "$ZOLA_IMAGE" "${zola_args[@]}"
    else
        docker run --rm "${docker_opts[@]}" \
            -v "$(pwd):/site" -w /site "$ZOLA_IMAGE" "${zola_args[@]}"
    fi
}

# Path to the site-tools binary, building it first if it isn't there.
#
# site-tools owns citation processing, PDF generation and the newsletter; the shell
# scripts that used to do those jobs are gone. Echoes the path on success.
site_tools_bin() {
    local root="$1"
    local dir="$root/tools/site-tools"
    local bin="$dir/target/release/site-tools"
    [ -f "$bin.exe" ] && bin="$bin.exe"

    # Build whenever cargo is available, not just when the binary is missing. cargo
    # no-ops in well under a second if nothing changed, and the old "only if absent"
    # check meant a stale binary silently outlived every source edit -- a new
    # subcommand would fail as "Unknown command" mid-build.
    if command -v cargo >/dev/null 2>&1; then
        (cd "$dir" && cargo build --release >&2) || return 1
        bin="$dir/target/release/site-tools"
        [ -f "$bin.exe" ] && bin="$bin.exe"
    elif [ ! -f "$bin" ]; then
        echo "Error: site-tools is not built and cargo is not installed." >&2
        return 1
    fi

    echo "$bin"
}

# Typst needs TTF/OTF sources. static/fonts/ is woff2 and unusable here, and fonts/ is
# gitignored, so a fresh clone has nothing and every PDF renders in fallback fonts.
# Populate with scripts/fetch-fonts.sh.
fonts_present() {
    local root="$1" d
    for d in inter literata jetbrains-mono libertinus; do
        ls "$root/fonts/$d"/*.[to]tf >/dev/null 2>&1 || return 1
    done
    return 0
}

# True when audio generation can run: ffmpeg on PATH and a reachable TTS endpoint.
#
# Both are warnings rather than errors. Audio is only re-synthesised when a script
# changes, so a build with the GPU asleep ships the committed MP3s unchanged -- that is
# the normal case, not a failure. Set SKIP_AUDIO=1 to skip the check entirely.
audio_ready() {
    local root="$1" base

    if ! command -v ffmpeg >/dev/null 2>&1; then
        echo "  Skipping audio: ffmpeg is not on PATH." >&2
        return 1
    fi

    base="$TTS_BASE_URL"
    if [ -z "$base" ] && [ -f "$root/.env" ]; then
        base="$(grep '^TTS_BASE_URL=' "$root/.env" | head -1 | cut -d= -f2-)"
    fi
    base="${base:-https://api.fish.audio}"

    if ! curl -s -o /dev/null -m 5 "$base" 2>/dev/null; then
        echo "  Skipping audio: $base is unreachable." >&2
        return 1
    fi

    return 0
}

# Refuse to build with an unconverted image sitting in content/.
#
# Images are co-located with their post and committed as WebP; `tools/img-optim`
# converts the source and the source is then deleted. Nothing in a build calls it,
# because by the time a build runs the conversion has already happened -- which means
# a forgotten source has nothing to stop it, and `deploy.sh` runs `git add -A`. A 4 MB
# DSLR photo is in the history for good once that happens.
#
# Set SKIP_IMAGE_CHECK=1 to build with one in the tree anyway.
preflight_images() {
    local root="$1" found

    found="$(find "$root/content" -type f         \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.gif'            -o -iname '*.bmp' -o -iname '*.tif' -o -iname '*.tiff' \) 2>/dev/null)"

    [ -z "$found" ] && return 0

    echo "Error: unconverted images under content/:" >&2
    echo "$found" | sed 's/^/       /' >&2
    echo "       Convert them, then delete the sources:" >&2
    echo "         cd tools/img-optim && cargo build --release" >&2
    echo "         ./tools/img-optim/target/release/img-optim -t <path>" >&2
    echo "       (or re-run with SKIP_IMAGE_CHECK=1)" >&2
    return 1
}

# Fail before generating PDFs we would otherwise commit in a degraded state.
# Set SKIP_PDFS=1 to build the site without touching PDFs at all.
preflight_pdfs() {
    local root="$1" ok=0

    # Checked here rather than only in deploy.sh: without typst, build.sh used to warn
    # per-post and carry on, leaving stale PDFs on a site that looked freshly built.
    if ! command -v typst >/dev/null 2>&1; then
        echo "Error: typst is required to generate PDFs and is not on PATH." >&2
        echo "       Install typst, or re-run with SKIP_PDFS=1." >&2
        ok=1
    fi

    if ! fonts_present "$root"; then
        echo "Error: font sources are missing from fonts/." >&2
        echo "       Typst would fall back to system fonts and every regenerated PDF" >&2
        echo "       would differ from the committed ones." >&2
        echo "       Run ./scripts/fetch-fonts.sh (or SKIP_PDFS=1 to skip PDFs)." >&2
        ok=1
    fi

    return $ok
}
