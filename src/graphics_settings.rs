//! Graphics settings the player chooses: display, frame pacing, anti-aliasing, upscaling,
//! ray-traced effects and the FPS readout. They belong to the player and the game, not
//! to a scene: they are applied over whatever the scene's post-process volumes ask for
//! (the player's choice always wins) and saved per user and project next to the save
//! data (`<user data dir>/wgpu-cube/settings/<project>/graphics.txt`, `localStorage` on
//! the web).
//!
//! Every app gets a settings page for them (see `ui::settings_page`, with the `egui`
//! feature): Escape opens and closes it unless the app or a script turns the hotkey off
//! (`set_settings_hotkey(false)` in Lua), and scripts open it from their own menus with
//! `open_settings()`.

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};

use crate::renderer::postprocess::{AntiAliasing, PostProcessSettings, Upscaler};

/// Where the settings are saved among the per-user files (see `save_user_file`).
const AREA: &str = "settings";
const KEY: &str = "graphics";

/// How the window is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    Windowed,
    /// A borderless window covering the monitor it is on.
    Fullscreen,
}

/// Reflections and ambient occlusion: off, screen space, or ray traced (which falls back
/// to screen space without hardware ray tracing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectQuality {
    Off,
    ScreenSpace,
    #[default]
    RayTraced,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsSettings {
    pub display_mode: DisplayMode,
    /// Window size in physical pixels while windowed; `None` keeps the size the app opened
    /// with.
    pub window_size: Option<[u32; 2]>,
    /// Wait for the display's refresh (no tearing, frame rate capped at the refresh
    /// rate). Off presents as soon as a frame is ready.
    pub vsync: bool,
    pub anti_aliasing: AntiAliasing,
    /// Share of the output resolution the scene renders at (1 = native), upscaled with
    /// `upscaler`.
    pub render_scale: f32,
    pub upscaler: Upscaler,
    /// FSR's sharpening, 0 (off) to 1.
    pub sharpness: f32,
    pub ray_traced_shadows: bool,
    pub reflections: EffectQuality,
    pub ambient_occlusion: EffectQuality,
    /// Light bends through glass and gems (otherwise they are drawn see-through).
    pub ray_traced_refraction: bool,
    /// The frame rate in the corner of the view.
    pub show_fps: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            display_mode: DisplayMode::Windowed,
            window_size: None,
            vsync: false,
            anti_aliasing: AntiAliasing::Taa,
            render_scale: 1.0,
            upscaler: Upscaler::Fsr,
            sharpness: 0.5,
            ray_traced_shadows: true,
            reflections: EffectQuality::RayTraced,
            ambient_occlusion: EffectQuality::RayTraced,
            ray_traced_refraction: true,
            show_fps: true,
        }
    }
}

impl GraphicsSettings {
    /// Render scales offered as presets (FSR's quality modes).
    pub const RENDER_SCALE_PRESETS: [(&'static str, f32); 5] = [
        ("Native", 1.0),
        ("Quality", 0.67),
        ("Balanced", 0.58),
        ("Performance", 0.5),
        ("Ultra performance", 0.33),
    ];

    /// Window sizes offered while windowed.
    pub const WINDOW_SIZES: [[u32; 2]; 6] = [
        [1280, 720],
        [1600, 900],
        [1920, 1080],
        [2560, 1440],
        [3200, 1800],
        [3840, 2160],
    ];

    /// Post-processing settings (dotted paths) these replace: the scene's volumes set
    /// them in vain.
    pub const CONTROLLED_PATHS: [&'static str; 8] = [
        "anti_aliasing",
        "upscaling",
        "rt_shadows.enabled",
        "ssr.enabled",
        "ssr.ray_traced",
        "ssao.enabled",
        "ssao.ray_traced",
        "refraction.ray_traced",
    ];

    /// `post`, the scene's post-processing, with the player's choices in place.
    pub fn apply(&self, mut post: PostProcessSettings) -> PostProcessSettings {
        post.anti_aliasing = self.anti_aliasing;
        post.upscaling.render_scale = self.render_scale;
        post.upscaling.method = self.upscaler;
        post.upscaling.sharpness = self.sharpness;
        post.rt_shadows.enabled = self.ray_traced_shadows;
        post.ssr.enabled = self.reflections != EffectQuality::Off;
        post.ssr.ray_traced = self.reflections == EffectQuality::RayTraced;
        post.ssao.enabled = self.ambient_occlusion != EffectQuality::Off;
        post.ssao.ray_traced = self.ambient_occlusion == EffectQuality::RayTraced;
        post.refraction.ray_traced = self.ray_traced_refraction;
        post
    }

    /// Clamp values into range (after loading or a remote change).
    pub fn sanitized(mut self) -> Self {
        let finite_or =
            |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        self.render_scale = finite_or(self.render_scale, 1.0).clamp(
            crate::renderer::postprocess::UpscalingSettings::MIN_RENDER_SCALE,
            1.0,
        );
        self.sharpness = finite_or(self.sharpness, 0.5).clamp(0.0, 1.0);
        self.window_size = self
            .window_size
            .filter(|size| size[0] >= 320 && size[1] >= 200)
            .map(|size| [size[0].min(16384), size[1].min(16384)]);
        self
    }
}

/// What the renderer can do, for the settings page to show which options take effect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphicsCapabilities {
    /// Hardware ray tracing (otherwise ray-traced effects fall back to raster ones).
    pub ray_tracing: bool,
    /// Upscaling works (it does not with MSAA, which renders at full resolution).
    pub upscaling: bool,
    /// Present modes the surface offers besides vsync (an off switch for vsync).
    pub vsync_optional: bool,
}

