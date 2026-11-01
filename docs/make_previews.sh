#!/usr/bin/env bash
# Regenerate the README screenshots and the GitHub social-preview image.
#
# Uses the app's debug-only preview mode (sample data, no system access), so it runs on
# macOS, Linux or Windows (Git Bash). Needs Rust and Python 3 with Pillow.
#   ./docs/make_previews.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -p syscleanx --features screenshot
BIN=target/debug/syscleanx
RAW=$(mktemp -d)
trap 'rm -rf "$RAW"' EXIT

# Each shot gets its own empty data folder so your real settings never leak into images.
shot() { # shot <name> <page> [settings-json] [extra env...]
  local name=$1 page=$2 settings=${3:-} data="$RAW/data-$1"
  shift 3 || shift $#
  mkdir -p "$data/SysCleanX"
  [ -n "$settings" ] && echo "$settings" > "$data/SysCleanX/settings.json"
  env APPDATA="$data" SYSCLEANX_DEMO=1 SYSCLEANX_PAGE="$page" EFRAME_SCREENSHOT_TO="$RAW/$name.png" "$@" "$BIN" >/dev/null 2>&1
}

shot clean clean ""
shot clean-details clean "" SYSCLEANX_DEMO_OPEN=1
shot clean-light clean '{"light_theme": true, "accent": 1}'
shot apps apps ""
shot store store ""
shot startup startup ""
shot repair repair ""
shot disk disk ""
shot duplicates duplicates ""
shot settings settings ""

python3 docs/make_previews.py "$RAW" docs
