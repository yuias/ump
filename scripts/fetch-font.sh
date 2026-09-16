#!/bin/sh
# Downloads DotGothic16-Regular.ttf (SIL OFL 1.1) from google/fonts, pinned to
# a fixed commit, and installs it into ump's font directory. ump does not
# bundle fonts; it auto-detects this file there when [font].path is unset in
# settings.toml (see README.md's Fonts section).
set -eu

COMMIT_SHA="9a6cc6b8ce992aff77b69c857c46af0b42cdff76"
TTF_SHA256="3ad9af88726d42b40f7f365f0dcac785af73cf20ea6f1d5b44e57cc21150b8f1"
TTF_URL="https://raw.githubusercontent.com/google/fonts/${COMMIT_SHA}/ofl/dotgothic16/DotGothic16-Regular.ttf"
OFL_URL="https://raw.githubusercontent.com/google/fonts/${COMMIT_SHA}/ofl/dotgothic16/OFL.txt"

# Next to settings.toml; matches Config::fonts_dir() (dirs::config_dir()) in src/config.rs.
if [ -n "${UMP_FONT_DIR:-}" ]; then
    dest="$UMP_FONT_DIR"
elif [ "$(uname)" = "Darwin" ]; then
    dest="$HOME/Library/Application Support/ump/fonts"
else
    dest="${XDG_CONFIG_HOME:-$HOME/.config}/ump/fonts"
fi

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        echo "Error: neither curl nor wget is available" >&2
        exit 1
    fi
}

mkdir -p "$dest"
ttf="$dest/DotGothic16-Regular.ttf"
license="$dest/DotGothic16-OFL.txt"

if [ -f "$ttf" ] && [ "$(sha256_of "$ttf")" = "$TTF_SHA256" ]; then
    echo "Already installed and verified: $ttf"
    exit 0
fi

# Download next to the destination so the final move is a same-volume,
# effectively atomic rename -- a failed download never leaves $ttf partial.
tmp=$(mktemp "$dest/DotGothic16-Regular.ttf.XXXXXX")
trap 'rm -f "$tmp"' EXIT

fetch "$TTF_URL" "$tmp"

actual_sha256=$(sha256_of "$tmp")
if [ "$actual_sha256" != "$TTF_SHA256" ]; then
    echo "Checksum mismatch: expected $TTF_SHA256, got $actual_sha256" >&2
    exit 1
fi

mv "$tmp" "$ttf"
fetch "$OFL_URL" "$license"

echo "Installed: $ttf"
echo "License:   $license"
echo "ump picks this up automatically on next launch when [font].path is unset in settings.toml."
