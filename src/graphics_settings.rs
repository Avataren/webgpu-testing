//! Graphics settings the player chooses: display, frame pacing, anti-aliasing, upscaling,
//! shadow budget, ray-traced effects and the FPS readout. They belong to the player and the game, not
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

/// Diffuse light that arrives indirectly (bounced, from glowing surfaces, from the sky
/// through openings): off (the environment lights every surface as if nothing were in
/// the way) or ray traced (raster direct light plus traced, denoised indirect light).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalIllumination {
    #[default]
    Off,
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
    /// The point and spot lights' shadow atlas: the most texels across it grows to (a
    /// power of two from 1024 to 8192). Bigger holds more shadowed lights at higher
    /// resolution; each texel costs 8 bytes (the shadow maps and their static casters).
    pub shadow_atlas_size: u32,
    /// Point and spot light shadow views rebuilt per frame at most. Fewer cost less GPU
    /// and CPU time, and the moving shadows of lower-ranked lights then trail by a frame or
    /// more.
    pub shadow_updates_per_frame: u32,
    /// Hardware ray tracing at all. Off, every effect takes its raster version (shadow
    /// maps, screen-space reflections and ambient occlusion, see-through glass, no traced
    /// global illumination or path tracing) and light probes show their bakes, as on a
    /// GPU without ray tracing; the choices below are kept for when it comes back on.
    pub ray_tracing: bool,
    pub ray_traced_shadows: bool,
    pub reflections: EffectQuality,
    pub ambient_occlusion: EffectQuality,
    /// Light bends through glass and gems (otherwise they are drawn see-through).
    pub ray_traced_refraction: bool,
    /// Ray-traced indirect diffuse light over the raster lighting: between the raster
    /// lighting and path tracing in cost and look. Path tracing wins when both are on.
    pub global_illumination: GlobalIllumination,
    /// Path-traced lighting with a denoiser instead of the raster lighting: soft shadows,
    /// bounced light, emissive surfaces lighting the scene. Reflections and ambient
    /// occlusion come with it.
    pub path_tracing: bool,
    /// Indirect bounces per path (1 to 8).
    pub path_tracing_bounces: u32,
    /// Paths per pixel per frame (1 to 8).
    pub path_tracing_samples: u32,
    /// Denoise the paths (off shows the raw samples).
    pub path_tracing_denoiser: bool,
    /// Light focused through glass and off mirrors (caustic photons).
    pub path_tracing_caustics: bool,
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
            shadow_atlas_size: Self::DEFAULT_SHADOW_ATLAS_SIZE,
            shadow_updates_per_frame: Self::DEFAULT_SHADOW_UPDATES_PER_FRAME,
            ray_tracing: true,
            ray_traced_shadows: true,
            reflections: EffectQuality::RayTraced,
            ambient_occlusion: EffectQuality::RayTraced,
            ray_traced_refraction: true,
            global_illumination: GlobalIllumination::Off,
            path_tracing: false,
            path_tracing_bounces: 2,
            path_tracing_samples: 1,
            path_tracing_denoiser: true,
            path_tracing_caustics: true,
            show_fps: true,
        }
    }
}

