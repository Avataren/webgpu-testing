//! Look and feel of the editor: colors, spacing and a few shared widgets.
//!
//! Icons are glyphs from egui's bundled fonts only (`emoji-icon-font` and Noto Emoji);
//! anything else renders as a missing-glyph box.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2};

/// Editor color palette.
pub mod colors {
    use egui::Color32;

    /// Tab bars, text fields and other recessed areas.
    pub const BG_DEEP: Color32 = Color32::from_rgb(0x15, 0x17, 0x1c);
    /// Dock panes.
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x1d, 0x20, 0x26);
    /// Floating windows and menus.
    pub const BG_WINDOW: Color32 = Color32::from_rgb(0x23, 0x26, 0x2e);
    /// Component headers and grouped blocks.
    pub const BG_RAISED: Color32 = Color32::from_rgb(0x27, 0x2b, 0x33);

    pub const WIDGET: Color32 = Color32::from_rgb(0x2d, 0x31, 0x3a);
    pub const WIDGET_HOVER: Color32 = Color32::from_rgb(0x38, 0x3d, 0x48);
    pub const WIDGET_ACTIVE: Color32 = Color32::from_rgb(0x42, 0x48, 0x55);

    pub const BORDER: Color32 = Color32::from_rgb(0x30, 0x34, 0x3d);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x3d, 0x42, 0x4d);

    pub const TEXT: Color32 = Color32::from_rgb(0xd5, 0xd8, 0xdf);
    pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xf2, 0xf4, 0xf7);
    pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8a, 0x90, 0x9c);

    pub const ACCENT: Color32 = Color32::from_rgb(0x4d, 0x8e, 0xff);
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x24, 0x47, 0x80);

    pub const SUCCESS: Color32 = Color32::from_rgb(0x5c, 0xc9, 0x8a);
    pub const WARNING: Color32 = Color32::from_rgb(0xe5, 0xb5, 0x4b);
    pub const ERROR: Color32 = Color32::from_rgb(0xf0, 0x61, 0x6d);

    // Asset and component kinds.
    pub const FOLDER: Color32 = Color32::from_rgb(0xe0, 0xb0, 0x5a);
    pub const SCRIPT: Color32 = Color32::from_rgb(0x6c, 0xa8, 0xff);
    pub const SCENE: Color32 = Color32::from_rgb(0xb4, 0x8c, 0xff);
    pub const MATERIAL: Color32 = Color32::from_rgb(0xff, 0x8f, 0x6b);
    pub const IMAGE: Color32 = Color32::from_rgb(0x5c, 0xc9, 0x8a);
    pub const MODEL: Color32 = Color32::from_rgb(0x4f, 0xc7, 0xd1);
    pub const SHADER: Color32 = Color32::from_rgb(0xe8, 0x78, 0xc8);
    pub const AUDIO: Color32 = Color32::from_rgb(0xd9, 0xc8, 0x5a);
}

/// Icon glyphs available in egui's bundled fonts.
pub mod icons {
    pub const PLAY: &str = "▶";
    pub const PAUSE: &str = "⏸";
    pub const STOP: &str = "⏹";
    pub const ADD: &str = "✚";
    pub const CLOSE: &str = "×";
    pub const SEARCH: &str = "🔍";
    pub const SAVE: &str = "💾";
    pub const DELETE: &str = "🗑";
    pub const RENAME: &str = "✏";
    pub const DUPLICATE: &str = "🗐";
    pub const REVERT: &str = "⟲";
    pub const UP: &str = "⬆";
    pub const FOLDER: &str = "🗁";
    pub const WARNING: &str = "⚠";

    pub const MESH: &str = "▣";
    pub const CAMERA: &str = "🎥";
    pub const LIGHT: &str = "💡";
    pub const SUN: &str = "☀";
    pub const SPOT: &str = "🔦";
    pub const PARTICLES: &str = "✨";
    pub const ENVIRONMENT: &str = "🌐";
    pub const POST_PROCESS: &str = "🎞";
    pub const LIGHT_PROBES: &str = "🔆";
    pub const RIGID_BODY: &str = "⚫";
    pub const COLLIDER: &str = "⬚";
    pub const CHARACTER: &str = "🏃";
    pub const ANIMATOR: &str = "🎬";
    pub const ANIMATION_TREE: &str = "🔀";
    pub const SPRING_CAMERA: &str = "📷";
    #[cfg(feature = "navigation")]
    pub const NAVMESH: &str = "🗺";
    #[cfg(feature = "navigation")]
    pub const NAV_AGENT: &str = "🚶";
    #[cfg(feature = "navigation")]
    pub const NAV_OBSTACLE: &str = "🚧";
    #[cfg(feature = "navigation")]
    pub const NAV_MODIFIER: &str = "🔧";
    #[cfg(feature = "navigation")]
    pub const NAV_LINK: &str = "🔗";
    pub const SCRIPT: &str = "📜";
    pub const TRANSFORM: &str = "✚";
    pub const MATERIAL: &str = "🎨";
}

