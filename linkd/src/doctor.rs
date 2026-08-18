use std::process::Command;

pub fn run() {
    println!("Linux Link doctor — {}", env!("CARGO_PKG_VERSION"));
    println!();

    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    line(true, &format!(
        "Desktop: {desktop} ({})",
        if wayland { "Wayland" } else { "X11" }
    ));

    match ping_daemon() {
        Ok(()) => line(true, "Service: linkd is running and answering"),
        Err(e) => {
            line(false, &format!("Service: {e}"));
            hint("systemctl --user restart linkd  (or the Restart button in Settings)");
        }
    }

    let bins = ["linux-link-gui", "linux-link-pair", "linux-link-settings", "linux-link-message"];
    let missing: Vec<&str> = bins.iter().copied().filter(|b| !has_cmd(b)).collect();
    if missing.is_empty() {
        line(true, "Binaries: all five installed");
    } else {
        line(false, &format!("Binaries missing from PATH: {}", missing.join(", ")));
        hint("re-run ./install.sh");
    }

    match bluetooth_state() {
        Some((powered, advertising)) => {
            line(powered, &format!(
                "Bluetooth: controller {}",
                if powered { "powered" } else { "present but off" }
            ));
            if powered {
                line(advertising, &format!(
                    "BLE wake beacon: {}",
                    if advertising {
                        "advertising (the phone can wake on this PC)"
                    } else {
                        "not advertising yet — linkd retries with backoff; give it a minute"
                    }
                ));
            } else {
                hint("bluetoothctl power on");
            }
        }
        None => {
            line(false, "Bluetooth: no controller found — wake-on-approach unavailable");
            hint("mDNS/UDP discovery still works; a BLE dongle restores auto-wake");
        }
    }

    let session_type = if wayland { "wayland" } else { "x11" };
    let d = desktop.to_lowercase();
    let mut deps: Vec<(&str, &str)> = vec![("gst-launch-1.0", "gstreamer tools")];
    if d.contains("gnome") && !wayland {
        line(false, "Second screen: GNOME on X11 cannot host a virtual monitor");
        hint("log into the \"GNOME\" (Wayland) session instead");
    }
    if d.contains("kde") {
        deps.push(("krfb-virtualmonitor", "krfb (KDE virtual monitor)"));
    }
    if d.contains("hyprland") || d.contains("sway") {
        deps.push(("wf-recorder", "wf-recorder"));
    }
    if !wayland {
        deps.push(("ffmpeg", "ffmpeg (X11 capture)"));
    }
    for (bin, label) in deps {
        if has_cmd(bin) {
            line(true, &format!("Second screen dependency: {label}"));
        } else {
            line(false, &format!("Second screen dependency missing: {label} ({bin})"));
            hint("re-run ./install.sh to install the packages for this desktop");
        }
    }
    let _ = session_type;

    let player = ["pw-cat", "paplay", "aplay"].iter().find(|c| has_cmd(c));
    match player {
        Some(p) => line(true, &format!("Phone audio player: {p}")),
        None => line(false, "Phone audio: no player found (pw-cat, paplay or aplay)"),
    }

    if wayland && d.contains("gnome") {
        line(true, "Input injection: native (Mutter remote desktop, no udev rule needed)");
    } else {
        let uinput_ok = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .is_ok();
        line(uinput_ok, &format!(
            "Input injection: /dev/uinput {}",
            if uinput_ok { "writable" } else { "not writable" }
        ));
        if !uinput_ok {
            hint("re-run ./install.sh (udev rule), then log out and back in");
        }
    }

    if has_cmd("ufw") {
        let out = Command::new("ufw").arg("status").output();
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
            if text.contains("status: active") && !text.contains("47100") {
                line(false, "Firewall: ufw is active and 47100/47101 are not listed");
                hint("sudo ufw allow 47100/udp && sudo ufw allow 47100/tcp && sudo ufw allow 47101/udp");
            } else {
                line(true, "Firewall: ufw not blocking Linux Link");
            }
        }
    }

    println!();
    println!("If a line above is marked ✗ and the hint does not fix it, this output is");
    println!("exactly what to share when asking for help.");
}

fn line(ok: bool, text: &str) {
    println!("  {} {text}", if ok { "✓" } else { "✗" });
}

fn hint(text: &str) {
    println!("      ↳ {text}");
}

fn has_cmd(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ping_daemon() -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    let path = crate::control::socket_path();
    let mut stream = std::os::unix::net::UnixStream::connect(&path)
        .map_err(|_| "linkd is not running (control socket absent)".to_string())?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .ok();
    stream.write_all(b"PING\n").map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    if reply.trim() == "PONG" {
        Ok(())
    } else {
        Err(format!("unexpected reply: {}", reply.trim()))
    }
}

fn bluetooth_state() -> Option<(bool, bool)> {
    let out = Command::new("bluetoothctl").arg("show").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if !text.contains("Controller") {
        return None;
    }
    let powered = text.contains("Powered: yes");
    let advertising = text.contains("ActiveInstances: 0x01")
        || text.to_lowercase().contains("advertising: yes")
        || text.contains("ActiveInstances: 0x02");
    Some((powered, advertising))
}
