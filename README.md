<p align="center">
  <img src="logo.png" alt="Linux Link" width="120">
</p>

<h1 align="center">Linux Link</h1>

<p align="center">
  <b>Apple-like continuity between Linux and Android — fully local, end-to-end encrypted, no cloud.</b>
</p>

<p align="center">
  Auto-connect on boot · notifications & quick reply · shared clipboard · handoff · volume & media ·
  AirDrop-style file transfer · proximity lock · battery · folder sync · webcam · microphone ·
  multi-device · system-tray app · Material You Android UI
</p>

---

## New in v0.52: the Android app goes Material You 🎨

Third building block of v1.0: the phone interface adopts the
**Pixel / Material You** style.

- **Dynamic colors**: the app's palette is generated from your
  wallpaper (Android 12+). Change your wallpaper → the app follows.
- **Automatic light / dark theme** based on the system setting (with a purple
  fallback if dynamic colors are unavailable).
- **Pixel style**: large title bar that collapses on scroll, cards
  with nicely rounded corners, full-width tonal buttons.

Only the Android app changes; the PC daemon (v0.51) is unchanged — you only
rebuild the APK. Remember to apply the Gradle fix if Android Studio has
pushed Gradle 9/10 (see the note at the bottom).

## New in v0.51: multi-device 📱📱

Linux Link now handles **multiple phones/tablets** at the same time —
second building block of v1.0.

- **Hot pairing**: no more need to stop the service to add a
  device. "Pair a device…" (in the icon menu) opens the QR via
  the running daemon and confirms pairing right away. Under the hood:
  `linkd pair-live`.
- **The device list in the menu**: each paired device is shown
  with its status (● connected / ○ offline).
- **Targeted file sending**: when several devices are connected,
  "Send a file…" offers a submenu to choose the destination
  (in the CLI: `linkd send-file <file> --to <fingerprint>`).

Each device is recognized by its certificate fingerprint, so no
change to the Android app is needed — a phone that is already paired
keeps working, and the clipboard stays shared across all of them.

## New in v0.50: a real app on the PC side 🖥️

Until now the PC only had a faceless daemon (`linkd`) driven from the command
line. v0.50 adds **a system tray icon** (`linux-link-gui`)
— the first building block of v1.0.

At a glance in the corner of the screen, without opening any window:

- **Connection status**: green dot = phone connected, gray = waiting.
  The device name is shown, and the `(+N)` counter prepares for
  **multi-device**.
- **Phone battery** directly in the menu.
- **Quick actions**: send one or more file(s) (graphical picker),
  control media (play/pause, previous/next, volume), toggle
  **proximity lock**, and **pair a new device** without going
  through the terminal.

Under the hood: the daemon publishes its status in
`~/.config/linux-link/status.json` every 2 s; the icon re-reads it and stays
accurate, even after a service restart (and detects a stopped service).

### One-command install

```bash
./install.sh
```

Builds both binaries, installs the systemd service (daemon at session
startup), makes the icon appear automatically each time you log in,
adds the applications menu entry and the right-click "Send to
phone". Zorin OS shows system tray icons by default; on
pure GNOME, install the "AppIndicator and KStatusNotifierItem Support" extension.

To reinstall without recompiling: `./install.sh --no-build`.

## New in v0.41

- **Sync of your standard folders**: Downloads, Documents, Pictures
  (+ LinuxLink) sync in both directions. On the PC side these are your real
  folders (Downloads, Documents, Pictures — XDG names respected).
- **Redesigned interface**: clear sections (Device, PC Control,
  Clipboard, Continuity, Camera & mic), purple theme.
- **Optimized transfers**: files sent as a stream (no more loading into
  memory), per-folder baseline, reduced logging.

⚠️ Two-way sync: deleting a file on one side deletes it on the
other. The first pass merges (no deletions).

## v0.40: folder sync 🔄

A **LinuxLink** folder on each side syncs automatically, in
both directions (like iCloud Drive). The last-modified file wins in
case of a conflict.

- PC: `~/LinuxLink` (created automatically).
- Phone: `/storage/emulated/0/LinuxLink`. Requires "all
  files" access (granted once). In the app: **"Enable LinuxLink
  folder sync"** → allow.

Sync runs on connection then every ~20 s. Put a file in
`~/LinuxLink` on the PC → it appears in `/LinuxLink` on the phone, and
vice versa. Deletions and changes propagated on both sides.

