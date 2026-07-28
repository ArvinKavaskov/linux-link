#!/usr/bin/env bash
#
# Linux Link — PC-side installation.
#
# Installs both binaries (the `linkd` daemon + the `linux-link-gui` system
# tray app), the systemd service (automatic daemon start at session login),
# the icon autostart, the applications menu entry and the Nautilus
# "Send to phone" script.
#
# Usage:
#   ./install.sh            # build then install everything
#   ./install.sh --no-build # reuse the already compiled binaries
#
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
AUTOSTART_DIR="$HOME/.config/autostart"
SYSTEMD_DIR="$HOME/.config/systemd/user"
NAUTILUS_DIR="$HOME/.local/share/nautilus/scripts"

say()  { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m/!\\\033[0m %s\n' "$*"; }

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT/linkd"

BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0

if [ "$BUILD" = 1 ]; then
    say "Building (release, may take a few minutes)…"
    cargo build --release --bin linkd --bin linux-link-gui
fi

if [ ! -x target/release/linkd ] || [ ! -x target/release/linux-link-gui ]; then
    warn "Binaries not found. Run without --no-build to compile them."
    exit 1
fi

say "Installing the binaries in $BIN_DIR"
mkdir -p "$BIN_DIR"
install -m755 target/release/linkd            "$BIN_DIR/linkd"
install -m755 target/release/linux-link-gui   "$BIN_DIR/linux-link-gui"

say "systemd service — the daemon starts at every session login"
mkdir -p "$SYSTEMD_DIR"
install -m644 systemd/linkd.service "$SYSTEMD_DIR/linkd.service"
systemctl --user daemon-reload
systemctl --user enable --now linkd

say "System tray app (menu + automatic launch)"
mkdir -p "$APP_DIR" "$AUTOSTART_DIR"
DESKTOP_TMP="$(mktemp)"
cat > "$DESKTOP_TMP" <<EOF
[Desktop Entry]
Type=Application
Name=Linux Link
GenericName=Android Continuity
Comment=Connection, battery and quick actions to the phone
Exec=$BIN_DIR/linux-link-gui
Icon=phone
Terminal=false
Categories=Network;Utility;
StartupNotify=false
EOF
install -m644 "$DESKTOP_TMP" "$APP_DIR/linux-link.desktop"
# Autostart at session login (same entry + autostart activation).
cp "$DESKTOP_TMP" "$AUTOSTART_DIR/linux-link.desktop"
printf 'X-GNOME-Autostart-enabled=true\n' >> "$AUTOSTART_DIR/linux-link.desktop"
rm -f "$DESKTOP_TMP"

say "Right-click \"Send to phone\" menu (Nautilus)"
mkdir -p "$NAUTILUS_DIR"
if [ -f "desktop/Send to phone" ]; then
    install -m755 "desktop/Send to phone" \
        "$NAUTILUS_DIR/Send to phone" || \
        warn "Nautilus script not installed (optional)"
fi

# Launch the icon immediately if we are in a graphical session.
if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    pkill -x linux-link-gui 2>/dev/null || true
    ( "$BIN_DIR/linux-link-gui" >/dev/null 2>&1 & ) || true
    say "Icon launched — check the system tray (corner of the screen)."
fi

echo
say "Done ✔"
echo "  • Daemon        : systemctl --user status linkd"
echo "  • System icon   : relaunched at every session login"
echo "  • Pair          : click the icon → \"Pair a device…\""
echo
warn "Pure GNOME (not Zorin): install the \"AppIndicator and KStatusNotifierItem Support\" extension to see the icon."