/// The settings and the page's state, shared by the app, its UI, scripts and remote
/// control.
#[derive(Debug, Default)]
pub struct GraphicsSettingsState {
    pub settings: GraphicsSettings,
    /// Whether the settings page is showing.
    pub open: bool,
    /// Whether Escape opens and closes the page (apps and scripts can turn it off).
    pub hotkey_enabled: bool,
    /// Whether the settings may change the window (display mode, size): not in the
    /// editor, whose window is not the game's.
    pub manage_window: bool,
    /// Whether the page offers to close the app.
    pub exit_button: bool,
    pub capabilities: GraphicsCapabilities,
    /// The project the settings were loaded for; another one loads its own.
    namespace: Option<String>,
    /// Bumped on every change, so the window and surface apply new values once.
    revision: u64,
}

impl GraphicsSettingsState {
    /// A changed value: save it, and let the window and surface pick it up.
    pub fn set(&mut self, settings: GraphicsSettings) {
        let settings = settings.sanitized();
        if settings != self.settings {
            self.settings = settings;
            self.revision += 1;
            self.save();
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Load this project's settings if they are not the ones in use (the project the save
    /// data belongs to changed, or this is the first frame).
    pub fn sync_namespace(&mut self) {
        let namespace = crate::scripting::lua::api::save_namespace();
        if self.namespace.as_deref() == Some(namespace.as_str()) {
            return;
        }
        self.namespace = Some(namespace);
        self.settings = load().unwrap_or_default();
        self.revision += 1;
    }

    fn save(&self) {
        let text = match serde_json::to_string_pretty(&self.settings) {
            Ok(text) => text,
            Err(err) => {
                log::warn!("Cannot encode graphics settings: {err}");
                return;
            }
        };
        if let Err(err) = crate::scripting::lua::api::save_user_file(AREA, KEY, Some(&text)) {
            log::warn!("Cannot save graphics settings: {err}");
        }
    }
}

fn load() -> Option<GraphicsSettings> {
    let text = match crate::scripting::lua::api::load_user_file(AREA, KEY) {
        Ok(text) => text?,
        Err(err) => {
            log::warn!("Cannot read graphics settings: {err}");
            return None;
        }
    };
    match serde_json::from_str::<GraphicsSettings>(&text) {
        Ok(settings) => Some(settings.sanitized()),
        Err(err) => {
            log::warn!("Ignoring unreadable graphics settings: {err}");
            None
        }
    }
}

static STATE: OnceLock<Mutex<GraphicsSettingsState>> = OnceLock::new();

/// The process's graphics settings (one app per process).
pub fn graphics_settings() -> MutexGuard<'static, GraphicsSettingsState> {
    STATE
        .get_or_init(|| {
            Mutex::new(GraphicsSettingsState {
                hotkey_enabled: true,
                manage_window: true,
                exit_button: true,
                ..GraphicsSettingsState::default()
            })
        })
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_players_choices_replace_the_scenes() {
        let mut scene = PostProcessSettings::default();
        scene.ssr.enabled = false;
        scene.rt_shadows.enabled = false;
        scene.anti_aliasing = AntiAliasing::Fxaa;
        let settings = GraphicsSettings {
            anti_aliasing: AntiAliasing::Taa,
            render_scale: 0.5,
            reflections: EffectQuality::ScreenSpace,
            ambient_occlusion: EffectQuality::Off,
            ..GraphicsSettings::default()
        };
        let applied = settings.apply(scene);
        assert_eq!(applied.anti_aliasing, AntiAliasing::Taa);
        assert_eq!(applied.upscaling.render_scale, 0.5);
        assert!(applied.ssr.enabled && !applied.ssr.ray_traced);
        assert!(!applied.ssao.enabled);
        assert!(applied.rt_shadows.enabled);
    }

    #[test]
    fn loading_repairs_bad_values() {
        let settings: GraphicsSettings = serde_json::from_str(
            r#"{"render_scale": 7, "sharpness": -1, "window_size": [10, 10]}"#,
        )
        .unwrap();
        let settings = settings.sanitized();
        assert_eq!(settings.render_scale, 1.0);
        assert_eq!(settings.sharpness, 0.0);
        assert_eq!(settings.window_size, None);
        // Fields missing from an older file keep their defaults.
        assert!(settings.show_fps);
    }
}
