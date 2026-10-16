use log::{info, warn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderSettings {
    #[serde(default = "RenderSettings::default_sample_count")]
    pub sample_count: u32,
    #[serde(default = "RenderSettings::default_shadow_map_size")]
    pub shadow_map_size: u32,
    #[serde(default)]
    pub resolution: Resolution,
    #[serde(default)]
    pub present_mode: PresentModeSetting,
    /// Monitor to open the window on: a 1-based index, an output name such as
    /// `HDMI-A-0`, or `primary`. `None` lets the window manager choose. The
    /// `ENGINE_MONITOR` environment variable overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    /// Output to HDR displays. The `ENGINE_HDR` environment variable overrides it.
    #[serde(default)]
    pub hdr_output: HdrOutputSetting,
    /// Brightness of white UI and of scene value 1.0 on an HDR display, in nits.
    #[serde(default = "RenderSettings::default_paper_white_nits")]
    pub hdr_paper_white_nits: f32,
    /// Brightest highlight the HDR display shows, in nits.
    #[serde(default = "RenderSettings::default_peak_nits")]
    pub hdr_peak_nits: f32,
}

/// Environment variable that overrides [`RenderSettings::hdr_output`].
pub const HDR_ENV_VAR: &str = "ENGINE_HDR";

/// Whether frames go to the display in HDR.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrOutputSetting {
    /// Standard dynamic range (the image is tonemapped to 0-1).
    #[default]
    Off,
    /// HDR when the display offers it (scRGB, else HDR10), otherwise SDR.
    Auto,
    /// The HDR output path on an SDR display, with highlights compressed to fit: for
    /// trying HDR settings and testing without an HDR screen.
    Simulate,
}

impl HdrOutputSetting {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "false" | "sdr" => Some(Self::Off),
            "auto" | "on" | "1" | "true" | "hdr" => Some(Self::Auto),
            "simulate" | "sim" => Some(Self::Simulate),
            _ => None,
        }
    }
}

/// Environment variable that overrides [`RenderSettings::monitor`].
pub const MONITOR_ENV_VAR: &str = "ENGINE_MONITOR";

/// Environment variable that overrides [`RenderSettings::resolution`] (the initial window
/// size) as `WIDTHxHEIGHT`, e.g. `ENGINE_WINDOW_SIZE=800x450`.
pub const WINDOW_SIZE_ENV_VAR: &str = "ENGINE_WINDOW_SIZE";

/// Parse a `WIDTHxHEIGHT` (or `WIDTH,HEIGHT`) window size.
pub fn parse_window_size(text: &str) -> Option<Resolution> {
    let text = text.trim();
    let (width, height) = text
        .split_once(['x', 'X', ','])
        .map(|(width, height)| (width.trim(), height.trim()))
        .or_else(|| {
            let mut parts = text.split_whitespace();
            Some((parts.next()?, parts.next()?))
        })?;
    let (width, height) = (width.parse::<u32>().ok()?, height.parse::<u32>().ok()?);
    ((16..=16384).contains(&width) && (16..=16384).contains(&height))
        .then_some(Resolution { width, height })
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            sample_count: Self::default_sample_count(),
            shadow_map_size: Self::default_shadow_map_size(),
            resolution: Resolution::default(),
            present_mode: PresentModeSetting::default(),
            monitor: None,
            hdr_output: HdrOutputSetting::default(),
            hdr_paper_white_nits: Self::default_paper_white_nits(),
            hdr_peak_nits: Self::default_peak_nits(),
        }
    }
}

