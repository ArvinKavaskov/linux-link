#!/usr/bin/env bash
#
# Linux Link — PC-side installation.
#
# Supports GNOME / KDE Plasma / Hyprland and apt / dnf / pacman based
# distributions (Debian, Ubuntu, Zorin, Fedora, Arch, …).
#
# Installs the binaries (`linkd` daemon, `linux-link-gui` tray app,
# `linux-link-pair` pairing window, `linux-link-settings` settings window),
# the systemd user service, the app icon, the desktop entries, autostart,
# the file-manager "Send to phone" actions (Nautilus, Nemo, Caja, Dolphin,
# Thunar) and the global keyboard shortcuts.
#
# Usage:
#   ./install.sh                 # build then install everything
#   ./install.sh --no-build      # reuse the already compiled binaries
#   ./install.sh --no-shortcuts  # do not touch the keyboard shortcuts
#
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
AUTOSTART_DIR="$HOME/.config/autostart"
SYSTEMD_DIR="$HOME/.config/systemd/user"
NAUTILUS_DIR="$HOME/.local/share/nautilus/scripts"
NAUTILUS_EXT_DIR="$HOME/.local/share/nautilus-python/extensions"
NEMO_SCRIPT_DIR="$HOME/.local/share/nemo/scripts"
NEMO_ACTION_DIR="$HOME/.local/share/nemo/actions"
CAJA_SCRIPT_DIR="$HOME/.config/caja/scripts"
THUNAR_DIR="$HOME/.config/Thunar"
KDE_MENU_DIR="$HOME/.local/share/kio/servicemenus"

say()  { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m/!\\\033[0m %s\n' "$*"; }

ROOT="$(cd "$(dirname "$0")" && pwd)"

# ---------------------------------------------------------------- distro
PKG=""
if command -v apt-get >/dev/null 2>&1; then PKG="apt"
elif command -v dnf   >/dev/null 2>&1; then PKG="dnf"
elif command -v pacman >/dev/null 2>&1; then PKG="pacman"
fi
DISTRO="$(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}" || echo unknown)"
say "Distribution: $DISTRO (package manager: ${PKG:-unknown})"

# Runtime tools the daemon relies on -> package name per manager.
missing=()
need() { command -v "$1" >/dev/null 2>&1 || missing+=("$2"); }
need playerctl playerctl
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    need wl-paste wl-clipboard
else
    need xclip xclip
fi
case "$PKG" in
    apt)    need pactl pulseaudio-utils ;;
    dnf)    need pactl pulseaudio-utils ;;
    pacman) need pactl libpulse ;;
    *)      need pactl pulseaudio-utils ;;
esac
# Second screen: encoder tools, per session type and desktop.
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    case "$PKG" in
        apt)    need gst-launch-1.0 gstreamer1.0-tools
                need gst-inspect-1.0 gstreamer1.0-plugins-ugly ;;
        dnf)    need gst-launch-1.0 gstreamer1-plugins-base ;;
        pacman) need gst-launch-1.0 gstreamer
                need gst-inspect-1.0 gst-plugins-ugly ;;
    esac
    case "${XDG_CURRENT_DESKTOP:-}" in
        *Hyprland*|*sway*|*Sway*) need wf-recorder wf-recorder ;;
        *KDE*) need krfb-virtualmonitor krfb ;;
    esac
    # PipeWire element for gst (package names differ everywhere).
    if command -v gst-inspect-1.0 >/dev/null 2>&1 && ! gst-inspect-1.0 --exists pipewiresrc 2>/dev/null; then
        case "$PKG" in
            apt)    missing+=("gstreamer1.0-pipewire") ;;
            dnf)    missing+=("pipewire-gstreamer") ;;
            pacman) missing+=("gst-plugin-pipewire") ;;
        esac
    fi
else
    need ffmpeg ffmpeg
fi

if [ "${#missing[@]}" -gt 0 ]; then
    warn "Missing tools: ${missing[*]}"
    case "$PKG" in
        apt)    CMD="sudo apt-get install -y ${missing[*]}" ;;
        dnf)    CMD="sudo dnf install -y ${missing[*]}" ;;
        pacman) CMD="sudo pacman -S --needed ${missing[*]}" ;;
        *)      CMD="" ;;
    esac
    if [ -n "$CMD" ] && [ -t 0 ]; then
        read -r -p "Install them now? [Y/n] " a
        case "${a:-Y}" in [Yy]*) $CMD || warn "Installation failed — continuing anyway." ;; esac
    elif [ -n "$CMD" ]; then
        warn "Install them with: $CMD"
    fi
