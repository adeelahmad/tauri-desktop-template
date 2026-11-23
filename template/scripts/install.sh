#!/bin/sh
# {{app_name}} for macOS (Apple Silicon) in one command, with no security prompts:
#
#   curl -fsSL https://raw.githubusercontent.com/{{github_repo}}/main/scripts/install.sh | sh
#
# Downloads the newest .dmg from the GitHub releases, copies the app to /Applications
# (asks for your password only if that folder needs it), clears the quarantine flag (the
# app is not signed with an Apple Developer ID) and opens it.
set -eu
REPO="{{github_repo}}"
APP="{{app_name}}.app"

[ "$(uname -s)" = Darwin ] || { echo "This installer is for macOS. On Windows run scripts/install.ps1, or the -setup.exe from the releases page." >&2; exit 1; }
[ "$(uname -m)" = arm64 ] || { echo "This build needs an Apple Silicon Mac (M1 or newer)." >&2; exit 1; }

echo "Finding the newest build..."
url=$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=30" \
  | grep -o '"browser_download_url": *"[^"]*aarch64\.dmg"' | head -1 | sed 's/.*"\(https[^"]*\)"/\1/')
[ -n "$url" ] || { echo "No macOS build found at https://github.com/$REPO/releases" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'hdiutil detach -quiet "$tmp/mnt" 2>/dev/null || true; rm -rf "$tmp"' EXIT
echo "Downloading $(basename "$url")..."
curl -fL --progress-bar -o "$tmp/app.dmg" "$url"
hdiutil attach -quiet -nobrowse -mountpoint "$tmp/mnt" "$tmp/app.dmg"
src=$(find "$tmp/mnt" -maxdepth 1 -name '*.app' | head -1)

osascript -e "tell application \"{{app_name}}\" to quit" >/dev/null 2>&1 || true
if [ -w /Applications ]; then SUDO=""; else SUDO="sudo"; echo "Copying to /Applications needs your Mac password:"; fi
$SUDO rm -rf "/Applications/$APP"
$SUDO cp -R "$src" "/Applications/$APP"
$SUDO xattr -dr com.apple.quarantine "/Applications/$APP" 2>/dev/null || true

echo "Installed /Applications/$APP. Opening it..."
open "/Applications/$APP"
