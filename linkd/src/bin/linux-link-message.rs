use eframe::egui::{self, Vec2};

#[path = "../ui_theme.rs"]
#[allow(dead_code)]
mod theme;

struct MessageApp {
    app: String,
    title: String,
    body: String,
    copied_at: Option<std::time::Instant>,
}

impl eframe::App for MessageApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        theme::window_frame(ctx, "", false, |ui, p| {
            {
                ui.label(egui::RichText::new(&self.app).size(12.0).strong().color(p.dim));
                if !self.title.is_empty() {
                    ui.label(egui::RichText::new(&self.title).size(18.0).strong().color(p.text));
                }
                ui.add_space(10.0);
                let footer = 46.0;
                let body_height = ui.available_height() - footer;
                theme::card(ui, p, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(body_height - 44.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.body).size(14.5).color(p.text),
                                )
                                .wrap(),
                            );
                        });
                });
                ui.horizontal(|ui| {
                    let label = match self.copied_at {
                        Some(t) if t.elapsed().as_secs_f32() < 1.5 => "Copied ✔",
                        _ => "Copy",
                    };
                    if theme::primary_button(ui, p, label).clicked() {
                        ctx.copy_text(self.body.clone());
                        self.copied_at = Some(std::time::Instant::now());
                        ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    }
                    if theme::quiet_button(ui, p, "Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
        });
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let app = args.next().unwrap_or_else(|| "Message".into());
    let title = args.next().unwrap_or_default();
    let body = args
        .next()
        .and_then(|p| {
            let text = std::fs::read_to_string(&p).ok();
            let _ = std::fs::remove_file(&p);
            text
        })
        .unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(Vec2::new(440.0, 520.0))
            .with_min_inner_size(Vec2::new(320.0, 240.0))
            .with_app_id("linux-link-message")
            .with_decorations(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "Message",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(MessageApp { app, title, body, copied_at: None }))
        }),
    )
}
