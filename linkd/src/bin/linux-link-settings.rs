//! The Linux Link settings window.
//!
//! Everything a user might want to change without opening a terminal: which
//! phones are trusted, whether the PC locks when the phone walks away, whether
//! the tray icon comes back at login, and the global keyboard shortcuts.
//!
//! This is a separate binary from the daemon, so it owns none of the daemon's
//! state. It reads the same files the daemon writes and asks the daemon for
//! anything that has to happen live, through the control socket. When the
//! daemon is down the window still works — it edits the files directly — which
//! matters because "the service will not start" is exactly when someone opens
//! the settings.

use eframe::egui::{self, Color32, Rounding, Vec2};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const BG: Color32 = Color32::from_rgb(0x12, 0x10, 0x18);
const CARD: Color32 = Color32::from_rgb(0x1B, 0x18, 0x24);
const ACCENT: Color32 = Color32::from_rgb(0x9E, 0x7B, 0xFF);
const OK_GREEN: Color32 = Color32::from_rgb(0x4C, 0xC9, 0x6E);
const WARN_RED: Color32 = Color32::from_rgb(0xE5, 0x6B, 0x6B);
const TEXT_DIM: Color32 = Color32::from_rgb(0xA8, 0xA2, 0xB8);

/// How often the window re-reads the world. Nothing here is expensive, but
/// there is no reason to stat four files sixty times a second.
const REFRESH: Duration = Duration::from_millis(1200);

// ---------------------------------------------------------------- the files

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

/// The daemon holds the control socket open for as long as it lives.
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

// -------------------------------------------------------------- the actions

/// Fire-and-forget line on the control socket.
fn tell_daemon(line: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket_path())?;
    s.write_all(format!("{line}\n").as_bytes())?;
    s.flush()
}

/// A line on the control socket, with the daemon's answer.
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

/// Forgets a device through the daemon when it is up — it can then refuse the
/// live connection straight away — and by editing `peers.json` when it is not.
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

/// The proximity lock lives in the daemon's memory as well as on disk, so it
/// has to go through the socket to take effect without a restart.
fn set_proximity(on: bool) -> Result<(), String> {
    tell_daemon(&format!("LOCKMODE {}", if on { "on" } else { "off" }))
        .map_err(|_| "the service is not running".to_string())
}

fn autostart_enabled() -> bool {
    match std::fs::read_to_string(autostart_file()) {
        // A desktop entry is disabled by `Hidden=true`, not by deleting it —
        // that way we never lose the Exec line the installer wrote.
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

/// The shortcuts belong to the daemon binary, which knows how to talk to each
/// desktop. Shelling out to it keeps that knowledge in one place.
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

// ------------------------------------------------------------------ the app

/// Everything the window shows, refreshed together on a timer.
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
    /// The last thing that happened, shown at the bottom.
    toast: Option<(String, bool)>,
    /// Fingerprint awaiting confirmation before it is forgotten.
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

    fn card<R>(&self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) {
        egui::Frame::none()
            .fill(CARD)
            .rounding(Rounding::same(14.0))
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui);
            });
        ui.add_space(12.0);
    }

    fn heading(ui: &mut egui::Ui, text: &str) {
        ui.label(egui::RichText::new(text).size(13.0).strong().color(TEXT_DIM));
        ui.add_space(8.0);
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_read.elapsed() >= REFRESH {
            self.refresh();
        }
        ctx.request_repaint_after(REFRESH);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(20.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.header(ui);
                    self.devices_card(ui);
                    self.behaviour_card(ui);
                    self.shortcuts_card(ui);
                    self.footer(ui);
                });
            });
    }
}

