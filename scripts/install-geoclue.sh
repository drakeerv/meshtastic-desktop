#!/usr/bin/env bash
# Install the GeoClue allow-list entry for this app id so location requests
# skip the authorization-agent prompt.
#
# GeoClue (and GeoClue-compatible services such as locrust) read application
# sections from /etc/geoclue/geoclue.conf and /etc/geoclue/conf.d/*.conf.
#
# Usage: scripts/install-geoclue.sh
set -euo pipefail

APP_ID=org.meshtastic.Meshtastic
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/packaging/50-meshtastic-geoclue.conf"
DST_DIR=/etc/geoclue/conf.d
DST="$DST_DIR/50-meshtastic.conf"

if [ "$(id -u)" = 0 ]; then
  ESCALATE=()
elif command -v doas >/dev/null 2>&1; then
  ESCALATE=(doas)
elif command -v sudo >/dev/null 2>&1; then
  ESCALATE=(sudo)
else
  echo "root privileges are needed; copy $SRC to $DST manually" >&2
  exit 1
fi

"${ESCALATE[@]}" mkdir -p "$DST_DIR"
"${ESCALATE[@]}" cp "$SRC" "$DST"
"${ESCALATE[@]}" chmod 644 "$DST"

echo "installed:"
echo "  $DST  (['$APP_ID'] allowed=true)"
echo
echo "GeoClue restarts on demand via D-Bus; to apply it now, restart the"
echo "service or toggle location once in the app."
