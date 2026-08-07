use eframe::egui::{self, Color32, Pos2, Rect, Rounding, Sense, Vec2};

#[path = "../ui_theme.rs"]
#[allow(dead_code)]
mod theme;
use qrcode::{EcLevel, QrCode};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const PAIR_WINDOW_SECS: u64 = 120;

/// The QR card is white with near-black modules in both themes: that is not
/// styling, it is what a phone camera locks onto fastest.
const QR_CARD: Color32 = Color32::WHITE;
const QR_MODULE: Color32 = Color32::from_rgb(0x14, 0x12, 0x1A);

#[derive(Clone)]
enum PairState {
    Connecting,
    Waiting { payload: String, since: Instant },
    Paired { name: String, at: Instant },
    Timeout,
    Error(String),
}

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("linuxlink.sock")
}

fn start_pairing(state: Arc<Mutex<PairState>>) {
    *state.lock().unwrap() = PairState::Connecting;
    std::thread::spawn(move || {
        let set = |s: PairState| *state.lock().unwrap() = s;
        let stream = match UnixStream::connect(socket_path()) {
            Ok(s) => s,
            Err(e) => {
                set(PairState::Error(format!(
                    "Daemon unreachable ({e}).\nIs the linkd service running?"
                )));
                return;
            }
        };
        let mut wr = match stream.try_clone() {
            Ok(w) => w,
            Err(e) => {
                set(PairState::Error(e.to_string()));
                return;
            }
        };
        if wr.write_all(b"PAIR\n").and_then(|_| wr.flush()).is_err() {
            set(PairState::Error("Cannot talk to the daemon.".into()));
            return;
        }
        let mut lines = BufReader::new(stream).lines();
        match lines.next() {
            Some(Ok(payload)) if !payload.starts_with("ERR ") => {
                set(PairState::Waiting { payload, since: Instant::now() });
            }
            Some(Ok(err)) => {
                set(PairState::Error(err.trim_start_matches("ERR ").to_string()));
                return;
            }
            _ => {
                set(PairState::Error("No response from the daemon.".into()));
                return;
            }
        }
        match lines.next() {
            Some(Ok(l)) if l.starts_with("PAIRED ") => {
                let name = l.trim_start_matches("PAIRED ").trim().to_string();
                set(PairState::Paired { name, at: Instant::now() });
            }
            _ => set(PairState::Timeout),
        }
    });
}

struct QrRender {
    width: usize,
    dark: Vec<bool>,
}

fn build_qr(payload: &str) -> Option<QrRender> {
    // Level M keeps the module count low -> bigger squares, easier to scan.
    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M).ok()?;
    let width = code.width();
    let dark = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    Some(QrRender { width, dark })
}

struct PairApp {
    state: Arc<Mutex<PairState>>,
    qr: Option<(String, QrRender)>,
    logo: Option<egui::TextureHandle>,
}

impl PairApp {
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
        let state = Arc::new(Mutex::new(PairState::Connecting));
        start_pairing(state.clone());
        Self { state, qr: None, logo }
    }

    fn draw_qr(&mut self, ui: &mut egui::Ui, payload: &str) {
        if self.qr.as_ref().map(|(p, _)| p.as_str()) != Some(payload) {
            if let Some(r) = build_qr(payload) {
                self.qr = Some((payload.to_string(), r));
            }
        }
        let Some((_, qr)) = &self.qr else { return };

        let card_size = ui.available_width().min(396.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(card_size), Sense::hover());
        let painter = ui.painter_at(rect.expand(4.0));
        painter.rect_filled(rect, Rounding::same(22.0), QR_CARD);

        // Snap the module size to whole pixels so every square stays crisp
        // (no anti-aliased gray edges the phone camera struggles with).
        let ppp = ui.ctx().pixels_per_point();
        let w = qr.width;
        let pad_min = card_size * 0.06;
        let m = (((card_size - 2.0 * pad_min) / w as f32) * ppp).floor() / ppp;
        let qr_side = m * w as f32;
        let origin = Pos2::new(
            rect.center().x - qr_side / 2.0,
            rect.center().y - qr_side / 2.0,
        );

        for y in 0..w {
            for x in 0..w {
                if !qr.dark[y * w + x] {
                    continue;
                }
                let r = Rect::from_min_size(
                    Pos2::new(origin.x + x as f32 * m, origin.y + y as f32 * m),
                    Vec2::splat(m),
                );
                painter.rect_filled(r, Rounding::ZERO, QR_MODULE);
            }
        }
    }
}

