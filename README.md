<p align="center">
  <img src="logo.png" alt="Linux Link" width="120">
</p>

<h1 align="center">Linux Link</h1>

<p align="center">
  <b>Your Android phone and your Linux PC, finally acting like they know each other.</b>
</p>

---

## Why this exists

I kept watching Apple users copy something on their Mac and paste it on their iPhone like it was nothing, and I wanted that on my own setup — a Linux PC and an Android phone. KDE Connect gets partway there, but I wanted something that just *works*: you pair once, and from then on the phone connects by itself every time the PC turns on. No opening an app, no clicking "refresh", no cloud account in the middle.

So I built Linux Link. Everything stays on your local network, everything is end-to-end encrypted, and there is no server anywhere — your devices talk directly to each other, or not at all.

## What it does

Once paired, without you touching anything:

- **Your phone's notifications show up on your desktop**, with a Reply button for messages (WhatsApp, SMS, Telegram…). Dismiss on one side, gone on the other.
- **The clipboard is shared both ways.** Copy on the PC, paste on the phone two seconds later. The other direction uses Shizuku for a fully automatic experience (Android blocks background clipboard reading otherwise — their rule, not mine).
- **Files fly both ways**, AirDrop-style: right-click → "Send to phone" on the PC, Share → Linux Link on the phone. Streamed, encrypted, checksummed.
- **A LinuxLink folder syncs in both directions**, plus your standard folders (Downloads, Documents, Pictures) if you want.
- **Handoff**: send the page you're reading from one device and it opens on the other.
- **Media and volume**: control the PC's player from a media bubble on your phone, use the physical volume keys, see the track title. Works the other way too.
- **Your phone becomes a webcam and a microphone** for Zoom, Meet, OBS.
- **Proximity lock**: walk away with your phone and the PC locks; come back and it unlocks.
- **Battery level** of the phone, visible from the PC's tray icon.
- **Several phones at once**, each recognized by its certificate.

On the PC all of this lives in a small tray icon — green dot when the phone is there, menu for quick actions, a proper pairing window with a QR code, and a settings window for paired devices, autostart and shortcuts. Three global keyboard shortcuts (`Super+Shift+V` to push the clipboard, `Super+Shift+B` to send a file, `Super+Shift+Space` for play/pause) are set up for you. On Android it's a Material You app that follows your wallpaper colors.

## How it works, in one paragraph

The PC runs a small Rust daemon (`linkd`). It advertises itself over BLE and mDNS; the phone recognizes it through Android's CompanionDeviceManager and connects over QUIC on your Wi-Fi. The very first time, you scan a QR code: that's when both sides exchange self-signed certificates and pin each other's SHA-256 fingerprint. After that it's mutual TLS 1.3 between two devices that trust only each other. No account, no relay, no telemetry — turn off your router's internet and everything still works.

## Installing

### PC side

You need Rust (`rustup`), and BlueZ (already there on almost every distro). Then:

```bash
./install.sh
```

The script figures out your distro (`apt`, `dnf` or `pacman`), offers to install the few runtime tools it needs, builds the four binaries, and wires everything up: the daemon as a systemd user service, the tray icon at session login, the app icon, the settings window in your application menu, the right-click "Send to phone" in whichever file manager you use, the global keyboard shortcuts, and Hyprland autostart if you're on Hyprland. Tested on GNOME/Zorin, KDE Plasma and Hyprland, across Debian/Ubuntu, Fedora and Arch.

| Desktop | Tray icon | Right-click "Send to phone" | Keyboard shortcuts |
|---|---|---|---|
| GNOME / Zorin | needs the *AppIndicator Support* extension on pure GNOME | Nautilus | yes |
| KDE Plasma | works out of the box | Dolphin | yes |
| Cinnamon | works out of the box | Nemo | — |
| MATE | works out of the box | Caja | — |
| XFCE | works out of the box | Thunar | — |
| Hyprland | shows up in waybar's `tray` module | — | yes |

On GNOME the shortcuts are registered as named custom keybindings, so your own shortcuts are never overwritten, and `linkd shortcuts remove` takes only ours back out. Skip that step entirely with `./install.sh --no-shortcuts`.

Reinstall without recompiling with `./install.sh --no-build`. Check on the daemon with `systemctl --user status linkd`.