Engine validated (adding on both sides, deletion, latest-wins conflict); to be
tested against your real usage.

## v0.31: the phone as a microphone 🎤

With the webcam, you get full video calls (phone camera + mic).
No extra prerequisites (pactl / pulseaudio-utils already installed for the
volume). In the app: 📷 Webcam → **🎤 Enable the microphone**. In
Zoom/Meet/OBS, choose the **"Linux Link"** microphone.

## v0.30: the phone as a webcam 📷

The phone becomes a real webcam for the PC (Zoom, OBS, browser…).

### PC prerequisites (once)
```bash
sudo apt install v4l2loopback-dkms ffmpeg
# Load the virtual webcam (redo after a reboot, or see below):
sudo modprobe v4l2loopback card_label="Linux Link" exclusive_caps=1
```
To load it automatically on every boot:
```bash
echo v4l2loopback | sudo tee /etc/modules-load.d/linuxlink.conf
echo 'options v4l2loopback card_label="Linux Link" exclusive_caps=1' | sudo tee /etc/modprobe.d/linuxlink.conf
```

### Usage
In the app: **📷 Webcam** button → Start. In Zoom/OBS/Meet, choose the
**"Linux Link"** camera. Button to switch front/rear.

⚠️ Heavy feature (real-time video streaming) — probably to be tuned
on your machine (resolution, latency, rotation). MJPEG streaming over encrypted QUIC.

## v0.21

### "Send to phone" DIRECTLY in the right-click menu
Direct entry (no longer under "Scripts") via a Nautilus extension:
```bash
sudo apt install python3-nautilus
mkdir -p ~/.local/share/nautilus-python/extensions
cp linkd/desktop/send_to_phone.py ~/.local/share/nautilus-python/extensions/
nautilus -q
```

### Universal Clipboard++: images (PC → phone)
Copy an image or take a screenshot on the PC → it passes
automatically into the phone's clipboard → paste it into WhatsApp,
an email, etc. (The phone → PC direction for images stays manual via
"Share", because of the Android restriction on reading the
clipboard in the background — same as for text.)

## v0.20

### "Send to phone" on right-click
Install the Nautilus script:
```bash
mkdir -p ~/.local/share/nautilus/scripts
cp "linkd/desktop/Send to phone" ~/.local/share/nautilus/scripts/
chmod +x ~/.local/share/nautilus/scripts/"Send to phone"
nautilus -q   # restart Files
```
Then: right-click on one or more file(s) → **Scripts → Send to phone**.
(Nemo: `~/.local/share/nemo/scripts/`.)

### "Do Not Disturb" sync
When you enable DND on the phone (or the PC), the other follows automatically.
In the app: **"Enable Do Not Disturb sync"** → grant access in
the Android settings. On the PC side, it drives GNOME's DND.

## v0.14: phone battery on the PC 🔋

The phone sends its battery level to the PC (on connection and on every
change of %). To check it:

```bash
linkd battery      # e.g.: 🔋 Phone: 73% (charging)
```

## v0.13: proximity lock 🔒

The PC locks when your phone moves away (is no longer reachable
for ~20 s) and unlocks when you return. Based on the presence of the
connection (reliable), not on RSSI. **Opt-in**, disabled by default:

```bash
linkd proximity-lock on     # enable
linkd proximity-lock off    # disable
```

Important notes:
- **Locking** works everywhere (loginctl, freedesktop ScreenSaver fallback).
- Auto **unlocking** may require a polkit rule depending on the distro.
  If unlocking fails (log "unlock refused"), create
  `/etc/polkit-1/rules.d/49-linuxlink-unlock.rules`:

  ```javascript
  polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.login1.unlock-session" &&
        subject.local && subject.active) {
      return polkit.Result.YES;
    }
  });
  ```
- Security: anyone with your phone within range unlocks your PC. Enable
  it knowingly.

## v0.12: AirDrop-style file transfer 📎

- **Phone → PC**: Share any file (photo, PDF…) →
  **Linux Link** → it arrives in the PC's ~/Downloads (notification).
- **PC → phone**: `linkd send-file <path>` → the file arrives in the
  phone's Downloads, with a clickable notification to open it.
- Streamed transfer (large files OK), encrypted over QUIC, integrity verified
  (identical MD5 sums in testing).