impl eframe::App for PairApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let p = theme::frame_palette(ctx);
        let state = self.state.lock().unwrap().clone();

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.bg).inner_margin(24.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(logo) = &self.logo {
                        ui.add(
                            egui::Image::new(logo)
                                .fit_to_exact_size(Vec2::splat(56.0))
                                .rounding(Rounding::same(14.0)),
                        );
                    }
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Linux Link").size(24.0).strong().color(p.text),
                    );
                    ui.label(egui::RichText::new("Pair a device").size(14.0).color(p.dim));
                    ui.add_space(18.0);

                    match &state {
                        PairState::Connecting => {
                            ui.add_space(60.0);
                            ui.spinner();
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("One moment…").color(p.dim));
                        }
                        PairState::Waiting { payload, since } => {
                            self.draw_qr(ui, payload);
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new("Scan this code with the Linux Link app")
                                    .size(15.0)
                                    .color(p.text),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("On the phone: Pair your PC")
                                    .size(12.5)
                                    .color(p.dim),
                            );
                            // The remaining time as a quietly draining bar —
                            // legible at a glance, no clock to read.
                            let left = PAIR_WINDOW_SECS
                                .saturating_sub(since.elapsed().as_secs());
                            let frac =
                                (left as f32 / PAIR_WINDOW_SECS as f32).clamp(0.0, 1.0);
                            ui.add_space(12.0);
                            let width = ui.available_width().min(280.0);
                            let (bar, _) = ui.allocate_exact_size(
                                Vec2::new(width, 6.0),
                                Sense::hover(),
                            );
                            let painter = ui.painter();
                            painter.rect_filled(bar, Rounding::same(3.0), p.raised);
                            let mut fill = bar;
                            fill.set_width(width * frac);
                            painter.rect_filled(fill, Rounding::same(3.0), p.accent);
                        }
                        PairState::Paired { name, at } => {
                            ui.add_space(50.0);
                            ui.label(egui::RichText::new("✔").size(52.0).color(p.ok));
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(format!("Paired with {name}"))
                                    .size(18.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Connected and ready — nothing else to do.")
                                    .size(13.0)
                                    .color(p.dim),
                            );
                            if at.elapsed().as_secs_f32() > 2.5 {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        PairState::Timeout => {
                            ui.add_space(60.0);
                            ui.label(egui::RichText::new("⏱").size(44.0).color(p.dim));
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("The code expired")
                                    .size(17.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("Codes stop working after two minutes.")
                                    .size(12.5)
                                    .color(p.dim),
                            );
                            ui.add_space(14.0);
                            if theme::primary_button(ui, &p, "Show a new code").clicked() {
                                start_pairing(self.state.clone());
                            }
                        }
                        PairState::Error(msg) => {
                            ui.add_space(50.0);
                            ui.label(egui::RichText::new("⚠").size(44.0).color(p.warn));
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(msg).size(14.0).color(p.text));
                            ui.add_space(14.0);
                            if theme::primary_button(ui, &p, "Try again").clicked() {
                                start_pairing(self.state.clone());
                            }
                        }
                    }
                });
            });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([470.0, 700.0])
            .with_min_inner_size([440.0, 650.0])
            .with_resizable(false)
            .with_app_id("linux-link")
            .with_title("Linux Link — Pairing"),
        ..Default::default()
    };
    eframe::run_native(
        "Linux Link — Pairing",
        options,
        Box::new(|cc| Ok(Box::new(PairApp::new(cc)))),
    )
}