/// Apply the editor theme to an egui style.
pub fn apply_editor_style(style: &mut egui::Style) {
    use colors::*;

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(17.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(6.0, 5.0);
    spacing.button_padding = Vec2::new(8.0, 3.0);
    spacing.window_margin = Margin::same(10);
    spacing.menu_margin = Margin::same(6);
    spacing.indent = 16.0;
    spacing.interact_size = Vec2::new(36.0, 22.0);
    spacing.combo_width = 120.0;
    spacing.icon_width = 14.0;
    spacing.icon_spacing = 5.0;

    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.override_text_color = None;
    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = BG_WINDOW;
    visuals.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(100),
    };
    visuals.extreme_bg_color = BG_DEEP;
    visuals.text_edit_bg_color = Some(BG_DEEP);
    visuals.faint_bg_color = Color32::from_rgb(0x21, 0x24, 0x2b);
    visuals.code_bg_color = BG_DEEP;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.striped = false;
    visuals.indent_has_left_vline = true;
    visuals.collapsing_header_frame = true;
    visuals.slider_trailing_fill = true;

    let radius = CornerRadius::same(4);
    let widgets = &mut visuals.widgets;

    widgets.noninteractive.bg_fill = BG_PANEL;
    widgets.noninteractive.weak_bg_fill = BG_PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.noninteractive.corner_radius = radius;

    widgets.inactive.bg_fill = WIDGET;
    widgets.inactive.weak_bg_fill = WIDGET;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.inactive.corner_radius = radius;

    widgets.hovered.bg_fill = WIDGET_HOVER;
    widgets.hovered.weak_bg_fill = WIDGET_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
    widgets.hovered.corner_radius = radius;
    widgets.hovered.expansion = 0.0;

    widgets.active.bg_fill = WIDGET_ACTIVE;
    widgets.active.weak_bg_fill = WIDGET_ACTIVE;
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
    widgets.active.corner_radius = radius;
    widgets.active.expansion = 0.0;

    widgets.open.bg_fill = WIDGET_HOVER;
    widgets.open.weak_bg_fill = WIDGET_HOVER;
    widgets.open.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    widgets.open.fg_stroke = Stroke::new(1.0, TEXT_STRONG);
    widgets.open.corner_radius = radius;
}

/// Inner frame for dock panes other than the viewports.
pub fn pane_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(colors::BG_PANEL)
        .inner_margin(Margin::symmetric(8, 6))
}

/// Small weak text, e.g. hints and secondary labels.
pub fn hint(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(colors::TEXT_WEAK)
}

/// A muted, centered message for empty panes ("Nothing selected").
pub fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(title)
                .color(colors::TEXT_WEAK)
                .size(14.0),
        );
        if !detail.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(detail).color(colors::TEXT_WEAK).small());
        }
    });
}

/// A collapsible component section with a strong header, open by default.
///
/// Returns the header response, so callers can attach a context menu.
pub fn section(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let text = egui::RichText::new(format!("{icon}  {title}"))
        .strong()
        .color(colors::TEXT_STRONG);
    let response = egui::CollapsingHeader::new(text)
        .id_salt(("inspector_section", title))
        .default_open(true)
        .show(ui, |ui| {
            ui.add_space(2.0);
            add_contents(ui);
            ui.add_space(2.0);
        });
    response.header_response
}

/// A button with an accent fill, for the primary action of a group.
pub fn primary_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).fill(colors::ACCENT_DIM)
}

/// Paint a short colored type badge ("LUA", "PNG", ...) and return its response.
pub fn type_badge(ui: &mut egui::Ui, label: &str, color: Color32) -> egui::Response {
    let font = FontId::new(9.0, FontFamily::Monospace);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::BLACK);
    let size = Vec2::new(galley.size().x.max(18.0) + 6.0, 14.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), color.gamma_multiply(0.85));
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        Color32::from_rgb(0x12, 0x14, 0x18),
    );
    response
}