## v0.11: title in the media bubble + optimizations

- The **media bubble** now shows the **title and artist** of what's
  playing on the PC (Spotify, YouTube, VLC…), and the ▶/⏸ icon follows the real state.
  The daemon watches playerctl and pushes the info to the phone.
- App: **scrollable** screen (the button list gets longer), play/pause
  state refresh.

## v0.10: media bubble + physical volume buttons

When the PC is connected, a "PC Media" media session appears on the
phone. Without opening the app, it gives you:
- a **media bubble in the control panel** (and on the lock screen)
  with ⏮ ⏯ ⏭ that drive the PC's player;
- the phone's **physical volume buttons** that adjust the PC's sound
  (as long as this session is the active media session).

The "Clipboard → PC" control-panel tile already exists (v0.3).

## v0.9: media control ⏯

- **Phone → PC**: ⏮ ⏯ ⏭ buttons in the app drive the PC's player
  (Spotify, VLC, YouTube in the browser… — via playerctl/MPRIS).
  PC prerequisite: `sudo apt install playerctl`.
- **PC → phone**: `linkd media play_pause|next|previous` drives the
  phone's active player (via MediaSession, uses notification access).

## v0.8: volume control 🔊

- **Phone → PC**: three buttons in the app (🔉− / 🔇 / 🔊+) adjust the
  PC's sound (via pactl, PulseAudio/PipeWire).
- **PC → phone**: `linkd phone-volume up|down|mute|set <0-100>` sets the
  phone's media volume (with the on-screen slider display).

## v0.7: handoff (resume a web page) ↔

- **Phone → PC**: in the phone's browser, Share → **Linux
  Link** → an "Open: [title]" notification appears on the desktop →
  click → the page opens in your browser.
- **PC → phone**: `linkd send-url "https://…"` (or without an argument = reads
  the URL from the clipboard) → a notification appears on the phone →
  tap → the page opens. A `desktop/linuxlink-send.desktop` launcher lets
  you turn it into a keyboard shortcut or an "open with".

## v0.6: quick reply from the PC ↩

Messaging notifications (WhatsApp, SMS, Telegram…) show a
**"Reply"** button on the desktop. Click → a small window opens (zenity) →
you type → the reply goes out through the original app on the phone.

PC prerequisite: `zenity` (often already there; otherwise `sudo apt install zenity`).
New button in the app: **"Connect now"** to force the
connection without waiting for the Bluetooth cycle.

## v0.5: RELIABLE automatic clipboard via Shizuku

Android 13+ deliberately blocks reading the clipboard in the background
(Google: "working as intended"). The only truly reliable path, used by
Tasker & co, is **Shizuku**: a free app that grants Linux Link
shell-level access to the clipboard.

Getting started on the phone side:
1. Install **Shizuku** (Play Store or GitHub).
2. Start it via **wireless debugging** (Android 11+, no PC) — the
   Shizuku app guides you step by step; or once over cable with `adb`.
3. In Linux Link: "Connect Shizuku" → allow.
Only thing to redo: restart Shizuku after a phone reboot.

Without Shizuku, the app falls back to the overlay (unreliable on strict OEMs); the
"Clipboard → PC" tile stays the 100% reliable fallback.

# Linux Link — prototype v0.4

## New in v0.4: phone → PC clipboard AUTOMATIC

No more need to press a button. In the app: "Enable automatic
clipboard" → allow "Display over other apps"
(once, it persists) → from now on everything you copy on the
phone arrives on the PC by itself.

