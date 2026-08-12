
use eframe::egui::{self, Vec2};
use serde::{Deserialize, Serialize};

#[path = "../ui_theme.rs"]
mod theme;
use theme::Palette;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_millis(1200);

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("linux-link")
}

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("linuxlink.sock")
}

fn autostart_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("autostart")
        .join("linux-link.desktop")
}

fn daemon_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

#[derive(Deserialize, Default, Clone)]
struct DeviceStatus {
    #[serde(default)]
    name: String,
    #[serde(default)]
    fingerprint: String,
}

#[derive(Deserialize, Default, Clone)]
struct Status {
    #[serde(default)]
    device_count: usize,
    #[serde(default)]
    devices: Vec<DeviceStatus>,
    #[serde(default)]
    battery: i32,
    #[serde(default)]
    charging: bool,
    #[serde(default)]
    proximity: bool,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Peer {
    name: String,
    fingerprint: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct TrustedPeers {
    peers: Vec<Peer>,
}

fn read_status() -> Status {
    std::fs::read_to_string(config_dir().join("status.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn read_peers() -> TrustedPeers {
    std::fs::read_to_string(config_dir().join("peers.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn this_pc_name() -> String {
    std::fs::read_to_string(config_dir().join("device_name"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "this PC".to_string())
}

fn tell_daemon(line: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket_path())?;
    s.write_all(format!("{line}\n").as_bytes())?;
    s.flush()
}

fn ask_daemon(line: &str) -> std::io::Result<String> {
    let stream = UnixStream::connect(socket_path())?;
    let mut wr = stream.try_clone()?;
    wr.write_all(format!("{line}\n").as_bytes())?;
    wr.flush()?;
    BufReader::new(stream)
        .lines()
        .next()
        .unwrap_or_else(|| Ok(String::new()))
}

fn forget_device(fingerprint: &str) -> Result<String, String> {
    if let Ok(reply) = ask_daemon(&format!("FORGET {fingerprint}")) {
        return match reply.strip_prefix("OK ") {
            Some(name) => Ok(name.trim().to_string()),
            None => Err(reply.trim_start_matches("ERR ").trim().to_string()),
        };
    }
    let mut peers = read_peers();
    let Some(pos) = peers.peers.iter().position(|p| p.fingerprint.starts_with(fingerprint)) else {
        return Err("unknown device".into());
    };
    let removed = peers.peers.remove(pos);
    let json = serde_json::to_string_pretty(&peers).map_err(|e| e.to_string())?;
    std::fs::write(config_dir().join("peers.json"), json).map_err(|e| e.to_string())?;
    Ok(removed.name)
}

fn restart_service() -> Result<(), String> {
    let out = std::process::Command::new("systemctl")
        .args(["--user", "restart", "linkd"])
        .output()
        .map_err(|e| format!("cannot run systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn set_proximity(on: bool) -> Result<(), String> {
    tell_daemon(&format!("LOCKMODE {}", if on { "on" } else { "off" }))
        .map_err(|_| "the service is not running".to_string())
}

fn autostart_enabled() -> bool {
    match std::fs::read_to_string(autostart_file()) {
        Ok(text) => !text.lines().any(|l| l.trim().eq_ignore_ascii_case("hidden=true")),
        Err(_) => false,
    }
}

fn set_autostart(on: bool) -> Result<(), String> {
    let path = autostart_file();
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "autostart entry missing — run install.sh again".to_string())?;
    let mut kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            let l = l.trim().to_ascii_lowercase();
            !l.starts_with("hidden=") && !l.starts_with("x-gnome-autostart-enabled=")
        })
        .collect();
    let flags = if on {
        "X-GNOME-Autostart-enabled=true"
    } else {
        "Hidden=true\nX-GNOME-Autostart-enabled=false"
    };
    kept.push(flags);
    std::fs::write(&path, format!("{}\n", kept.join("\n"))).map_err(|e| e.to_string())
}

fn linkd_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("linkd")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("linkd"))
}

fn shortcuts(action: &str) -> Result<String, String> {
    let out = std::process::Command::new(linkd_bin())
        .args(["shortcuts", action])
        .output()
        .map_err(|e| format!("cannot run linkd: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn shortcuts_installed() -> bool {
    shortcuts("status").map(|s| s.contains(": installed")).unwrap_or(false)
}

fn launch(bin: &str) {
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(bin)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(bin));
    let _ = std::process::Command::new(path).spawn();
}

struct World {
    alive: bool,
    status: Status,
    peers: TrustedPeers,
    autostart: bool,
    shortcuts: bool,
}

impl World {
    fn read() -> Self {
        Self {
            alive: daemon_running(),
            status: read_status(),
            peers: read_peers(),
            autostart: autostart_enabled(),
            shortcuts: shortcuts_installed(),
        }
    }
}

struct SettingsApp {
    world: World,
    last_read: Instant,
    logo: Option<egui::TextureHandle>,
    pc_name: String,
    toast: Option<(String, bool)>,
    confirm_forget: Option<String>,
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let logo = image::load_from_memory(include_bytes!("../../assets/logo.png"))
            .ok()
            .map(|img| {
                let img = img.to_rgba8();
                let (w, h) = img.dimensions();
                let ci = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    img.as_raw(),
                );
                cc.egui_ctx.load_texture("logo", ci, Default::default())
            });
        Self {
            world: World::read(),
            last_read: Instant::now(),
            logo,
            pc_name: this_pc_name(),
            toast: None,
            confirm_forget: None,
        }
    }

    fn refresh(&mut self) {
        self.world = World::read();
        self.last_read = Instant::now();
    }

    fn say(&mut self, msg: impl Into<String>, ok: bool) {
        self.toast = Some((msg.into(), ok));
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_read.elapsed() >= REFRESH {
            self.refresh();
        }
        ctx.request_repaint_after(REFRESH);

        let p = theme::frame_palette(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.bg).inner_margin(20.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.header(ui, &p);
                    self.devices_card(ui, &p);
                    self.behaviour_card(ui, &p);
                    self.shortcuts_card(ui, &p);
                    self.footer(ui, &p);
                });
            });
    }
}

