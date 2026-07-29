use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{Tray, TrayMethods};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Deserialize, Clone, Default, PartialEq)]
struct DeviceStatus {
    #[serde(default)]
    name: String,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    connected: bool,
}

#[derive(Deserialize, Clone, PartialEq)]
struct Status {
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    device_count: usize,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    devices: Vec<DeviceStatus>,
    #[serde(default = "neg_one")]
    battery: i32,
    #[serde(default)]
    charging: bool,
    #[serde(default)]
    proximity: bool,
    /// Not in the file: the daemon used to stamp `status.json` every two
    /// seconds so we could treat a stale timestamp as "service stopped". That
    /// heartbeat cost a disk write every two seconds forever, so it is gone —
    /// we now ask the daemon directly by knocking on its control socket.
    #[serde(skip)]
    alive: bool,
}

fn neg_one() -> i32 {
    -1
}

impl Default for Status {
    fn default() -> Self {
        Status {
            connected: false,
            device_count: 0,
            device_name: String::new(),
            devices: Vec::new(),
            battery: -1,
            charging: false,
            proximity: false,
            alive: false,
        }
    }
}

impl Status {
    fn daemon_alive(&self) -> bool {
        self.alive
    }

    fn online(&self) -> bool {
        self.daemon_alive() && self.connected
    }
}

fn status_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("linux-link")
        .join("status.json")
}

fn socket_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("linuxlink.sock")
}

/// The daemon holds this socket open for as long as it lives, and the kernel
/// refuses the connection the moment it dies — a truthful liveness check that
/// costs nothing while idle.
fn daemon_running() -> bool {
    std::os::unix::net::UnixStream::connect(socket_path()).is_ok()
}

