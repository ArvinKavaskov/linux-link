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
- **Your tablet becomes a true second monitor** — extension, not mirroring, like Apple's Sidecar. The PC grows a real extra display that shows up in your display settings; drag windows onto it, the cursor flows across the edge, and the tablet's touch, pen (with pressure) and any attached keyboard act on the PC. H.264 over the same encrypted link, reconnects by itself. Works over Wi-Fi, or over USB via Android's USB tethering.
- **Your phone becomes a webcam, a microphone or a speaker.** The mic and the speaker appear in your PC's sound settings as ordinary devices — "Linux Link" as an input, "Phone (Linux Link)" as an output — so you pick them exactly where you'd pick a headset, in the volume applet or in Zoom/Meet/OBS. Flip the toggle in the app and the phone keeps working with its screen off.
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

#### Firewall

If pairing fails with **"PC unreachable"** while both devices sit on the same Wi-Fi, it is almost always the firewall eating the phone's packets. Linux Link needs two UDP ports open on the PC: **47100** (the QUIC connection itself) and **47101** (the discovery beacon that lets the phone find the PC again after an IP change). Allowing mDNS (UDP 5353) as well makes rediscovery faster.

Whether you have a firewall at all depends on the distro:

| Distro | Default firewall | Open the ports |
|---|---|---|
| Ubuntu, Zorin, Debian, Mint | none active — nothing to do | — |
| CachyOS (and some Arch spins) | **ufw, active** | `sudo ufw allow 47100/udp && sudo ufw allow 47101/udp` |
| Fedora, openSUSE | **firewalld, active** | `sudo firewall-cmd --permanent --add-port=47100/udp --add-port=47101/udp --add-service=mdns && sudo firewall-cmd --reload` |
| Arch (vanilla) | none unless you added one | — |

`install.sh` detects an active ufw or firewalld and offers to open the ports for you. The symptom of a closed firewall is distinctive: the PC can ping the phone, `linkd` runs fine, the QR scans — and the connection still times out, because the firewall drops the incoming packets silently.

One more pairing gotcha: if the QR ever shows an address like `172.16.x.x`, that was a VPN or container interface. Since v3.1 those are filtered out of the QR automatically; if you're on an older build, disconnect the VPN, generate a new QR, and re-scan.

### Phone side

Open the `android/` folder in Android Studio (not `android/app`), let Gradle sync, and run it on a real phone — BLE doesn't work in the emulator.

Then pair: click the tray icon → **"Pair a device…"**, scan the QR from the app, accept the "Associate the device" dialog Android shows you. That's it, and that's the last time you'll think about it. The real test is turning the PC off and on again: the "Connected" notification should appear on the phone without you opening anything.

#### Android permissions — what to grant and why

Android guards each capability behind its own permission, so the app asks for them as you enable features rather than all at once. Here is the full list, so nothing surprises you:

| When | Permission | Why |
|---|---|---|
| First launch | Notifications | the "Connected" status and transfer notifications |
| Pairing | Camera | scanning the QR code |
| Pairing | "Associate the device" (companion dialog) | this is the big one: it lets Android wake the app when the PC appears, without the app running — the whole "never open the app" promise rests on it |
| Notification mirror | Notification access (special settings page) | reading your notifications to forward them to the PC |
| Folder sync | All-files access (special settings page) | reading/writing the synced folders |
| Do Not Disturb sync | DND access (special settings page) | mirroring DND state both ways |
| Phone as microphone | Microphone | obvious, but note the PC never gets audio unless you flip the toggle |
| Phone as speaker | none | playback needs no permission |
| Automatic clipboard | Display over other apps, or Shizuku | Android forbids background clipboard reading; the overlay (or Shizuku) is the workaround |

On Honor/MagicOS and some other OEM Androids, also mark Linux Link as **"protected"** or disable battery optimization for it (Settings → Battery), otherwise the OEM's task killer will quietly stop the background connection and reconnection will feel random.

### Bits that need a one-time setup

A few features touch parts of the system that need your explicit blessing once:

