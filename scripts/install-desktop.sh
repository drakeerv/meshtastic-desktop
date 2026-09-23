#!/usr/bin/env bash
# Install the desktop entry and icon so KDE/GNOME can associate the running
# window (Wayland application id "org.meshtastic.Meshtastic") with an icon.
#
# The app must be installed first, so the entry points at a stable binary
# instead of guessing between a release and debug build tree:
#
#   cargo install --path .
#
# Usage: scripts/install-desktop.sh [/path/to/meshtastic-binary]
set -euo pipefail

APP_ID=org.meshtastic.Meshtastic
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ICON_SRC="$ROOT/assets/meshtastic/app_icon.png"

# Older versions of this script symlinked the build tree into ~/.local/bin.
# Drop that first so it cannot shadow the cargo-installed binary on PATH.
LOCAL_BIN="$HOME/.local/bin/meshtastic"
if [ -L "$LOCAL_BIN" ]; then
  case "$(readlink "$LOCAL_BIN")" in
    */target/release/meshtastic | */target/debug/meshtastic) rm -f "$LOCAL_BIN" ;;
  esac
fi

# Resolve the installed binary. An explicit path wins; otherwise prefer the
# one `cargo install --path .` placed in the cargo bin directory, falling back
# to whatever `meshtastic` resolves to on PATH.
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/meshtastic"
BIN="${1:-}"
if [ -z "$BIN" ] && [ -x "$CARGO_BIN" ]; then
  BIN="$CARGO_BIN"
fi
if [ -z "$BIN" ]; then
  BIN="$(command -v meshtastic || true)"
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  cat >&2 <<'EOF'
error: the 'meshtastic' binary was not found.

Install it first, then re-run this script:

    cargo install --path .
EOF
  exit 1
fi
BIN="$(readlink -f "$BIN")"

ICON_DIR="$HOME/.local/share/icons/hicolor"
APPS_DIR="$HOME/.local/share/applications"

mkdir -p "$ICON_DIR/512x512/apps" "$ICON_DIR/256x256/apps" "$APPS_DIR"
magick "$ICON_SRC" -resize 512x512 "$ICON_DIR/512x512/apps/$APP_ID.png"
magick "$ICON_SRC" -resize 256x256 "$ICON_DIR/256x256/apps/$APP_ID.png"

sed "s|@EXEC@|$BIN|" "$ROOT/packaging/$APP_ID.desktop" > "$APPS_DIR/$APP_ID.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS_DIR" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$ICON_DIR" 2>/dev/null || true

echo "installed:"
echo "  $APPS_DIR/$APP_ID.desktop  (Exec=$BIN)"
echo "  $ICON_DIR/512x512/apps/$APP_ID.png"
