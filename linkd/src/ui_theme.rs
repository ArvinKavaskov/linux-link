//! The Linux Link design system for the desktop windows.
//!
//! One file, shared by every egui binary through `#[path]`, so the pairing
//! window and the settings window can never drift apart. The rules:
//!
//! * **One accent, and it is not a colour.** Near-black on light, soft
//!   white on dark — the interface is monochrome by intent, and colour is
//!   reserved for *meaning*: the green of a live link, the red of a
//!   destructive act. The logo keeps its violet; the chrome stays quiet.
//! * **Light and dark**, following the system. Nothing is hard-coded to a
//!   mode; every color goes through [`Palette`].
//! * **A spacing scale** (4 / 8 / 12 / 16 / 20 / 24) and two radii: 16 for
//!   cards, 12 for controls. Pills and switches are fully round.
//! * **Motion is state change only** — the switch knob slides, the toast
//!   fades. Nothing moves for decoration.

use eframe::egui::{self, Color32, Response, Rounding as Radius, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    /// A surface one step above a card: field wells, the QR frame.
    pub raised: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub outline: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: Color32::from_rgb(0x00, 0x00, 0x00),
            card: Color32::from_rgb(0x14, 0x14, 0x16),
            raised: Color32::from_rgb(0x24, 0x24, 0x27),
            text: Color32::from_rgb(0xF5, 0xF5, 0xF7),
            dim: Color32::from_rgb(0x98, 0x98, 0x9E),
            accent: Color32::from_rgb(0xF5, 0xF5, 0xF7),
            on_accent: Color32::from_rgb(0x11, 0x11, 0x13),
            ok: Color32::from_rgb(0x4C, 0xC9, 0x6E),
            warn: Color32::from_rgb(0xE5, 0x6B, 0x6B),
            outline: Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 0x1C),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(0xF4, 0xF4, 0xF6),
            card: Color32::WHITE,
            raised: Color32::from_rgb(0xEC, 0xEC, 0xEF),
            text: Color32::from_rgb(0x11, 0x11, 0x13),
            dim: Color32::from_rgb(0x6E, 0x6E, 0x73),
            accent: Color32::from_rgb(0x11, 0x11, 0x13),
            on_accent: Color32::WHITE,
            ok: Color32::from_rgb(0x1F, 0x9D, 0x4D),
            warn: Color32::from_rgb(0xC9, 0x41, 0x41),
            outline: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0x1A),
        }
    }
}

/// Reads the resolved system theme and returns the matching palette, after
/// applying the shared style (spacing scale, control shapes, focus color) to
/// the context. Call it once at the top of every frame.
pub fn frame_palette(ctx: &egui::Context) -> Palette {
    let dark = ctx.theme() == egui::Theme::Dark;
    let p = palette(dark);
    apply(ctx, &p);
    p
}

fn apply(ctx: &egui::Context, p: &Palette) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.spacing.interact_size = Vec2::new(40.0, 32.0);

    let v = &mut style.visuals;
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.accent);

    // Quiet controls: filled a step above the card, no border noise, and the
    // accent only appears on focus rings and primary actions.
    let radius = Radius::same(12.0);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = radius;
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.0, p.text);
    }
    v.widgets.inactive.weak_bg_fill = p.raised;
    v.widgets.inactive.bg_fill = p.raised;
    v.widgets.hovered.weak_bg_fill = lighten(p.raised, if v.dark_mode { 12 } else { -8 });
    v.widgets.hovered.bg_fill = v.widgets.hovered.weak_bg_fill;
    v.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.5);
    v.widgets.active.bg_fill = v.widgets.active.weak_bg_fill;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.outline);

    ctx.set_style(style);
}

fn lighten(c: Color32, by: i16) -> Color32 {
    let f = |x: u8| (x as i16 + by).clamp(0, 255) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

// ------------------------------------------------------------- components

/// A content card: radius 16, hairline outline, 16 px padding, full width.
pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = egui::Frame::none()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.outline))
        .rounding(Radius::same(16.0))
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner;
    ui.add_space(12.0);
    out
}

/// The small dim capitals that open a card. One per card, nothing louder.
pub fn section_title(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).strong().color(p.dim));
    ui.add_space(8.0);
}

/// One line of quiet explanation under a control.
pub fn hint(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(p.dim));
}

/// The one loud control on a screen: accent-filled pill, generous padding.
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let padded = format!("  {text}  ");
    ui.add(
        egui::Button::new(
            egui::RichText::new(padded).size(14.0).strong().color(p.on_accent),
        )
        .fill(p.accent)
        .rounding(Radius::same(20.0))
        .min_size(Vec2::new(0.0, 38.0)),
    )
}

/// A destructive confirmation. Red is earned, not decorative.
pub fn danger_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(13.0).color(Color32::WHITE))
            .fill(p.warn)
            .rounding(Radius::same(20.0)),
    )
}

/// A quiet inline action: no fill until hovered, dim text.
pub fn quiet_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(13.0).color(p.dim))
            .fill(Color32::TRANSPARENT)
            .rounding(Radius::same(20.0)),
    )
}

/// A status dot: a filled circle when on, a ring when off.
pub fn status_dot(ui: &mut Ui, color: Color32, on: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    let c = rect.center();
    if on {
        ui.painter().circle_filled(c, 4.0, color);
    } else {
        ui.painter().circle_stroke(c, 3.5, Stroke::new(1.5, color));
    }
}

/// The switch. Slides in 120 ms on egui's own animation clock, so it runs at
/// whatever the display refreshes at. Returns `changed()` like a checkbox.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let size = Vec2::new(42.0, 24.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui
            .ctx()
            .animate_bool_with_time(response.id, *on, 0.12);
        let track = mix(p.raised, p.accent, t);
        // The knob must read against both ends of the track: it borrows the
        // accent's own contrast colour once the switch is on.
        let knob_colour = mix(Color32::WHITE, p.on_accent, t);
        let painter = ui.painter();
        painter.rect_filled(rect, Radius::same(12.0), track);
        painter.rect_stroke(rect, Radius::same(12.0), Stroke::new(1.0, p.outline));
        let knob_x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), t);
        let knob = egui::Pos2::new(knob_x, rect.center().y);
        painter.circle_filled(knob, 9.0, knob_colour);
        painter.circle_stroke(knob, 9.0, Stroke::new(1.0, p.outline));
    }
    response
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A full settings row: title + optional hint on the left, a switch on the
/// right, the whole row clickable. Returns true when the value flipped.
pub fn switch_row(ui: &mut Ui, p: &Palette, title: &str, sub: Option<&str>, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 58.0);
            ui.label(egui::RichText::new(title).size(14.0).color(p.text));
            if let Some(sub) = sub {
                hint(ui, p, sub);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if toggle(ui, p, on).changed() {
                changed = true;
            }
        });
    });
    changed
}
