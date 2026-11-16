#!/usr/bin/env sh
set -eu

# User-level installer: no sudo, no system directories.
# Usage: ./packaging/install-linux.sh [path-to-binary]

BIN="${1:-./lockin-final-hope-linux-x86_64}"
PREFIX="${HOME}/.local"
APPS="${PREFIX}/share/applications"
TARGET="${PREFIX}/bin/lockin-final-hope"

if [ ! -f "$BIN" ]; then
  echo "Binary not found: $BIN" >&2
  echo "Download lockin-final-hope-linux-x86_64 from GitHub Releases first." >&2
  exit 1
fi

mkdir -p "${PREFIX}/bin" "$APPS"
install -m 755 "$BIN" "$TARGET"

cat > "${APPS}/lockin-final-hope.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Lockin — study OS
Comment=Local-first study and exam OS
Exec=${TARGET}
Icon=applications-education
Terminal=false
Categories=Education;
StartupNotify=true
EOF

update-desktop-database "$APPS" 2>/dev/null || true

echo "Installed Lockin to ${TARGET}"
echo "Desktop entry: ${APPS}/lockin-final-hope.desktop"
case ":${PATH}:" in
  *":${PREFIX}/bin:"*) ;;
  *) echo "Add ${PREFIX}/bin to PATH to run 'lockin-final-hope' from a terminal." ;;
esac