impl SettingsApp {
    fn header(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(
                    egui::Image::new(logo)
                        .fit_to_exact_size(Vec2::splat(44.0))
                        .rounding(egui::Rounding::same(12.0)),
                );
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Linux Link").size(21.0).strong().color(p.text));
                ui.horizontal(|ui| {
                    let (text, colour, on) = if !self.world.alive {
                        ("Service stopped".to_string(), p.warn, false)
                    } else if self.world.status.device_count > 0 {
                        let bat = if self.world.status.battery >= 0 {
                            format!(
                                " · {}%{}",
                                self.world.status.battery,
                                if self.world.status.charging { " ⚡" } else { "" }
                            )
                        } else {
                            String::new()
                        };
                        (format!("Connected{bat}"), p.ok, true)
                    } else {
                        ("Waiting for a phone".to_string(), p.dim, false)
                    };
                    theme::status_dot(ui, colour, on);
                    ui.label(egui::RichText::new(text).size(13.0).color(colour));
                });
            });
        });
        ui.add_space(16.0);

        if !self.world.alive {
            let mut restart = false;
            theme::card(ui, p, |ui| {
                ui.label(
                    egui::RichText::new("The Linux Link service is not running.")
                        .size(14.0)
                        .color(p.text),
                );
                theme::hint(ui, p, "Pairing, sync and the second screen need it.");
                ui.add_space(10.0);
                if theme::primary_button(ui, p, "Restart the service").clicked() {
                    restart = true;
                }
            });
            if restart {
                match restart_service() {
                    Ok(()) => {
                        self.say("Service restarting…", true);
                        self.refresh();
                    }
                    Err(e) => self.say(e, false),
                }
            }
        }
    }

    fn devices_card(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let peers = self.world.peers.peers.clone();
        let connected: Vec<String> = self
            .world
            .status
            .devices
            .iter()
            .map(|d| d.fingerprint.clone())
            .collect();
        let connected_names: Vec<String> =
            self.world.status.devices.iter().map(|d| d.name.clone()).collect();
        let confirming = self.confirm_forget.clone();

        let mut to_forget: Option<String> = None;
        let mut ask_forget: Option<String> = None;
        let mut cancel = false;
        let mut pair = false;

        theme::card(ui, p, |ui| {
            theme::section_title(ui, p, "DEVICES");
            if peers.is_empty() {
                ui.label(
                    egui::RichText::new("No device yet — pairing takes about a minute.")
                        .size(13.0)
                        .color(p.dim),
                );
                ui.add_space(4.0);
            }
            for (i, peer) in peers.iter().enumerate() {
                let short = &peer.fingerprint[..peer.fingerprint.len().min(16)];
                let online = connected.iter().any(|f| f.starts_with(short))
                    || connected_names.iter().any(|n| n == &peer.name);
                ui.horizontal(|ui| {
                    theme::status_dot(ui, if online { p.ok } else { p.dim }, online);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&peer.name).size(14.0).color(p.text));
                        ui.label(
                            egui::RichText::new(if online { "Connected" } else { "Not connected" })
                                .size(11.5)
                                .color(p.dim),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if confirming.as_deref() == Some(peer.fingerprint.as_str()) {
                            if theme::danger_button(ui, p, "Remove").clicked() {
                                to_forget = Some(peer.fingerprint.clone());
                            }
                            if theme::quiet_button(ui, p, "Keep").clicked() {
                                cancel = true;
                            }
                        } else if theme::quiet_button(ui, p, "Forget…").clicked() {
                            ask_forget = Some(peer.fingerprint.clone());
                        }
                    });
                });
                if i + 1 < peers.len() {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }
            }
            ui.add_space(10.0);
            if theme::primary_button(ui, p, "Pair a device").clicked() {
                pair = true;
            }
        });

        if cancel {
            self.confirm_forget = None;
        }
        if let Some(fp) = ask_forget {
            self.confirm_forget = Some(fp);
        }
        if let Some(fp) = to_forget {
            self.confirm_forget = None;
            match forget_device(&fp) {
                Ok(name) => {
                    self.say(format!("{name} forgotten."), true);
                    self.refresh();
                }
                Err(e) => self.say(e, false),
            }
        }
        if pair {
            launch("linux-link-pair");
        }
    }

    fn behaviour_card(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let mut proximity = self.world.status.proximity;
        let mut autostart = self.world.autostart;
        let mut prox_changed = false;
        let mut auto_changed = false;

        theme::card(ui, p, |ui| {
            theme::section_title(ui, p, "BEHAVIOUR");
            if theme::switch_row(
                ui,
                p,
                "Lock when the phone leaves",
                Some("Unlocks on its own when it comes back, with a few seconds' grace."),
                &mut proximity,
            ) {
                prox_changed = true;
            }
            ui.add_space(10.0);
            if theme::switch_row(
                ui,
                p,
                "Start with the session",
                Some("Brings the tray icon back at login."),
                &mut autostart,
            ) {
                auto_changed = true;
            }
        });

        if prox_changed {
            match set_proximity(proximity) {
                Ok(()) => {
                    self.world.status.proximity = proximity;
                    self.say(
                        if proximity {
                            "Proximity lock enabled."
                        } else {
                            "Proximity lock disabled."
                        },
                        true,
                    );
                }
                Err(e) => self.say(e, false),
            }
        }
        if auto_changed {
            match set_autostart(autostart) {
                Ok(()) => {
                    self.world.autostart = autostart;
                    self.say(if autostart { "Autostart on." } else { "Autostart off." }, true);
                }
                Err(e) => self.say(e, false),
            }
        }
    }

    fn shortcuts_card(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let installed = self.world.shortcuts;
        let mut enabled = installed;
        let mut toggle = false;

        theme::card(ui, p, |ui| {
            theme::section_title(ui, p, "KEYBOARD SHORTCUTS");
            for (keys, what) in [
                ("Super + Shift + V", "Send the clipboard to the phone"),
                ("Super + Shift + B", "Send a file to the phone"),
                ("Super + Shift + Space", "Play / pause on the phone"),
            ] {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(keys)
                            .size(12.0)
                            .monospace()
                            .color(if installed { p.accent } else { p.dim }),
                    );
                    ui.label(egui::RichText::new(what).size(12.0).color(p.dim));
                });
            }
            ui.add_space(10.0);
            if theme::switch_row(ui, p, "Enable the shortcuts", None, &mut enabled) {
                toggle = true;
            }
        });

        if toggle {
            let action = if installed { "remove" } else { "install" };
            match shortcuts(action) {
                Ok(_) => {
                    self.say(
                        if installed { "Shortcuts removed." } else { "Shortcuts added." },
                        true,
                    );
                    self.refresh();
                }
                Err(e) => self.say(
                    if e.is_empty() { "Could not change the shortcuts.".into() } else { e },
                    false,
                ),
            }
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if let Some((msg, ok)) = self.toast.clone() {
            let colour = if ok { p.ok } else { p.warn };
            ui.label(egui::RichText::new(msg).size(12.0).color(colour));
            ui.add_space(6.0);
        }
        ui.label(
            egui::RichText::new(format!(
                "{} · Linux Link {}",
                self.pc_name,
                env!("CARGO_PKG_VERSION")
            ))
            .size(11.0)
            .color(p.dim),
        );
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 720.0])
            .with_min_inner_size([420.0, 520.0])
            .with_app_id("linux-link")
            .with_title("Linux Link — Settings"),
        ..Default::default()
    };
    eframe::run_native(
        "Linux Link — Settings",
        options,
        Box::new(|cc| Ok(Box::new(SettingsApp::new(cc)))),
    )
}
