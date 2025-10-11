use std::path::{Path, PathBuf};

use wgpu::Color;

/// Describes high-level environment settings applied while rendering a scene.
///
/// The environment controls global rendering parameters such as the clear
/// color, sky rendering, and image-based lighting. In addition to the clear
/// color, the environment can optionally reference an HDR background that will
/// be used for image-based lighting when enabled.
#[derive(Debug, Clone)]
pub struct Environment {
    clear_color: Color,
    ambient_intensity: f32,
    hdr_background: Option<HdrBackground>,
    color_grading: ColorGrading,
}

/// Global color adjustments applied during the post-processing pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorGrading {
    exposure: f32,
    saturation: f32,
    contrast: f32,
}

#[derive(Debug, Clone)]
pub struct HdrBackground {
    enabled: bool,
    image_path: PathBuf,
    intensity: f32,
}

impl Environment {
    /// Creates a new environment with the provided clear color.
    pub fn new(clear_color: Color) -> Self {
        Self {
            clear_color,
            ambient_intensity: 0.03,
            hdr_background: None,
            color_grading: ColorGrading::default(),
        }
    }

    /// Returns the clear color that should be used when starting a frame.
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Sets the clear color used for rendering.
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Returns a copy of the environment with the given clear color.
    pub fn with_clear_color(mut self, color: Color) -> Self {
        self.clear_color = color;
        self
    }

    /// Returns the ambient intensity multiplier used when no HDR background is active.
    pub fn ambient_intensity(&self) -> f32 {
        self.ambient_intensity
    }

    /// Sets the ambient intensity multiplier used when no HDR background is active.
    pub fn set_ambient_intensity(&mut self, intensity: f32) {
        self.ambient_intensity = intensity.max(0.0);
    }

    /// Returns a copy of the environment with the provided ambient intensity.
    pub fn with_ambient_intensity(mut self, intensity: f32) -> Self {
        self.set_ambient_intensity(intensity);
        self
    }

    /// Enables the HDR background using the provided image path. If a background already
    /// exists it is updated to use the new path and enabled.
    pub fn enable_hdr_background<P>(&mut self, image_path: P)
    where
        P: Into<PathBuf>,
    {
        let path = image_path.into();
        match self.hdr_background.as_mut() {
            Some(background) => {
                background.set_path(path);
                background.set_enabled(true);
            }
            None => {
                self.hdr_background = Some(HdrBackground::new(path));
            }
        }
    }

    /// Disables the HDR background while keeping the stored configuration intact.
    pub fn disable_hdr_background(&mut self) {
        if let Some(background) = self.hdr_background.as_mut() {
            background.set_enabled(false);
        }
    }

    /// Sets the HDR background configuration directly.
    pub fn set_hdr_background(&mut self, background: Option<HdrBackground>) {
        self.hdr_background = background;
    }

    /// Retrieves the HDR background configuration, regardless of enabled state.
    pub fn hdr_background(&self) -> Option<&HdrBackground> {
        self.hdr_background.as_ref()
    }

    /// Retrieves a mutable reference to the HDR background configuration, regardless of enabled state.
    pub fn hdr_background_mut(&mut self) -> Option<&mut HdrBackground> {
        self.hdr_background.as_mut()
    }

    /// Returns the active HDR background if enabled.
    pub fn active_hdr_background(&self) -> Option<&HdrBackground> {
        self.hdr_background
            .as_ref()
            .filter(|background| background.enabled())
    }

    /// Returns true if an HDR background exists and is enabled.
    pub fn is_hdr_enabled(&self) -> bool {
        self.active_hdr_background().is_some()
    }

    /// Returns the color grading configuration used during post-processing.
    pub fn color_grading(&self) -> ColorGrading {
        self.color_grading
    }

    /// Provides mutable access to the color grading configuration.
    pub fn color_grading_mut(&mut self) -> &mut ColorGrading {
        &mut self.color_grading
    }

    /// Sets the color grading configuration used during post-processing.
    pub fn set_color_grading(&mut self, grading: ColorGrading) {
        self.color_grading = grading;
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new(Color {
            r: 0.231,
            g: 0.269,
            b: 0.338,
            a: 1.0,
        })
    }
}

impl ColorGrading {
    /// Creates a new color grading configuration.
    pub fn new(exposure: f32, saturation: f32, contrast: f32) -> Self {
        let mut grading = Self::default();
        grading.set_exposure(exposure);
        grading.set_saturation(saturation);
        grading.set_contrast(contrast);
        grading
    }

    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    pub fn set_exposure(&mut self, exposure: f32) {
        self.exposure = exposure.max(0.0);
    }

    pub fn saturation(&self) -> f32 {
        self.saturation
    }

    pub fn set_saturation(&mut self, saturation: f32) {
        self.saturation = saturation.max(0.0);
    }

    pub fn contrast(&self) -> f32 {
        self.contrast
    }

    pub fn set_contrast(&mut self, contrast: f32) {
        self.contrast = contrast.max(0.0);
    }
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            saturation: 1.0,
            contrast: 1.0,
        }
    }
}

impl HdrBackground {
    pub fn new<P>(image_path: P) -> Self
    where
        P: Into<PathBuf>,
    {
        Self {
            enabled: true,
            image_path: image_path.into(),
            intensity: 1.0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn path(&self) -> &Path {
        &self.image_path
    }

    pub fn set_path<P>(&mut self, path: P)
    where
        P: Into<PathBuf>,
    {
        self.image_path = path.into();
    }

    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.max(0.0);
    }

    pub fn with_intensity(mut self, intensity: f32) -> Self {
        self.set_intensity(intensity);
        self
    }
}
