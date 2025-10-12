#[cfg(feature = "egui")]
use egui::{Color32, Context, Visuals};

#[cfg(feature = "egui")]
#[derive(Clone, Debug)]
pub struct UiStyle {
    /// Background color for windows (with alpha)
    pub window_fill: Color32,
    /// Background color for panels (with alpha)
    pub panel_fill: Color32,
    /// Whether to use dark or light theme as base
    pub dark_mode: bool,
    /// Opacity for window backgrounds (0.0 - 1.0)
    pub window_opacity: f32,
    /// Opacity for panel backgrounds (0.0 - 1.0)
    pub panel_opacity: f32,
}

#[cfg(feature = "egui")]
impl Default for UiStyle {
    fn default() -> Self {
        Self {
            window_fill: Color32::from_rgba_unmultiplied(27, 27, 27, 230),
            panel_fill: Color32::from_rgba_unmultiplied(27, 27, 27, 200),
            dark_mode: true,
            window_opacity: 0.9,
            panel_opacity: 0.8,
        }
    }
}

#[cfg(feature = "egui")]
impl UiStyle {
    /// Create a new style with custom opacity values
    pub fn with_opacity(window_opacity: f32, panel_opacity: f32) -> Self {
        let mut style = Self::default();
        style.window_opacity = window_opacity.clamp(0.0, 1.0);
        style.panel_opacity = panel_opacity.clamp(0.0, 1.0);
        style.apply_opacity();
        style
    }

    /// Create a fully transparent style
    pub fn transparent() -> Self {
        Self::with_opacity(0.0, 0.0)
    }

    /// Create a semi-transparent style
    pub fn semi_transparent() -> Self {
        Self::with_opacity(0.85, 0.75)
    }

    /// Apply opacity values to the colors
    fn apply_opacity(&mut self) {
        let base_window = if self.dark_mode {
            Color32::from_gray(27)
        } else {
            Color32::from_gray(248)
        };
        
        let base_panel = if self.dark_mode {
            Color32::from_gray(27)
        } else {
            Color32::from_gray(245)
        };

        self.window_fill = Color32::from_rgba_unmultiplied(
            base_window.r(),
            base_window.g(),
            base_window.b(),
            (255.0 * self.window_opacity) as u8,
        );

        self.panel_fill = Color32::from_rgba_unmultiplied(
            base_panel.r(),
            base_panel.g(),
            base_panel.b(),
            (255.0 * self.panel_opacity) as u8,
        );
    }

    /// Set opacity and recalculate colors
    pub fn set_window_opacity(&mut self, opacity: f32) {
        self.window_opacity = opacity.clamp(0.0, 1.0);
        self.apply_opacity();
    }

    /// Set panel opacity and recalculate colors
    pub fn set_panel_opacity(&mut self, opacity: f32) {
        self.panel_opacity = opacity.clamp(0.0, 1.0);
        self.apply_opacity();
    }

    /// Apply this style to an egui context
    pub fn apply(&self, ctx: &Context) {
        let mut visuals = if self.dark_mode {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        // Set window and panel backgrounds
        visuals.window_fill = self.window_fill;
        visuals.panel_fill = self.panel_fill;
        
        // Optional: also make popup backgrounds transparent
        visuals.window_shadow.color = Color32::from_black_alpha(60);
        
        ctx.set_visuals(visuals);
    }
}