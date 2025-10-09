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
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            sample_count: Self::default_sample_count(),
            shadow_map_size: Self::default_shadow_map_size(),
            resolution: Resolution::default(),
            present_mode: PresentModeSetting::default(),
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
            Self::load_from_path("settings.json")
        }
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
        8
    }

    const fn default_shadow_map_size() -> u32 {
        4096
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
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentModeSetting {
    Fifo,
    FifoRelaxed,
    Immediate,
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

impl Default for PresentModeSetting {
    fn default() -> Self {
        PresentModeSetting::Fifo
    }
}

#[cfg(test)]
mod tests {
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
        };

        let validated = valid.clone().validate();

        assert_eq!(validated.sample_count, valid.sample_count);
        assert_eq!(validated.shadow_map_size, valid.shadow_map_size);
        assert_eq!(validated.resolution.width, valid.resolution.width);
        assert_eq!(validated.resolution.height, valid.resolution.height);
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
