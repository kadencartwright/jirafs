#!/usr/bin/env bash
set -euo pipefail

OLD_NAME="fs-jira"
NEW_NAME="jirafs"
UID_NUM="$(id -u)"
OS="$(uname -s)"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"

OLD_CFG="$XDG_CONFIG_HOME/$OLD_NAME"
NEW_CFG="$XDG_CONFIG_HOME/$NEW_NAME"

echo "==> Migrating config: $OLD_CFG -> $NEW_CFG"
if [ -d "$OLD_CFG" ]; then
  mkdir -p "$NEW_CFG"
  python - "$OLD_CFG" "$NEW_CFG" <<'PY'
from pathlib import Path
import shutil, sys

old = Path(sys.argv[1])
new = Path(sys.argv[2])

for src in old.rglob("*"):
    rel = src.relative_to(old)
    dst = new / rel
    if src.is_dir():
        dst.mkdir(parents=True, exist_ok=True)
    elif src.is_file() and not dst.exists():
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
PY
  rm -rf "$OLD_CFG"
fi

echo "==> Cleaning old binaries"
rm -f "$HOME/.cargo/bin/$OLD_NAME" \
      "$HOME/.local/bin/$OLD_NAME-desktop"

echo "==> Cleaning old desktop artifacts"
rm -f "$XDG_DATA_HOME/applications/$OLD_NAME-desktop.desktop" \
      "$XDG_DATA_HOME/icons/hicolor/256x256/apps/$OLD_NAME-desktop.png"
rm -rf "$HOME/Applications/$OLD_NAME Desktop.app"

if [ "$OS" = "Linux" ]; then
  echo "==> Cleaning old Linux service"
  systemctl --user disable --now "$OLD_NAME.service" >/dev/null 2>&1 || true
  rm -f "$XDG_CONFIG_HOME/systemd/user/$OLD_NAME.service"
  systemctl --user daemon-reload || true
  systemctl --user reset-failed || true
elif [ "$OS" = "Darwin" ]; then
  echo "==> Cleaning old macOS service"
  launchctl bootout "gui/$UID_NUM/com.$OLD_NAME.mount" >/dev/null 2>&1 || true
  rm -f "$HOME/Library/LaunchAgents/com.$OLD_NAME.mount.plist"
  rm -f "$HOME/Library/Logs/$OLD_NAME.log" "$HOME/Library/Logs/$OLD_NAME.err.log"
fi

echo "==> Done. Next:"
echo "    just install"
echo "    just service-install"
echo "    just service-enable"