impl SettingsApp {
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(
                    egui::Image::new(logo)
                        .fit_to_exact_size(Vec2::splat(44.0))
                        .rounding(Rounding::same(11.0)),
                );
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Linux Link").size(21.0).strong().color(Color32::WHITE),
                );
                let (text, colour) = if !self.world.alive {
                    ("Service stopped".to_string(), WARN_RED)
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
                    (format!("{} connected{bat}", self.world.status.device_count), OK_GREEN)
                } else {
                    ("Waiting for a phone".to_string(), TEXT_DIM)
                };
                ui.label(egui::RichText::new(text).size(13.0).color(colour));
            });
        });
        ui.add_space(16.0);

        if !self.world.alive {
            self.card(ui, |ui| {
                ui.label(
                    egui::RichText::new("The linkd service is not answering.")
                        .size(13.0)
                        .color(Color32::WHITE),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("systemctl --user restart linkd")
                        .size(12.0)
                        .monospace()
                        .color(TEXT_DIM),
                );
            });
        }
    }

    fn devices_card(&mut self, ui: &mut egui::Ui) {
        // Collected before the closure so the borrow checker is happy about
        // `self` being used inside it.
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

        egui::Frame::none()
            .fill(CARD)
            .rounding(Rounding::same(14.0))
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                Self::heading(ui, "PAIRED DEVICES");
                if peers.is_empty() {
                    ui.label(
                        egui::RichText::new("No device yet. Pair your phone to get started.")
                            .size(13.0)
                            .color(TEXT_DIM),
                    );
                }
                for p in &peers {
                    let short = &p.fingerprint[..p.fingerprint.len().min(16)];
                    // The status file carries fingerprints when it can and only
                    // names when it cannot; match on either.
                    let online = connected.iter().any(|f| f.starts_with(short))
                        || connected_names.iter().any(|n| n == &p.name);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(if online { "●" } else { "○" })
                                .size(13.0)
                                .color(if online { OK_GREEN } else { TEXT_DIM }),
                        );
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&p.name).size(14.0).color(Color32::WHITE),
                            );
                            ui.label(
                                egui::RichText::new(short).size(11.0).monospace().color(TEXT_DIM),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if confirming.as_deref() == Some(p.fingerprint.as_str()) {
                                if ui
                                    .button(egui::RichText::new("Confirm").color(WARN_RED))
                                    .clicked()
                                {
                                    to_forget = Some(p.fingerprint.clone());
                                }
                                if ui.button("Cancel").clicked() {
                                    cancel = true;
                                }
                            } else if ui.button("Forget").clicked() {
                                ask_forget = Some(p.fingerprint.clone());
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(4.0);
                if ui
                    .button(egui::RichText::new("  Pair a device…  ").size(14.0))
                    .clicked()
                {
                    pair = true;
                }
            });
        ui.add_space(12.0);

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

    fn behaviour_card(&mut self, ui: &mut egui::Ui) {
        let mut proximity = self.world.status.proximity;
        let mut autostart = self.world.autostart;
        let mut prox_changed = false;
        let mut auto_changed = false;

        egui::Frame::none()
            .fill(CARD)
            .rounding(Rounding::same(14.0))
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                Self::heading(ui, "BEHAVIOUR");
                if ui
                    .checkbox(&mut proximity, "Lock the session when the phone leaves")
                    .changed()
                {
                    prox_changed = true;
                }
                ui.label(
                    egui::RichText::new(
                        "Unlocks on its own when it comes back. A few seconds' grace, \
                         so a Wi-Fi hiccup does not lock you out.",
                    )
                    .size(11.5)
                    .color(TEXT_DIM),
                );
                ui.add_space(10.0);
                if ui.checkbox(&mut autostart, "Start with the session").changed() {
                    auto_changed = true;
                }
                ui.label(
                    egui::RichText::new(
                        "The tray icon comes back at login. The daemon itself is a systemd \
                         user service and starts regardless.",
                    )
                    .size(11.5)
                    .color(TEXT_DIM),
                );
            });
        ui.add_space(12.0);

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

    fn shortcuts_card(&mut self, ui: &mut egui::Ui) {
        let installed = self.world.shortcuts;
        let mut toggle = false;

        egui::Frame::none()
            .fill(CARD)
            .rounding(Rounding::same(14.0))
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                Self::heading(ui, "KEYBOARD SHORTCUTS");
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
                                .color(if installed { ACCENT } else { TEXT_DIM }),
                        );
                        ui.label(egui::RichText::new(what).size(12.0).color(TEXT_DIM));
                    });
                }
                ui.add_space(10.0);
                let label = if installed { "  Remove them  " } else { "  Add them  " };
                if ui.button(egui::RichText::new(label).size(14.0)).clicked() {
                    toggle = true;
                }
            });
        ui.add_space(12.0);

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

    fn footer(&mut self, ui: &mut egui::Ui) {
        if let Some((msg, ok)) = self.toast.clone() {
            ui.label(
                egui::RichText::new(msg)
                    .size(12.0)
                    .color(if ok { OK_GREEN } else { WARN_RED }),
            );
            ui.add_space(6.0);
        }
        ui.label(
            egui::RichText::new(format!(
                "{} · Linux Link {}",
                self.pc_name,
                env!("CARGO_PKG_VERSION")
            ))
            .size(11.0)
            .color(TEXT_DIM),
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
