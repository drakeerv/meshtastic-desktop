#!/usr/bin/env bash
# Install the desktop entry and icon so KDE/GNOME can associate the running
# window (Wayland application id "org.meshtastic.Meshtastic") with an icon.
#
# Usage: scripts/install-desktop.sh [/path/to/meshtastic-binary]
set -euo pipefail

APP_ID=org.meshtastic.Meshtastic
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ICON_SRC="$ROOT/assets/meshtastic/app_icon.png"

BIN="${1:-}"
if [ -z "$BIN" ]; then
  if [ -x "$ROOT/target/release/meshtastic" ]; then
    BIN="$ROOT/target/release/meshtastic"
  elif [ -x "$ROOT/target/debug/meshtastic" ]; then
    BIN="$ROOT/target/debug/meshtastic"
  else
    BIN="meshtastic"
  fi
fi

ICON_DIR="$HOME/.local/share/icons/hicolor"
APPS_DIR="$HOME/.local/share/applications"
BIN_DIR="$HOME/.local/bin"

mkdir -p "$ICON_DIR/512x512/apps" "$ICON_DIR/256x256/apps" "$APPS_DIR" "$BIN_DIR"
magick "$ICON_SRC" -resize 512x512 "$ICON_DIR/512x512/apps/$APP_ID.png"
magick "$ICON_SRC" -resize 256x256 "$ICON_DIR/256x256/apps/$APP_ID.png"

if [ "$BIN" = "meshtastic" ]; then
  EXEC="meshtastic"
else
  ln -sf "$BIN" "$BIN_DIR/meshtastic"
  EXEC="$BIN_DIR/meshtastic"
fi

sed "s|@EXEC@|$EXEC|" "$ROOT/packaging/$APP_ID.desktop" > "$APPS_DIR/$APP_ID.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS_DIR" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$ICON_DIR" 2>/dev/null || true

echo "installed:"
echo "  $APPS_DIR/$APP_ID.desktop  (Exec=$EXEC)"
echo "  $ICON_DIR/512x512/apps/$APP_ID.png"