impl RenderSettings {
    pub fn load() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            info!("Using default render settings for WebAssembly build");
            return Self::default();
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut settings = Self::load_from_path("settings.json");
            if let Ok(monitor) = std::env::var(MONITOR_ENV_VAR) {
                let monitor = monitor.trim();
                if !monitor.is_empty() {
                    settings.monitor = Some(monitor.to_owned());
                }
            }
            if let Ok(value) = std::env::var(HDR_ENV_VAR) {
                match HdrOutputSetting::parse(&value) {
                    Some(mode) => settings.hdr_output = mode,
                    None => warn!("Ignoring {HDR_ENV_VAR}={value:?}; use off, auto or simulate"),
                }
            }
            if let Ok(value) = std::env::var(WINDOW_SIZE_ENV_VAR) {
                match parse_window_size(&value) {
                    Some(resolution) => settings.resolution = resolution,
                    None => warn!(
                        "Ignoring {WINDOW_SIZE_ENV_VAR}={value:?}; use WIDTHxHEIGHT such as 800x450"
                    ),
                }
            }
            settings
        }
    }

    /// Index into `monitors` (output names in enumeration order) that
    /// [`Self::monitor`] selects. `primary` is the index of the primary monitor.
    pub fn select_monitor(&self, monitors: &[String], primary: Option<usize>) -> Option<usize> {
        let wanted = self.monitor.as_deref()?.trim();
        if wanted.eq_ignore_ascii_case("primary") {
            return primary;
        }
        if let Ok(number) = wanted.parse::<usize>() {
            return (1..=monitors.len()).contains(&number).then(|| number - 1);
        }
        monitors.iter().position(|name| name == wanted).or_else(|| {
            monitors
                .iter()
                .position(|name| name.eq_ignore_ascii_case(wanted))
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_from_path<P: AsRef<std::path::Path>>(path: P) -> Self {
        use std::fs;

        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<RenderSettings>(&contents) {
                Ok(settings) => {
                    info!("Loaded render settings from {:?}", path);
                    settings.validate()
                }
                Err(err) => {
                    warn!(
                        "Failed to parse {:?} ({}). Falling back to default render settings.",
                        path, err
                    );
                    RenderSettings::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                info!(
                    "Render settings file {:?} not found. Using default settings.",
                    path
                );
                RenderSettings::default()
            }
            Err(err) => {
                warn!(
                    "Failed to read {:?} ({}). Falling back to default render settings.",
                    path, err
                );
                RenderSettings::default()
            }
        }
    }

    fn validate(mut self) -> Self {
        if self.sample_count == 0 {
            warn!("Sample count must be greater than zero. Using 1 instead.");
            self.sample_count = Self::default_sample_count();
        }

        if self.shadow_map_size == 0 {
            warn!("Shadow map size must be greater than zero. Using default value.");
            self.shadow_map_size = Self::default_shadow_map_size();
        }

        if !(self.hdr_paper_white_nits.is_finite() && self.hdr_paper_white_nits >= 1.0) {
            self.hdr_paper_white_nits = Self::default_paper_white_nits();
        }
        if !(self.hdr_peak_nits.is_finite() && self.hdr_peak_nits >= self.hdr_paper_white_nits) {
            self.hdr_peak_nits = Self::default_peak_nits().max(self.hdr_paper_white_nits);
        }

        if self.resolution.width == 0 || self.resolution.height == 0 {
            warn!("Resolution must be greater than zero. Using default resolution.");
            self.resolution = Resolution::default();
        }

        self
    }

    pub fn present_mode(&self, available: &[wgpu::PresentMode]) -> wgpu::PresentMode {
        let desired = self.present_mode.to_wgpu();
        if available.contains(&desired) {
            return desired;
        }

        warn!(
            "Requested present mode {:?} is not supported. Falling back to FIFO.",
            desired
        );

        if available.contains(&wgpu::PresentMode::Fifo) {
            wgpu::PresentMode::Fifo
        } else {
            available
                .first()
                .copied()
                .unwrap_or(wgpu::PresentMode::Fifo)
        }
    }

    const fn default_sample_count() -> u32 {
        1
    }

    const fn default_shadow_map_size() -> u32 {
        2048
    }

    const fn default_paper_white_nits() -> f32 {
        200.0
    }

    const fn default_peak_nits() -> f32 {
        1000.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Default for Resolution {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PresentModeSetting {
    Fifo,
    FifoRelaxed,
    Immediate,
    #[default]
    Mailbox,
    AutoVsync,
    AutoNoVsync,
}

impl PresentModeSetting {
    fn to_wgpu(&self) -> wgpu::PresentMode {
        match self {
            PresentModeSetting::Fifo => wgpu::PresentMode::Fifo,
            PresentModeSetting::FifoRelaxed => wgpu::PresentMode::FifoRelaxed,
            PresentModeSetting::Immediate => wgpu::PresentMode::Immediate,
            PresentModeSetting::Mailbox => wgpu::PresentMode::Mailbox,
            PresentModeSetting::AutoVsync => wgpu::PresentMode::AutoVsync,
            PresentModeSetting::AutoNoVsync => wgpu::PresentMode::AutoNoVsync,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_size_parses_width_by_height() {
        let size = super::parse_window_size("800x450").unwrap();
        assert_eq!((size.width, size.height), (800, 450));
        let size = super::parse_window_size(" 1280 , 720 ").unwrap();
        assert_eq!((size.width, size.height), (1280, 720));
        assert!(super::parse_window_size("800").is_none());
        assert!(super::parse_window_size("0x10").is_none());
    }

    use super::*;

    fn invalid_settings() -> RenderSettings {
        RenderSettings {
            sample_count: 0,
            shadow_map_size: 0,
            resolution: Resolution {
                width: 0,
                height: 0,
            },
            present_mode: PresentModeSetting::Immediate,
            monitor: None,
            ..RenderSettings::default()
        }
    }

    #[test]
    fn validate_replaces_invalid_values_with_defaults() {
        let validated = invalid_settings().validate();

        assert_eq!(
            validated.sample_count,
            RenderSettings::default().sample_count
        );
        assert_eq!(
            validated.shadow_map_size,
            RenderSettings::default().shadow_map_size
        );
        assert_eq!(validated.resolution.width, Resolution::default().width);
        assert_eq!(validated.resolution.height, Resolution::default().height);
    }

    #[test]
    fn validate_preserves_valid_values() {
        let valid = RenderSettings {
            sample_count: 4,
            shadow_map_size: 2048,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            present_mode: PresentModeSetting::Mailbox,
            monitor: None,
            ..RenderSettings::default()
        };

        let validated = valid.clone().validate();

        assert_eq!(validated.sample_count, valid.sample_count);
        assert_eq!(validated.shadow_map_size, valid.shadow_map_size);
        assert_eq!(validated.resolution.width, valid.resolution.width);
        assert_eq!(validated.resolution.height, valid.resolution.height);
    }

    #[test]
    fn select_monitor_by_number_name_or_primary() {
        let monitors = vec!["DisplayPort-0".to_string(), "HDMI-A-0".to_string()];
        let with = |monitor: &str| RenderSettings {
            monitor: Some(monitor.to_string()),
            ..RenderSettings::default()
        };

        assert_eq!(
            RenderSettings::default().select_monitor(&monitors, Some(0)),
            None
        );
        assert_eq!(with("2").select_monitor(&monitors, Some(0)), Some(1));
        assert_eq!(with("3").select_monitor(&monitors, Some(0)), None);
        assert_eq!(with("0").select_monitor(&monitors, Some(0)), None);
        assert_eq!(with("hdmi-a-0").select_monitor(&monitors, Some(0)), Some(1));
        assert_eq!(with("primary").select_monitor(&monitors, Some(0)), Some(0));
        assert_eq!(with("VGA-1").select_monitor(&monitors, Some(0)), None);
    }

    #[test]
    fn monitor_is_optional_in_settings_files() {
        let settings: RenderSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.monitor, None);
        let settings: RenderSettings = serde_json::from_str(r#"{"monitor": "2"}"#).unwrap();
        assert_eq!(settings.monitor.as_deref(), Some("2"));
    }

    #[test]
    fn hdr_output_parses_and_defaults_off() {
        let settings: RenderSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.hdr_output, HdrOutputSetting::Off);
        let settings: RenderSettings =
            serde_json::from_str(r#"{"hdr_output": "auto", "hdr_peak_nits": 600}"#).unwrap();
        assert_eq!(settings.hdr_output, HdrOutputSetting::Auto);
        assert_eq!(settings.hdr_peak_nits, 600.0);
        assert_eq!(
            HdrOutputSetting::parse("Simulate"),
            Some(HdrOutputSetting::Simulate)
        );
        assert_eq!(HdrOutputSetting::parse("bogus"), None);
        let fixed = RenderSettings {
            hdr_paper_white_nits: 400.0,
            hdr_peak_nits: 100.0,
            ..RenderSettings::default()
        }
        .validate();
        assert!(fixed.hdr_peak_nits >= fixed.hdr_paper_white_nits);
    }

    #[test]
    fn present_mode_returns_desired_when_available() {
        let settings = RenderSettings {
            present_mode: PresentModeSetting::Mailbox,
            ..RenderSettings::default()
        };

        let available = [
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Immediate,
        ];

        assert_eq!(
            settings.present_mode(&available),
            wgpu::PresentMode::Mailbox
        );
    }

    #[test]
    fn present_mode_falls_back_to_fifo_when_desired_missing() {
        let settings = RenderSettings {
            present_mode: PresentModeSetting::Mailbox,
            ..RenderSettings::default()
        };

        let available = [wgpu::PresentMode::Fifo, wgpu::PresentMode::Immediate];

        assert_eq!(settings.present_mode(&available), wgpu::PresentMode::Fifo);
    }

    #[test]
    fn present_mode_uses_first_available_when_fifo_missing() {
        let settings = RenderSettings {
            present_mode: PresentModeSetting::Mailbox,
            ..RenderSettings::default()
        };

        let available = [wgpu::PresentMode::Immediate];

        assert_eq!(
            settings.present_mode(&available),
            wgpu::PresentMode::Immediate
        );
    }
}