impl GraphicsSettings {
    /// These settings with the fields `patch` names replaced (field names and values as
    /// they serialize, e.g. `{"reflections": "screen_space", "render_scale": 0.5}`).
    /// Unknown names and invalid values are errors.
    pub fn patched(
        &self,
        patch: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<GraphicsSettings, String> {
        let mut merged = serde_json::to_value(self).map_err(|err| err.to_string())?;
        let mut unknown = Vec::new();
        for (key, value) in patch {
            match merged.get_mut(key) {
                Some(slot) => *slot = value.clone(),
                None => unknown.push(key.clone()),
            }
        }
        if !unknown.is_empty() {
            return Err(format!("unknown graphics settings: {}", unknown.join(", ")));
        }
        serde_json::from_value(merged).map_err(|err| format!("invalid graphics settings: {err}"))
    }

    /// The shadow atlas's default size: 8192 natively, 4096 on the web.
    #[cfg(not(target_arch = "wasm32"))]
    pub const DEFAULT_SHADOW_ATLAS_SIZE: u32 = 8192;
    #[cfg(target_arch = "wasm32")]
    pub const DEFAULT_SHADOW_ATLAS_SIZE: u32 = 4096;
    pub const DEFAULT_SHADOW_UPDATES_PER_FRAME: u32 = 48;
    /// Shadow atlas sizes offered.
    pub const SHADOW_ATLAS_SIZES: [u32; 4] = [1024, 2048, 4096, 8192];

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
    pub const CONTROLLED_PATHS: [&'static str; 14] = [
        "anti_aliasing",
        "upscaling",
        "rt_shadows.enabled",
        "ssr.enabled",
        "ssr.ray_traced",
        "ssao.enabled",
        "ssao.ray_traced",
        "refraction.ray_traced",
        "global_illumination.enabled",
        "path_tracing.enabled",
        "path_tracing.bounces",
        "path_tracing.samples",
        "path_tracing.denoiser",
        "path_tracing.caustics",
    ];

    /// `post`, the scene's post-processing, with the player's choices in place.
    pub fn apply(&self, mut post: PostProcessSettings) -> PostProcessSettings {
        post.anti_aliasing = self.anti_aliasing;
        post.upscaling.render_scale = self.render_scale;
        post.upscaling.method = self.upscaler;
        post.upscaling.sharpness = self.sharpness;
        let rt = self.ray_tracing;
        post.rt_shadows.enabled = rt && self.ray_traced_shadows;
        post.ssr.enabled = self.reflections != EffectQuality::Off;
        post.ssr.ray_traced = rt && self.reflections == EffectQuality::RayTraced;
        post.ssao.enabled = self.ambient_occlusion != EffectQuality::Off;
        post.ssao.ray_traced = rt && self.ambient_occlusion == EffectQuality::RayTraced;
        post.refraction.ray_traced = rt && self.ray_traced_refraction;
        post.global_illumination.enabled =
            rt && self.global_illumination == GlobalIllumination::RayTraced;
        post.path_tracing.enabled = rt && self.path_tracing;
        post.path_tracing.bounces = self.path_tracing_bounces;
        post.path_tracing.samples = self.path_tracing_samples;
        post.path_tracing.denoiser = self.path_tracing_denoiser;
        post.path_tracing.caustics = self.path_tracing_caustics;
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
        use crate::renderer::postprocess::PathTracingSettings;
        self.path_tracing_bounces = self
            .path_tracing_bounces
            .clamp(1, PathTracingSettings::MAX_BOUNCES);
        self.path_tracing_samples = self
            .path_tracing_samples
            .clamp(1, PathTracingSettings::MAX_SAMPLES);
        self.window_size = self
            .window_size
            .filter(|size| size[0] >= 320 && size[1] >= 200)
            .map(|size| [size[0].min(16384), size[1].min(16384)]);
        self.shadow_atlas_size = self.shadow_atlas_size.clamp(1024, 8192).next_power_of_two();
        self.shadow_updates_per_frame = self.shadow_updates_per_frame.clamp(1, 1024);
        self
    }
}

/// What the renderer can do, for the settings page to show which options take effect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphicsCapabilities {
    /// Hardware ray tracing (otherwise ray-traced effects fall back to raster ones).
    pub ray_tracing: bool,
    /// Path tracing and ray-traced global illumination can run (hardware ray tracing and
    /// no MSAA).
    pub path_tracing: bool,
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
        if self.set_unsaved(settings) {
            self.save();
        }
    }

    /// Apply a changed value for this run only, without saving it (a benchmark trying
    /// configurations). Returns whether anything changed.
    pub fn set_unsaved(&mut self, settings: GraphicsSettings) -> bool {
        let settings = settings.sanitized();
        if settings == self.settings {
            return false;
        }
        self.settings = settings;
        self.revision += 1;
        true
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

/// The process's graphics settings (one app per process), those of the open project:
/// a script changing them on its first frame changes the project's settings, not ones
/// the first render would then load over its change.
pub fn graphics_settings() -> MutexGuard<'static, GraphicsSettingsState> {
    let mut state = STATE
        .get_or_init(|| {
            Mutex::new(GraphicsSettingsState {
                hotkey_enabled: true,
                manage_window: true,
                exit_button: true,
                ..GraphicsSettingsState::default()
            })
        })
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state.sync_namespace();
    state
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
        assert!(!applied.path_tracing.enabled);
        assert!(!applied.global_illumination.enabled);
        let traced = GraphicsSettings {
            global_illumination: GlobalIllumination::RayTraced,
            ..GraphicsSettings::default()
        }
        .apply(PostProcessSettings::default());
        assert!(traced.global_illumination.enabled);
        let path_traced = GraphicsSettings {
            path_tracing: true,
            path_tracing_bounces: 3,
            path_tracing_denoiser: false,
            ..GraphicsSettings::default()
        }
        .apply(PostProcessSettings::default());
        assert!(path_traced.path_tracing.enabled && !path_traced.path_tracing.denoiser);
        assert_eq!(path_traced.path_tracing.bounces, 3);
    }

    #[test]
    fn ray_tracing_off_takes_the_raster_effects() {
        let settings = GraphicsSettings {
            ray_tracing: false,
            global_illumination: GlobalIllumination::RayTraced,
            path_tracing: true,
            ..GraphicsSettings::default()
        };
        let applied = settings.apply(PostProcessSettings::default());
        assert!(!applied.rt_shadows.enabled);
        assert!(applied.ssr.enabled && !applied.ssr.ray_traced);
        assert!(applied.ssao.enabled && !applied.ssao.ray_traced);
        assert!(!applied.refraction.ray_traced);
        assert!(!applied.global_illumination.enabled && !applied.path_tracing.enabled);
        // The choices stay for when it is switched back on.
        let back = GraphicsSettings {
            ray_tracing: true,
            ..settings
        }
        .apply(PostProcessSettings::default());
        assert!(back.rt_shadows.enabled && back.ssr.ray_traced && back.path_tracing.enabled);
    }

    #[test]
    fn patches_change_only_the_named_settings() {
        let base = GraphicsSettings::default();
        let patch = serde_json::json!({ "reflections": "screen_space", "render_scale": 0.5 });
        let patched = base.patched(patch.as_object().unwrap()).unwrap();
        assert_eq!(patched.reflections, EffectQuality::ScreenSpace);
        assert_eq!(patched.render_scale, 0.5);
        assert_eq!(patched.ambient_occlusion, base.ambient_occlusion);

        let unknown = serde_json::json!({ "reflection": "off" });
        let err = base.patched(unknown.as_object().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown graphics settings: reflection"),
            "{err}"
        );
        let invalid = serde_json::json!({ "reflections": "mirror" });
        assert!(base.patched(invalid.as_object().unwrap()).is_err());
    }

    #[test]
    fn loading_repairs_bad_values() {
        let settings: GraphicsSettings = serde_json::from_str(
            r#"{"render_scale": 7, "sharpness": -1, "window_size": [10, 10], "path_tracing_bounces": 0}"#,
        )
        .unwrap();
        let settings = settings.sanitized();
        assert_eq!(settings.render_scale, 1.0);
        assert_eq!(settings.sharpness, 0.0);
        assert_eq!(settings.window_size, None);
        assert_eq!(settings.path_tracing_bounces, 1);
        // Fields missing from an older file keep their defaults.
        assert!(settings.show_fps);
        assert_eq!(settings.global_illumination, GlobalIllumination::Off);
        let settings: GraphicsSettings =
            serde_json::from_str(r#"{"global_illumination": "ray_traced"}"#).unwrap();
        assert_eq!(settings.global_illumination, GlobalIllumination::RayTraced);
    }
}