fi

# ---------------------------------------------------------------- build
cd "$ROOT/linkd"
BUILD=1
SHORTCUTS=1
for arg in "$@"; do
    case "$arg" in
        --no-build)     BUILD=0 ;;
        --no-shortcuts) SHORTCUTS=0 ;;
    esac
done

if [ "$BUILD" = 1 ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        warn "Rust is not installed. Install it with:"
        warn "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi
    say "Building (release, may take a few minutes)…"
    cargo build --release \
        --bin linkd --bin linux-link-gui --bin linux-link-pair --bin linux-link-settings
fi

for b in linkd linux-link-gui linux-link-pair linux-link-settings; do
    if [ ! -x "target/release/$b" ]; then
        warn "Binary $b not found. Run without --no-build to compile it."
        exit 1
    fi
done

say "Installing the binaries in $BIN_DIR"
mkdir -p "$BIN_DIR"
install -m755 target/release/linkd            "$BIN_DIR/linkd"
install -m755 target/release/linux-link-gui   "$BIN_DIR/linux-link-gui"
install -m755 target/release/linux-link-pair  "$BIN_DIR/linux-link-pair"
install -m755 target/release/linux-link-settings "$BIN_DIR/linux-link-settings"

say "App icon"
mkdir -p "$ICON_DIR"
install -m644 "$ROOT/logo.png" "$ICON_DIR/linux-link.png"
command -v gtk-update-icon-cache >/dev/null 2>&1 && \
    gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

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
Icon=linux-link
Terminal=false
Categories=Network;Utility;
StartupNotify=false
StartupWMClass=linux-link
EOF
install -m644 "$DESKTOP_TMP" "$APP_DIR/linux-link.desktop"
# Autostart at session login (same entry + autostart activation).
cp "$DESKTOP_TMP" "$AUTOSTART_DIR/linux-link.desktop"
printf 'X-GNOME-Autostart-enabled=true\n' >> "$AUTOSTART_DIR/linux-link.desktop"
rm -f "$DESKTOP_TMP"

say "Settings window (menu entry)"
cat > "$APP_DIR/linux-link-settings.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Linux Link Settings
GenericName=Android Continuity
Comment=Paired devices, proximity lock, autostart and keyboard shortcuts
Exec=$BIN_DIR/linux-link-settings
Icon=linux-link
Terminal=false
Categories=Settings;Network;Utility;
StartupNotify=true
StartupWMClass=linux-link
Keywords=phone;android;link;sync;
EOF
chmod 644 "$APP_DIR/linux-link-settings.desktop"
command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database "$APP_DIR" 2>/dev/null || true

# ------------------------------------------------- file managers
SENDER="$NAUTILUS_DIR/Send to phone"

say "Right-click \"Send to phone\" — Nautilus (GNOME)"
mkdir -p "$NAUTILUS_DIR"
if [ -f "desktop/Send to phone" ]; then
    install -m755 "desktop/Send to phone" "$SENDER" || \
        warn "Nautilus script not installed (optional)"
fi

# The python extension puts the entry at the top level of the menu instead of
# under "Scripts", and offers one line per phone when several are connected.
# The script above stays installed as the fallback when python3-nautilus is not.
if [ -f "desktop/send_to_phone.py" ]; then
    if python3 -c "import gi; gi.require_version('Nautilus', '4.0')" >/dev/null 2>&1 || \
       python3 -c "import gi; gi.require_version('Nautilus', '3.0')" >/dev/null 2>&1; then
        mkdir -p "$NAUTILUS_EXT_DIR"
        install -m644 "desktop/send_to_phone.py" "$NAUTILUS_EXT_DIR/send_to_phone.py"
        say "  … top-level entry installed (nautilus-python)"
        pkill -f "nautilus --gapplication-service" 2>/dev/null || true
    else
        case "$PKG" in
            apt)    warn "  For a top-level entry: sudo apt install python3-nautilus" ;;
            dnf)    warn "  For a top-level entry: sudo dnf install nautilus-python" ;;
            pacman) warn "  For a top-level entry: sudo pacman -S python-nautilus" ;;
        esac
    fi
fi

