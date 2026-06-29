//! Visual theme for the Stow window: palette, Manrope fonts, and egui styling.
//!
//! Mirrors the `StowWindow.dc.html` design spec (warm cream background, orange
//! accent). Typography uses Manrope at several weights, registered as named
//! font families so headings/labels can pick a weight per text run.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, RichText,
    Stroke, Style,
};

// ── Palette (from the design spec) ───────────────────────────────────────────
pub const WIN_BG: Color32 = Color32::from_rgb(0xFA, 0xFA, 0xF7);
pub const SURFACE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const SURFACE2: Color32 = Color32::from_rgb(0xF4, 0xF2, 0xEC);
pub const INK: Color32 = Color32::from_rgb(0x0F, 0x0F, 0x12);
pub const INK2: Color32 = Color32::from_rgb(0x3D, 0x3D, 0x44);
pub const MUTED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
pub const MUTED_SOFT: Color32 = Color32::from_rgb(0x9C, 0xA0, 0xAB);
pub const LINE: Color32 = Color32::from_rgb(0xEC, 0xEA, 0xE3);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0xF2, 0xF0, 0xEA);
pub const ACCENT: Color32 = Color32::from_rgb(0xF2, 0x6A, 0x21);
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(0xC2, 0x4E, 0x12);
pub const SUCCESS: Color32 = Color32::from_rgb(0x0A, 0x7D, 0x4A);
pub const ERROR: Color32 = Color32::from_rgb(0xC1, 0x35, 0x2A);
pub const FIELD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const DZ: Color32 = Color32::from_rgb(0xFB, 0xFA, 0xF6);

/// Install Manrope (regular plus four heavier weights as named families).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let faces = [
        ("mr-regular", include_bytes!("../assets/fonts/Manrope-Regular.ttf").as_slice()),
        ("mr-medium", include_bytes!("../assets/fonts/Manrope-Medium.ttf").as_slice()),
        ("mr-semibold", include_bytes!("../assets/fonts/Manrope-SemiBold.ttf").as_slice()),
        ("mr-bold", include_bytes!("../assets/fonts/Manrope-Bold.ttf").as_slice()),
        ("mr-extrabold", include_bytes!("../assets/fonts/Manrope-ExtraBold.ttf").as_slice()),
    ];
    for (name, bytes) in faces {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }

    // Regular becomes the default proportional face.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "mr-regular".to_owned());

    // Heavier weights as named families, selected per text run.
    for (fam, face) in [
        ("medium", "mr-medium"),
        ("semibold", "mr-semibold"),
        ("bold", "mr-bold"),
        ("extrabold", "mr-extrabold"),
    ] {
        fonts
            .families
            .insert(FontFamily::Name(fam.into()), vec![face.to_owned()]);
    }

    ctx.set_fonts(fonts);
}

/// Apply the light, warm visual style.
pub fn apply_style(ctx: &egui::Context) {
    let mut style = Style::default();
    let v = &mut style.visuals;
    v.dark_mode = false;
    v.override_text_color = Some(INK);
    v.panel_fill = WIN_BG;
    v.window_fill = WIN_BG;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = SURFACE2;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let r = CornerRadius::same(9);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
        w.bg_fill = SURFACE;
        w.weak_bg_fill = SURFACE;
        w.bg_stroke = Stroke::new(1.0, LINE);
        w.fg_stroke = Stroke::new(1.0, INK2);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 28.0;

    ctx.set_style(style);
}

// ── Typed text helpers ───────────────────────────────────────────────────────
fn fam(name: &'static str) -> FontFamily {
    FontFamily::Name(name.into())
}

pub fn reg(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color)
}
pub fn med(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(fam("medium"))
}
pub fn semi(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(fam("semibold"))
}
pub fn bold(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(fam("bold"))
}
pub fn extra(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(fam("extrabold"))
}
pub fn mono(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).monospace()
}

/// A card frame (white surface, hairline border, rounded).
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
}

/// A drop-zone frame (soft tint, rounded; solid border approximates the
/// design's dashed border, which egui cannot draw natively).
pub fn drop_frame(active_border: bool) -> egui::Frame {
    let stroke = if active_border {
        Stroke::new(1.5, ACCENT)
    } else {
        Stroke::new(1.5, LINE)
    };
    egui::Frame::new()
        .fill(DZ)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
}