fn read_status() -> Status {
    let mut status: Status = std::fs::read_to_string(status_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    status.alive = daemon_running();
    status
}

struct LinkTray {
    status: Status,
}

impl Tray for LinkTray {
    fn id(&self) -> String {
        "linux-link".into()
    }

    fn title(&self) -> String {
        "Linux Link".into()
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![make_icon(self.status.online())]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = if !self.status.daemon_alive() {
            "Service stopped".to_string()
        } else if self.status.online() {
            let name = if self.status.device_name.is_empty() {
                "Phone".to_string()
            } else {
                self.status.device_name.clone()
            };
            let mut s = format!("Connected: {name}");
            if self.status.device_count > 1 {
                s.push_str(&format!(" (+{})", self.status.device_count - 1));
            }
            if self.status.battery >= 0 {
                s.push_str(&format!(
                    "\nBattery: {}%{}",
                    self.status.battery,
                    if self.status.charging { " ⚡" } else { "" }
                ));
            }
            s
        } else {
            "Waiting for the phone…".to_string()
        };
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![],
            title: "Linux Link".into(),
            description,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = &self.status;
        let mut items: Vec<MenuItem<Self>> = Vec::new();

        if !s.daemon_alive() {
            items.push(
                StandardItem {
                    label: "⚠  Service stopped".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else if s.devices.is_empty() {
            items.push(
                StandardItem {
                    label: "○  No paired device".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            for d in &s.devices {
                let dot = if d.connected { "●" } else { "○" };
                items.push(
                    StandardItem {
                        label: format!("{dot}  {}", d.name),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }

        if s.online() && s.battery >= 0 {
            let icon = if s.charging { "⚡" } else { "🔋" };
            items.push(
                StandardItem {
                    label: format!("{icon}  Battery: {}%", s.battery),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(MenuItem::Separator);

        let connected: Vec<&DeviceStatus> = s.devices.iter().filter(|d| d.connected).collect();
        match connected.len() {
            0 => items.push(
                StandardItem {
                    label: "Send a file…".into(),
                    icon_name: "document-send".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ),
            1 => {
                let fp = connected[0].fingerprint.clone();
                items.push(
                    StandardItem {
                        label: "Send a file…".into(),
                        icon_name: "document-send".into(),
                        activate: Box::new(move |_: &mut Self| send_file_dialog(Some(fp.clone()))),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            _ => {
                let submenu: Vec<MenuItem<Self>> = connected
                    .iter()
                    .map(|d| {
                        let fp = d.fingerprint.clone();
                        StandardItem {
                            label: d.name.clone(),
                            icon_name: "smartphone".into(),
                            activate: Box::new(move |_: &mut Self| {
                                send_file_dialog(Some(fp.clone()))
                            }),
                            ..Default::default()
                        }
                        .into()
                    })
                    .collect();
                items.push(MenuItem::SubMenu(SubMenu {
                    label: "Send a file…".into(),
                    icon_name: "document-send".into(),
                    submenu,
                    ..Default::default()
                }));
            }
        }

        items.push(MenuItem::SubMenu(SubMenu {
            label: "Phone media".into(),
            icon_name: "multimedia-player".into(),
            enabled: s.online(),
            submenu: vec![
                StandardItem {
                    label: "Play / Pause".into(),
                    icon_name: "media-playback-start".into(),
                    activate: Box::new(|_: &mut Self| linkd(&["media", "play_pause"])),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Previous".into(),
                    icon_name: "media-skip-backward".into(),
                    activate: Box::new(|_: &mut Self| linkd(&["media", "previous"])),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Next".into(),
                    icon_name: "media-skip-forward".into(),
                    activate: Box::new(|_: &mut Self| linkd(&["media", "next"])),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Volume +".into(),
                    icon_name: "audio-volume-high".into(),
                    activate: Box::new(|_: &mut Self| linkd(&["phone-volume", "up"])),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Volume −".into(),
                    icon_name: "audio-volume-low".into(),
                    activate: Box::new(|_: &mut Self| linkd(&["phone-volume", "down"])),
                    ..Default::default()
                }
                .into(),
            ],
            ..Default::default()
        }));

        items.push(
            CheckmarkItem {
                label: "Proximity lock".into(),
                checked: s.proximity,
                enabled: s.daemon_alive(),
                activate: Box::new(|this: &mut Self| {
                    let on = !this.status.proximity;
                    this.status.proximity = on;
                    linkd(&["proximity-lock", if on { "on" } else { "off" }]);
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "Pair a device…".into(),
                icon_name: "list-add".into(),
                activate: Box::new(|_: &mut Self| pair_new_device()),
                ..Default::default()
            }
            .into(),
        );

        items.push(
            StandardItem {
                label: "Settings…".into(),
                icon_name: "preferences-system".into(),
                activate: Box::new(|_: &mut Self| open_settings()),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

fn logo_rgba() -> &'static Vec<u8> {
    static LOGO: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        image::load_from_memory(include_bytes!("../../assets/logo.png"))
            .expect("embedded logo")
            .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
            .into_raw()
    })
}

fn make_icon(connected: bool) -> ksni::Icon {
    const S: i32 = 32;
    let logo = logo_rgba();
    let mut data = vec![0u8; (S * S * 4) as usize];

    let corner = 8.0f32;
    for y in 0..S {
        for x in 0..S {
            let i = ((y * S + x) * 4) as usize;
            let (r, g, b, a) = (logo[i], logo[i + 1], logo[i + 2], logo[i + 3]);
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let cx = fx.clamp(corner, S as f32 - corner);
            let cy = fy.clamp(corner, S as f32 - corner);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            data[i] = if d <= corner { a } else { 0 };
            data[i + 1] = r;
            data[i + 2] = g;
            data[i + 3] = b;
        }
    }

    let (dr, dg, db) = if connected {
        (76u8, 201u8, 110u8)
    } else {
        (150u8, 150u8, 150u8)
    };
    let (dx0, dy0) = (25.0f32, 25.0f32);
    for y in 0..S {
        for x in 0..S {
            let d = ((x as f32 - dx0).powi(2) + (y as f32 - dy0).powi(2)).sqrt();
            let i = ((y * S + x) * 4) as usize;
            if d <= 4.5 {
                data[i] = 255;
                data[i + 1] = dr;
                data[i + 2] = dg;
                data[i + 3] = db;
            } else if d <= 6.0 {
                data[i] = 255;
                data[i + 1] = 0x12;
                data[i + 2] = 0x10;
                data[i + 3] = 0x18;
            }
        }
    }

    ksni::Icon {
        width: S,
        height: S,
        data,
    }
}

fn linkd_bin() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".local/bin/linkd");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("linkd")
}

fn linkd(args: &[&str]) {
    let bin = linkd_bin();
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let _ = Command::new(bin).args(&owned).status();
    });
}

fn notify(title: &str, body: &str) {
    let _ = Command::new("notify-send").arg(title).arg(body).spawn();
}

fn send_file_dialog(target_fp: Option<String>) {
    std::thread::spawn(move || {
        let out = Command::new("zenity")
            .args([
                "--file-selection",
                "--multiple",
                "--separator=\n",
                "--title=Send to phone",
            ])
            .output();
        let out = match out {
            Ok(o) => o,
            Err(_) => {
                notify("Linux Link", "zenity not found — install the zenity package.");
                return;
            }
        };
        if !out.status.success() {
            return;
        }
        let paths = String::from_utf8_lossy(&out.stdout);
        let bin = linkd_bin();
        let (mut ok, mut fail) = (0u32, 0u32);
        for path in paths.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let mut cmd = Command::new(&bin);
            cmd.arg("send-file").arg(path);
            if let Some(fp) = &target_fp {
                cmd.arg("--to").arg(fp);
            }
            let done = cmd.status().map(|s| s.success()).unwrap_or(false);
            if done {
                ok += 1;
            } else {
                fail += 1;
            }
        }
        if fail == 0 {
            notify("Linux Link", &format!("{ok} file(s) sent 📎"));
        } else {
            notify(
                "Linux Link",
                &format!("{ok} sent, {fail} failed. Is the device connected?"),
            );
        }
    });
}

/// Opens the settings window, next to our own binary first — the tray app is
/// normally launched by an absolute path from the autostart entry, so `$PATH`
/// may not contain `~/.local/bin` at all.
fn open_settings() {
    std::thread::spawn(|| {
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("linux-link-settings")));
        if let Some(path) = sibling {
            if path.exists() && Command::new(&path).spawn().is_ok() {
                return;
            }
        }
        let beside_linkd = linkd_bin().with_file_name("linux-link-settings");
        if beside_linkd.exists() && Command::new(&beside_linkd).spawn().is_ok() {
            return;
        }
        if Command::new("linux-link-settings").spawn().is_err() {
            notify("Linux Link", "Settings window not found — run install.sh again.");
        }
    });
}

fn pair_new_device() {
    std::thread::spawn(|| {
        let bin = linkd_bin();
        let pair_gui = bin.with_file_name("linux-link-pair");
        if Command::new(&pair_gui).spawn().is_ok() {
            return;
        }
        if Command::new("linux-link-pair").spawn().is_ok() {
            return;
        }
        let script = format!(
            "'{}' pair-live; echo; read -p 'Press Enter to close…' _",
            bin.display()
        );
        let terms: &[(&str, &[&str])] = &[
            ("gnome-terminal", &["--", "bash", "-lc"]),
            ("x-terminal-emulator", &["-e", "bash", "-lc"]),
            ("konsole", &["-e", "bash", "-lc"]),
            ("xfce4-terminal", &["-x", "bash", "-lc"]),
            ("xterm", &["-e", "bash", "-lc"]),
        ];
        for (term, pre) in terms {
            let mut c = Command::new(term);
            for a in pre.iter() {
                c.arg(a);
            }
            c.arg(&script);
            if c.spawn().is_ok() {
                return;
            }
        }
        notify(
            "Linux Link",
            "No terminal found. Run `linkd pair` in a terminal.",
        );
    });
}

#[tokio::main]
async fn main() {
    // At session login we are often started BEFORE the desktop's tray support
    // is ready (on GNOME the AppIndicator extension loads a few seconds after
    // autostart apps). So instead of giving up, keep retrying quietly: the
    // icon then appears by itself as soon as the tray becomes available.
    let mut attempts = 0u32;
    'session: loop {
        let handle = loop {
            match (LinkTray {
                status: read_status(),
            })
            .spawn()
            .await
            {
                Ok(h) => break h,
                Err(e) => {
                    attempts += 1;
                    if attempts == 1 {
                        eprintln!("Tray not ready yet ({e}) — retrying…");
                    }
                    // ~10 minutes of patience, then a clear message.
                    if attempts >= 200 {
                        eprintln!(
                            "Cannot display the system tray icon: {e}\n\
                             Your desktop must support StatusNotifierItem/AppIndicator.\n\
                             On GNOME, install the \"AppIndicator support\" extension."
                        );
                        std::process::exit(1);
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
        };
        attempts = 0;

        let mut shown = read_status();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;

            // The tray host (the shell or the bar) may have restarted while we
            // were asleep. Checking the handle is free; it used to be inferred
            // from a failed update, which meant issuing one every two seconds.
            if handle.is_closed() {
                eprintln!("Tray connection lost — reconnecting…");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue 'session;
            }

            // Reading a 300-byte file is nothing; redrawing the icon is a D-Bus
            // round trip plus a repaint in the shell. Only do it when the state
            // has actually moved.
            let next = read_status();
            if next == shown {
                continue;
            }
            shown = next.clone();
            if handle.update(|t: &mut LinkTray| t.status = next).await.is_none() {
                eprintln!("Tray connection lost — reconnecting…");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue 'session;
            }
        }
    }
}