### Phone side

Open the `android/` folder in Android Studio (not `android/app`), let Gradle sync, and run it on a real phone — BLE doesn't work in the emulator.

Then pair: click the tray icon → **"Pair a device…"**, scan the QR from the app, accept the "Associate the device" dialog Android shows you. That's it, and that's the last time you'll think about it. The real test is turning the PC off and on again: the "Connected" notification should appear on the phone without you opening anything.

### Bits that need a one-time setup

A few features touch parts of the system that need your explicit blessing once:

- **Webcam**: `sudo apt install v4l2loopback-dkms ffmpeg`, then load the module (`sudo modprobe v4l2loopback card_label="Linux Link" exclusive_caps=1` — or drop it in `/etc/modules-load.d/` to make it permanent).
- **Automatic phone→PC clipboard**: install [Shizuku](https://shizuku.rikka.app/) on the phone and connect it in the app. Without it, the quick-settings tile and Share menu still work as a manual fallback.
- **Proximity unlock**: locking works everywhere; unlocking may need a small polkit rule depending on your distro — see [`docs/`](docs/) for the three lines to add.

## Where the project stands

The core is solid on my machines (Zorin PC, Honor phone): pairing, auto-reconnect on boot, notifications, clipboard, files, media, webcam. v3.0 is where reliability and idle cost got taken seriously — reconnection is now driven by network events rather than timers, and the daemon no longer polls anything while nothing is happening. That work is fresh, and long-run behaviour across suspend cycles and Wi-Fi changes is exactly what I'd like other people's machines to tell me about. If something breaks for you, open an issue — the logs from `journalctl --user -u linkd` are usually what I'll ask for.

<details>
<summary><b>Version history</b></summary>

- **v3.0** — reliability, idle cost and desktop integration. Reconnection reacts to network changes and wake-from-suspend instead of waiting for a timer; every polling loop in the daemon is gone, replaced by an internal event bus, and the phone watches folders with inotify rather than walking the filesystem every five minutes. On the desktop: global keyboard shortcuts on GNOME, KDE and Hyprland, a settings window, and "Send to phone" in Nautilus, Nemo, Caja, Dolphin and Thunar, with a per-device submenu when several phones are connected.
- **v2.0** — the app got a face: launcher icon on Android (adaptive + Material You monochrome), app icon on the PC, a real graphical pairing window instead of the terminal, a custom dark scan screen on the phone, and the multi-distro installer (apt/dnf/pacman, Dolphin action, Hyprland autostart).
- **v0.52** — the Android app went Material You: dynamic colors from your wallpaper, light/dark, Pixel-style layout.
- **v0.51** — multi-device: several phones at once, hot pairing without restarting the service, per-device file sending.
- **v0.50** — the tray icon on the PC: status, battery, quick actions, pairing from the menu.
- **v0.41** — sync of standard folders (Downloads, Documents, Pictures), redesigned app, streamed transfers.
- **v0.40** — two-way LinuxLink folder sync, last-modified wins.
- **v0.31 / v0.30** — the phone as microphone and webcam.
- **v0.21 / v0.20** — "Send to phone" in the right-click menu, image clipboard PC→phone, Do Not Disturb sync.
- **v0.14 / v0.13** — battery on the PC, proximity lock.
- **v0.12** — AirDrop-style file transfer.
- **v0.11 / v0.10 / v0.9 / v0.8** — media controls, media bubble, volume both ways.
- **v0.7** — handoff of web pages.
- **v0.6** — quick reply to messages from the desktop.
- **v0.5** — reliable automatic clipboard via Shizuku.
- **v0.4 → v0.1** — automatic phone→PC clipboard via overlay, shared clipboard, notification mirroring, and the foundation: QUIC + pinned certificates, BLE wake, QR pairing.

</details>

## Protocol, for the curious

QUIC with ALPN `linuxlink/1`, mutual TLS 1.3, self-signed certificates pinned by SHA-256 fingerprint at pairing (trust-on-first-use via the QR code). Messages are JSON, one per line, over a bidirectional stream. The BLE advertisement (service UUID `4c4c0001-…`) carries the PC's IPv4 and QUIC port, so the phone can connect even on networks that block mDNS multicast.

## License

GPL-3.0 — see [LICENSE](LICENSE).
