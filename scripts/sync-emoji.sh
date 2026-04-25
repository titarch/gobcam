#!/usr/bin/env bash
# Download Fluent emoji assets listed in assets/fluent/manifest.toml.
# Idempotent: skips files already on disk. No `git lfs` install required —
# we hit raw.githubusercontent.com (static) and media.githubusercontent.com
# (LFS-resolved) directly.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
manifest="$repo_root/assets/fluent/manifest.toml"
dest_root="$repo_root/assets/fluent"

[[ -f "$manifest" ]] || { echo "manifest not found: $manifest" >&2; exit 1; }

# Emit one line per (emoji, tone, style): "id|upstream_name|stub|tone|style"
# An empty tone means the emoji is tone-less. Python is invoked once for the parse.
plan=$(python3 - "$manifest" <<'PY'
import sys, tomllib
with open(sys.argv[1], "rb") as f:
    data = tomllib.load(f)
default_styles = data.get("default_styles", ["3d", "animated"])
for e in data.get("emoji", []):
    tones = e.get("tones", [])
    styles = e.get("styles", default_styles)
    if not tones:
        for s in styles:
            print(f"{e['id']}|{e['upstream_name']}|{e['stub']}||{s}")
    else:
        for t in tones:
            for s in styles:
                print(f"{e['id']}|{e['upstream_name']}|{e['stub']}|{t}|{s}")
PY
)

# Map our lowercase tone tokens to the casing the upstream repo uses.
tone_dir() {
    case "$1" in
        default)       echo "Default" ;;
        light)         echo "Light" ;;
        medium_light)  echo "Medium-Light" ;;
        medium)        echo "Medium" ;;
        medium_dark)   echo "Medium-Dark" ;;
        dark)          echo "Dark" ;;
        *) echo "unknown tone: $1" >&2; return 1 ;;
    esac
}

# Map a style token to (repo, dir, extension). The repo decides which host
# we hit: static comes from the plain `fluentui-emoji` repo, animated from
# `fluentui-emoji-animated` via LFS.
style_meta() {
    case "$1" in
        3d)            echo "fluentui-emoji 3D png" ;;
        color)         echo "fluentui-emoji Color svg" ;;
        flat)          echo "fluentui-emoji Flat svg" ;;
        high_contrast) echo "fluentui-emoji High%20Contrast svg" ;;
        animated)      echo "fluentui-emoji-animated animated png" ;;
        *) echo "unknown style: $1" >&2; return 1 ;;
    esac
}

url_for() {
    local repo="$1" tone_url="$2" style_dir="$3" filename="$4" upstream_name="$5"
    local enc_name="${upstream_name// /%20}"
    local path="assets/${enc_name}"
    [[ -n "$tone_url" ]] && path+="/${tone_url}"
    path+="/${style_dir}/${filename}"

    if [[ "$repo" == "fluentui-emoji-animated" ]]; then
        echo "https://media.githubusercontent.com/media/microsoft/${repo}/main/${path}"
    else
        echo "https://raw.githubusercontent.com/microsoft/${repo}/main/${path}"
    fi
}

count_total=0
count_downloaded=0
count_skipped=0

while IFS='|' read -r id upstream_name stub tone style; do
    [[ -z "$id" ]] && continue
    read -r repo style_dir ext < <(style_meta "$style")
    if [[ -n "$tone" ]]; then
        tone_url=$(tone_dir "$tone")
        filename="${stub}_${style}_${tone}.${ext}"
        dest="$dest_root/$id/$tone/$style/$filename"
    else
        tone_url=""
        filename="${stub}_${style}.${ext}"
        dest="$dest_root/$id/$style/$filename"
    fi

    count_total=$((count_total + 1))
    if [[ -f "$dest" ]]; then
        count_skipped=$((count_skipped + 1))
        continue
    fi

    url=$(url_for "$repo" "$tone_url" "$style_dir" "$filename" "$upstream_name")
    mkdir -p "$(dirname "$dest")"
    printf "  fetching %s\n" "$id/${tone:+$tone/}$style"
    if ! curl -fsSL --max-time 60 -o "$dest" "$url"; then
        echo "  ERROR: download failed: $url" >&2
        rm -f "$dest"
        exit 1
    fi
    count_downloaded=$((count_downloaded + 1))
done <<< "$plan"

echo "[sync-emoji] $count_total entries: $count_downloaded fetched, $count_skipped already present"