if command -v nemo >/dev/null 2>&1; then
    say "Right-click \"Send to phone\" — Nemo (Cinnamon)"
    mkdir -p "$NEMO_SCRIPT_DIR" "$NEMO_ACTION_DIR"
    install -m755 "desktop/Send to phone" "$NEMO_SCRIPT_DIR/Send to phone"
    sed "s|<SENDER>|$NEMO_SCRIPT_DIR/Send to phone|" \
        "desktop/linux-link-send.nemo_action" \
        > "$NEMO_ACTION_DIR/linux-link-send.nemo_action"
    chmod 644 "$NEMO_ACTION_DIR/linux-link-send.nemo_action"
fi

if command -v caja >/dev/null 2>&1; then
    say "Right-click \"Send to phone\" — Caja (MATE)"
    mkdir -p "$CAJA_SCRIPT_DIR"
    install -m755 "desktop/Send to phone" "$CAJA_SCRIPT_DIR/Send to phone"
fi

if command -v thunar >/dev/null 2>&1; then
    say "Right-click \"Send to phone\" — Thunar (XFCE)"
    mkdir -p "$THUNAR_DIR"
    # uca.xml is the user's own file and may already hold their custom actions,
    # so we parse it, replace only our entry, and write it back.
    UCA="$THUNAR_DIR/uca.xml" SENDER="$SENDER" python3 - <<'PY' || warn "Thunar action not installed (optional)"
import os
import xml.etree.ElementTree as ET

path = os.environ["UCA"]
sender = os.environ["SENDER"]
UNIQUE = "linux-link-send-to-phone"

if os.path.exists(path):
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError:
        # A malformed uca.xml is the user's problem, not ours to overwrite.
        raise SystemExit(1)
else:
    root = ET.Element("actions")

for action in list(root.findall("action")):
    node = action.find("unique-id")
    if node is not None and node.text == UNIQUE:
        root.remove(action)

action = ET.SubElement(root, "action")
for tag, text in [
    ("icon", "linux-link"),
    ("name", "Send to phone"),
    ("submenu", ""),
    ("unique-id", UNIQUE),
    ("command", '"%s" %%F' % sender),
    ("description", "Send the selection to the phone via Linux Link"),
    ("range", "*"),
    ("patterns", "*"),
]:
    ET.SubElement(action, tag).text = text
# No <directories/>: the action only makes sense on files, and Thunar hides it
# everywhere it is not listed.
for kind in ("audio-files", "image-files", "other-files",
             "text-files", "video-files"):
    ET.SubElement(action, kind)

ET.ElementTree(root).write(path, encoding="UTF-8", xml_declaration=True)
PY
fi

say "Right-click \"Send to phone\" — Dolphin (KDE)"
mkdir -p "$KDE_MENU_DIR"
cat > "$KDE_MENU_DIR/linux-link.desktop" <<EOF
[Desktop Entry]
Type=Service
ServiceTypes=KonqPopupMenu/Plugin
MimeType=all/allfiles;
Actions=sendToPhone;
X-KDE-Priority=TopLevel
Icon=linux-link

[Desktop Action sendToPhone]
Name=Send to phone
Icon=linux-link
Exec=sh -c 'for f; do "$BIN_DIR/linkd" send-file "\$f"; done' _ %F
EOF
chmod 755 "$KDE_MENU_DIR/linux-link.desktop"

# ------------------------------------------------------- Hyprland autostart
HYPR_CONF="$HOME/.config/hypr/hyprland.conf"
if [ -f "$HYPR_CONF" ]; then
    say "Hyprland detected — autostart via exec-once"
    if ! grep -q "linux-link-gui" "$HYPR_CONF"; then
        {
            echo ""
            echo "# Linux Link — tray icon at session start"
            echo "exec-once = $BIN_DIR/linux-link-gui"
        } >> "$HYPR_CONF"
        say "Added to $HYPR_CONF"
    fi
    if ! command -v waybar >/dev/null 2>&1; then
        warn "No status bar with a system tray detected."
        warn "The tray icon needs an SNI host: install waybar and enable its \"tray\" module."
    fi
fi