- **Webcam**: `sudo apt install v4l2loopback-dkms ffmpeg`, then load the module (`sudo modprobe v4l2loopback card_label="Linux Link" exclusive_caps=1` — or drop it in `/etc/modules-load.d/` to make it permanent).
- **Automatic phone→PC clipboard**: install [Shizuku](https://shizuku.rikka.app/) on the phone and connect it in the app. Without it, the quick-settings tile and Share menu still work as a manual fallback.
- **Proximity unlock**: locking works everywhere; unlocking may need a small polkit rule depending on your distro — see [`docs/`](docs/) for the three lines to add.

### Second screen

Tap **"Use as a second screen"** in the app — or, better, click it in the PC's tray menu and never touch the tablet at all — and the PC creates a virtual monitor sized for the tablet, placed next to your real one. From there it is just a monitor: arrange it in your display settings, drag windows onto it, present from it. Close the app (or walk out of range) and the monitor folds back up; come back and it reconnects on its own.

How the monitor gets created depends on the desktop, and so does the small one-time setup:

| Desktop | Virtual monitor via | One-time setup |
|---|---|---|
| GNOME / Zorin (Wayland) | Mutter's own remote-desktop API | none — fully automatic |
| KDE Plasma (Wayland) | `krfb-virtualmonitor` + screencast portal | install `krfb`; approve the screen picker once (remembered afterwards) |
| Hyprland / Sway | headless output + `wf-recorder` | install `wf-recorder` |
| Most desktops on X11 | forced mode on a spare port, or `xrandr` virtual region + ffmpeg | none |

One exception: **GNOME on X11** cannot host a virtual monitor — its window manager reverts an enlarged framebuffer and ignores outputs it believes disconnected, so Linux Link refuses cleanly and tells you the fix: log out and pick the Wayland session (the gear icon on the login screen), where the second screen works through GNOME's own API, one line up in the table.

`install.sh` offers to install the right tools for your setup. On everything except GNOME Wayland, input (touch, pen, keyboard) is injected through `/dev/uinput`; the installer drops a udev rule so your user may open it — if the video shows but touches do nothing, log out and back in once so the rule applies to your session.

Touch behaves like a touchscreen: tap to click, drag to drag, two fingers to scroll, and either a held finger or a quick two-finger tap for a right click — the button a tablet does not have. The stylus is not a mouse pretending: the PC creates a real graphics tablet, so pressure, tilt and the eraser end reach Krita, GIMP and Xournal++ the way they would from a Wacom. A small toolbar in the corner carries the keys a tablet is missing (Ctrl, Alt, Shift, Super, Esc, Tab) — the modifiers latch, so tapping Ctrl and then the screen is a Ctrl+click — plus a button that raises the on-screen keyboard, which types on the PC in the PC's own layout. A hardware keyboard attached to the tablet works too, and needs none of this. Latency is a function of your Wi-Fi; on a sane 5 GHz network expect it to feel like a slightly relaxed wired monitor. For the lowest latency, plug in a USB cable and enable **USB tethering** on the tablet (Settings → Network → Hotspot & tethering): that creates a private wired network between the two devices and Linux Link finds the PC over it automatically — no extra configuration.

## Where the project stands

The core is solid on my machines (Zorin PC, Honor phone): pairing, auto-reconnect on boot, notifications, clipboard, files, media, webcam. v3.0 is where reliability and idle cost got taken seriously — reconnection is now driven by network events rather than timers, and the daemon no longer polls anything while nothing is happening. That work is fresh, and long-run behaviour across suspend cycles and Wi-Fi changes is exactly what I'd like other people's machines to tell me about. If something breaks for you, open an issue — the logs from `journalctl --user -u linkd` are usually what I'll ask for.

<details>
<summary><b>Version history</b></summary>

- **v5.0.2** — a sharper picture and quieter source files. The second screen encodes at 12 Mbit/s for 1080p (up to 24 for larger panels) with the superfast preset — CABAC and real motion estimation make desktop text crisp at the same latency. The code sheds its running commentary and lets the names do the talking.
- **v5.0.1** — the second screen chases zero. Intra-refresh encoding flattens the periodic keyframe burst into a rolling column, a two-frame VBV caps every frame's transmission time, the outgoing queue shrinks from a quarter second of possible backlog to four frames, the idle flush drops from 40 ms to 10 ms, the video stream takes priority over file transfers on the shared connection, and the decoder is asked for realtime priority in every dialect Android knows.
- **v5.0** — monochrome, multi-PC, and defaults that know the device. The interface drops colour altogether: white and near-black in light, true black and soft white in dark, on both platforms — colour now only ever means something (a live link, an error). The phone finally treats owning several computers as normal: every paired PC is remembered, the home screen lists them, tapping one switches, and when the active PC is off the phone quietly connects to whichever known PC answers. The second screen is on by default on tablets and off by default on phones, with a per-device switch and a Quick Settings tile that respects it.
- **v4.2** — the accent stops negotiating. Material You is out: the Linux Link violet is the identity on every phone, in both modes, matching the PC windows. Pairing another PC returns to the home screen as its own row, and the second screen gets a Quick Settings tile — swipe down, tap, the device is a monitor.
- **v4.1** — the interface grows up. One design system on both sides: the Linux Link violet as the single accent, light and dark following the system on the PC windows, real switches instead of checkboxes, and a "Restart the service" button where a systemctl command used to be. On Android, a first-run screen with exactly one button, a connection card that tells the truth, a proper remote with icons, and settings rows that show their state instead of hiding it in button labels.
- **v4.0** — the second screen grows up. The PC can now offer it (tray menu or `linkd screen`) instead of waiting to be asked from the tablet; the stylus drives a real virtual graphics tablet, with pressure, tilt, barrel button and eraser, so drawing applications treat it as a digitizer rather than a mouse; a finger held still or a two-finger tap is a right click; an on-screen toolbar carries the latching modifiers, Esc, Tab and the soft keyboard; and the picture is letterboxed to the monitor the PC actually created, so the pointer lands exactly where you touched.
- **v3.2** — the tablet becomes a real second monitor (Sidecar-style extension): virtual display per compositor (Mutter API on GNOME, headless outputs on Hyprland/Sway, krfb + portal on KDE, xrandr on X11), H.264 low-latency streaming over the existing QUIC link, touch/pen/keyboard injection (Mutter remote API or uinput), automatic reconnection, USB via tethering.
- **v3.1** — the phone joins the PC's sound settings: "Phone (Linux Link)" as an output device (play anything on the PC, hear it on the phone) and the mic usable without opening the webcam screen, both from simple toggles that keep working with the phone's screen off. Also: virtual interfaces (VPN, Docker) are filtered out of the pairing QR, and `install.sh` detects an active ufw/firewalld and offers to open the ports.
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

QUIC with ALPN `linuxlink/1`, mutual TLS 1.3, self-signed certificates pinned by SHA-256 fingerprint at pairing (trust-on-first-use via the QR code). Messages are JSON, one per line, over a bidirectional stream. The BLE advertisement carries the service UUID (`4c4c0001-…`) and nothing else — it exists to wake the phone, not to carry data. The PC's address comes from a one-datagram UDP probe on port 47101 (answered in milliseconds, and it works on networks that block mDNS multicast), with mDNS and the last known address as fallbacks.

## License

GPL-3.0 — see [LICENSE](LICENSE).