How: Android only allows reading the clipboard in the background
for apps that have a visible window. The app shows an invisible window
(1 px transparent) that unlocks this right — without showing anything on screen, without
a cable, without root, and without having to redo the trick on every restart
(unlike KDE Connect's ADB hack). The tile and sharing
stay available as a fallback.

## v0.3: shared clipboard 📋

- **PC → phone: automatic.** You copy on the PC, it's pasteable on
  the phone within 2 seconds. PC prerequisite: `wl-clipboard` (Wayland)
  or `xclip` (X11) — `sudo apt install wl-clipboard xclip`.
- **Phone → PC: two gestures.** "Clipboard → PC" tile to add
  in the quick settings (panel → pencil → drag the tile), or
  "Share → Linux Link" from any app, or the button in the
  app. (Android forbids automatic background reading for all
  apps — that's the platform's limit, not the app's.)
- PC-only test: `testclient --listen 30` in a terminal (simulates the
  phone), copy some text on the PC → it appears via push.

## v0.2: notification mirroring 🔔

The phone's notifications appear on the Linux desktop (standard
freedesktop — GNOME, KDE, Cinnamon, XFCE…), update instead of
stacking up, and disappear from the PC when you dismiss them on the phone.

Getting started: recompile both sides, then in the app tap
**"Enable notification mirroring"** and check Linux Link in the
Android settings that open. Quick test without a phone:
`testclient --notify "Title:Message body"`.

---


Linux ↔ Android continuity: you pair once, then the phone connects
by itself as soon as the PC turns on (BLE wake via
Android's CompanionDeviceManager). See `docs/` and the project for
the complete architecture.

```
linux-link/
├── linkd/      Linux daemon (Rust): BLE + mDNS advertising, QUIC server,
│               QR code pairing, v0.1 protocol (hello/ping).
│               ✅ Compiled and tested (pairing, reconnection, rejections).
└── android/    Android app (Kotlin): CDM pairing + automatic wake.
                ⚠️ Complete skeleton, to be compiled in Android Studio.
```

## 1. PC side: compile and run the daemon

Prerequisites: Rust (rustup), `libdbus-1-dev` (Debian/Ubuntu) or `dbus-devel`
(Fedora), BlueZ active (installed by default on almost all distros).

```bash
cd linkd
cargo build --release

# First run: pairing mode (shows the QR code)
./target/release/linkd pair

# Subsequent runs: normal mode
./target/release/linkd run

# Without a Bluetooth adapter (mDNS-only fallback, no auto wake):
./target/release/linkd run --no-ble

# See the PC's identity and the paired phones:
./target/release/linkd status
```

### Automatic start with the session (the whole point)

```bash
cp target/release/linkd ~/.local/bin/
mkdir -p ~/.config/systemd/user
cp systemd/linkd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now linkd
```

### Testing without a phone

A test client that simulates the phone is included:

```bash
# Terminal 1
./target/release/linkd pair --no-ble        # note the displayed token

# Terminal 2
./target/release/testclient --token <TOKEN>   # pairing + 3 pings
./target/release/testclient                    # reconnection (hello) + pings
```

## 2. Phone side: compile the app

Open `android/` in Android Studio (Ladybug or newer), let
Gradle sync, then Run on a physical phone (BLE does not
work in the emulator).

Flow in the app:
1. **Pair a PC** → scan the QR shown by `linkd pair` → the app
   connects over QUIC on Wi-Fi, exchanges the token, pins the certificate.
2. Android shows the **"Associate the device"** dialog (it's the
   CompanionDeviceManager that spotted the PC's BLE advertisement) → accept.
3. **Test the connection (ping)** to check.
4. The real test: turn off the PC, turn it back on, and watch the notification
   "Connected to <your PC>" appear **without opening the app**.

## 3. The v0.1 protocol (summary)

- Transport: QUIC (ALPN `linuxlink/1`), mutual TLS 1.3, self-signed
  certificates pinned by SHA-256 fingerprint at pairing (TOFU
  via the QR code). No cloud, no account.
- Messages: JSON, one per line, over a bidirectional stream:
  `pair_request` / `pair_ok` / `pair_rejected`, `hello` / `hello_ok` /
  `not_trusted`, `ping` / `pong`.
- BLE advertisement: service UUID `4c4c0001-6c69-6e75-786c-696e6b000001`,
  service data = PC's IPv4 (4 bytes) + QUIC port (2 bytes BE) → the app
  connects directly, even if the network blocks mDNS multicast.

## 4. State and known limits of the prototype

- The Rust daemon is **compiled and tested** (pairing, reconnection,
  rejection of invalid tokens and unpaired devices). The
  BLE advertisement could not be tested in the development
  environment (no Bluetooth adapter) — to be validated on a real machine.
- The Android app is a complete skeleton but **not compiled here**: the
  exact names of the kwik builder API (`clientCertificate` /
  `clientCertificateKey`) and the variant of `startObservingDevicePresence`
  depending on the Android version are the two points to adjust first
  if compilation snags.
- v0.1 = pairing + wake + ping. The notifications, clipboard
  and handoff modules arrive in v1 (see the architecture document).