# ------------------------------------------------------------- firewall
# Ubuntu and Zorin ship with no active firewall, but CachyOS enables ufw and
# Fedora enables firewalld out of the box — and both silently drop the phone's
# QUIC packets. "Pairing failed: PC unreachable" with the PC a metre away is
# almost always this.
open_firewall() {
    if command -v ufw >/dev/null 2>&1 && sudo ufw status 2>/dev/null | head -1 | grep -qi active; then
        say "Firewall: ufw is active — Linux Link needs UDP 47100 (QUIC) and 47101 (discovery)"
        printf '    Open them now? [Y/n] '
        read -r reply
        case "$reply" in
            [nN]*) warn "Skipped. The phone will NOT reach this PC until you run:"
                   warn "  sudo ufw allow 47100/udp && sudo ufw allow 47101/udp" ;;
            *)
                sudo ufw allow 47100/udp comment 'Linux Link QUIC' >/dev/null
                sudo ufw allow 47101/udp comment 'Linux Link discovery' >/dev/null
                sudo ufw allow from "$(ip -4 route show default 2>/dev/null | awk '{print $3}' | head -1 | sed 's/\.[0-9]*$/.0\/24/')" to any port 5353 proto udp comment 'mDNS' >/dev/null 2>&1 || true
                say "Ports opened in ufw."
                ;;
        esac
    elif command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active firewalld >/dev/null 2>&1; then
        say "Firewall: firewalld is active — Linux Link needs UDP 47100 (QUIC) and 47101 (discovery)"
        printf '    Open them now? [Y/n] '
        read -r reply
        case "$reply" in
            [nN]*) warn "Skipped. The phone will NOT reach this PC until you run:"
                   warn "  sudo firewall-cmd --permanent --add-port=47100/udp --add-port=47101/udp && sudo firewall-cmd --reload" ;;
            *)
                sudo firewall-cmd --permanent --add-port=47100/udp >/dev/null
                sudo firewall-cmd --permanent --add-port=47101/udp >/dev/null
                sudo firewall-cmd --permanent --add-service=mdns >/dev/null 2>&1 || true
                sudo firewall-cmd --reload >/dev/null
                say "Ports opened in firewalld."
                ;;
        esac
    fi
}
open_firewall

# ------------------------------------------------------- second screen input
# The tablet-as-second-screen feature injects mouse/keyboard through
# /dev/uinput on every desktop except GNOME (which has a D-Bus API for it).
# Stock permissions are root-only; this udev rule opens the node to the
# locally logged-in user (uaccess), which is the modern, narrow way to do it.
setup_uinput() {
    # GNOME Wayland does not need it — skip the sudo prompt there.
    case "${XDG_CURRENT_DESKTOP:-}" in
        *GNOME*|*Zorin*) [ -z "${WAYLAND_DISPLAY:-}" ] || return 0 ;;
    esac
    RULE_FILE=/etc/udev/rules.d/60-linuxlink-uinput.rules
    [ -f "$RULE_FILE" ] && return 0
    say "Second screen: allowing user access to /dev/uinput (udev rule)"
    printf '    Install the rule now? [Y/n] '
    read -r reply
    case "$reply" in
        [nN]*) warn "Skipped. The tablet's touch/keyboard will not reach the PC until you run:"
               warn "  echo 'KERNEL==\"uinput\", SUBSYSTEM==\"misc\", OPTIONS+=\"static_node=uinput\", TAG+=\"uaccess\"' | sudo tee $RULE_FILE"
               warn "  sudo udevadm control --reload && sudo udevadm trigger /dev/uinput" ;;
        *)
            echo 'KERNEL=="uinput", SUBSYSTEM=="misc", OPTIONS+="static_node=uinput", TAG+="uaccess"' | sudo tee "$RULE_FILE" >/dev/null
            sudo udevadm control --reload 2>/dev/null || true
            sudo modprobe uinput 2>/dev/null || true
            sudo udevadm trigger /dev/uinput 2>/dev/null || true
            say "udev rule installed (log out and back in if the second screen reports no input)."
            ;;
    esac
}
setup_uinput

# ---------------------------------------------------- keyboard shortcuts
if [ "$SHORTCUTS" = 1 ]; then
    say "Global keyboard shortcuts"
    if "$BIN_DIR/linkd" shortcuts install 2>/dev/null; then
        :
    else
        warn "Shortcuts not registered automatically on this desktop."
        warn "Add them from the settings window, or by hand:"
        "$BIN_DIR/linkd" shortcuts status 2>/dev/null || true
    fi
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
echo "  • Settings      : click the icon → \"Settings…\"  (or linux-link-settings)"
echo "  • Shortcuts     : Super+Shift+V clipboard · Super+Shift+B file · Super+Shift+Space play/pause"
echo
case "${XDG_CURRENT_DESKTOP:-}" in
    *GNOME*|*Zorin*)
        warn "Pure GNOME: install the \"AppIndicator and KStatusNotifierItem Support\" extension to see the icon." ;;
    *KDE*)
        say "KDE Plasma: the tray icon works out of the box." ;;
    *Hyprland*)
        warn "Hyprland: the icon appears in waybar's \"tray\" module." ;;
esac
