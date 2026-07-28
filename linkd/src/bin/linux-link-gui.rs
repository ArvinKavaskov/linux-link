use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{Tray, TrayMethods};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Deserialize, Clone, Default)]
struct DeviceStatus {
    #[serde(default)]
    name: String,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    connected: bool,
}

#[derive(Deserialize, Clone)]
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
    #[serde(default)]
    updated: u64,
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
            updated: 0,
        }
    }
}

impl Status {
    fn daemon_alive(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.updated != 0 && now.saturating_sub(self.updated) < 8
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

fn read_status() -> Status {
    std::fs::read_to_string(status_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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

fn make_icon(connected: bool) -> ksni::Icon {
    const W: i32 = 24;
    const H: i32 = 24;
    let mut data = vec![0u8; (W * H * 4) as usize];

    fn px(data: &mut [u8], x: i32, y: i32, a: u8, r: u8, g: u8, b: u8) {
        if x < 0 || y < 0 || x >= 24 || y >= 24 {
            return;
        }
        let i = ((y * 24 + x) * 4) as usize;
        data[i] = a;
        data[i + 1] = r;
        data[i + 2] = g;
        data[i + 3] = b;
    }

    let (pr, pg, pb) = (124u8, 77u8, 255u8);
    for y in 2..=21 {
        for x in 6..=15 {
            let corner = (x == 6 || x == 15) && (y == 2 || y == 21);
            if corner {
                continue;
            }
            px(&mut data, x, y, 255, pr, pg, pb);
        }
    }
    for y in 4..=18 {
        for x in 7..=14 {
            px(&mut data, x, y, 255, 226, 220, 255);
        }
    }
    let (dr, dg, db) = if connected {
        (52u8, 168u8, 83u8)
    } else {
        (150u8, 150u8, 150u8)
    };
    let (cx, cy, rad) = (17i32, 18i32, 4i32);
    for y in (cy - rad)..=(cy + rad) {
        for x in (cx - rad)..=(cx + rad) {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= rad * rad {
                px(&mut data, x, y, 255, dr, dg, db);
            }
        }
    }

    ksni::Icon {
        width: W,
        height: H,
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

fn pair_new_device() {
    std::thread::spawn(|| {
        let bin = linkd_bin();
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
    let handle = match (LinkTray {
        status: read_status(),
    })
    .spawn()
    .await
    {
        Ok(h) => h,
        Err(e) => {
            eprintln!(
                "Cannot display the system tray icon: {e}\n\
                 Your desktop must support StatusNotifierItem/AppIndicator.\n\
                 On GNOME, install the \"AppIndicator support\" extension."
            );
            std::process::exit(1);
        }
    };

    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let next = read_status();
        if handle.update(|t: &mut LinkTray| t.status = next).await.is_none() {
            break;
        }
    }
}
