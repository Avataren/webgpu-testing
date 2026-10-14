//! 3D colour lookup tables in the Adobe/Resolve `.cube` format, applied to the display
//! image as the last colour-correction step.

use half::f16;
use std::path::{Path, PathBuf};

/// A parsed `.cube` 3D LUT: `size`³ RGB entries, red varying fastest.
#[derive(Debug, Clone, PartialEq)]
pub struct CubeLut {
    pub size: u32,
    pub data: Vec<[f32; 3]>,
}

/// Largest LUT the engine loads (65³ is the biggest size grading tools export).
const MAX_SIZE: u32 = 128;

impl CubeLut {
    /// Parse the text of a `.cube` file.
    ///
    /// # Examples
    ///
    /// ```
    /// use wgpu_cube::renderer::postprocess::CubeLut;
    ///
    /// let text = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    /// let lut = CubeLut::parse(text).unwrap();
    /// assert_eq!(lut.size, 2);
    /// assert_eq!(lut.data[1], [1.0, 0.0, 0.0]);
    /// ```
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut size = None;
        let mut data = Vec::new();
        for (number, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or("");
            match first {
                "TITLE" | "LUT_IN_VIDEO_RANGE" | "LUT_OUT_VIDEO_RANGE" => {}
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    let expected = if first == "DOMAIN_MIN" { 0.0 } else { 1.0 };
                    let custom = words
                        .filter_map(|word| word.parse::<f32>().ok())
                        .any(|value| (value - expected).abs() > 1e-6);
                    if custom {
                        return Err(format!(
                            "line {}: only the default 0-1 domain is supported",
                            number + 1
                        ));
                    }
                }
                "LUT_1D_SIZE" => {
                    return Err("1D LUTs are not supported; export a 3D .cube".into());
                }
                "LUT_3D_SIZE" => {
                    let value: u32 = words
                        .next()
                        .and_then(|word| word.parse().ok())
                        .ok_or_else(|| format!("line {}: bad LUT_3D_SIZE", number + 1))?;
                    if !(2..=MAX_SIZE).contains(&value) {
                        return Err(format!("LUT_3D_SIZE {value} is outside 2..={MAX_SIZE}"));
                    }
                    size = Some(value);
                    data.reserve((value * value * value) as usize);
                }
                _ if first.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') => {
                    let values: Vec<f32> = line
                        .split_whitespace()
                        .map(str::parse)
                        .collect::<Result<_, _>>()
                        .map_err(|_| format!("line {}: expected three numbers", number + 1))?;
                    let [r, g, b] = values[..] else {
                        return Err(format!("line {}: expected three numbers", number + 1));
                    };
                    data.push([r, g, b]);
                }
                // Unknown keywords (vendor extensions) are skipped, as other tools do.
                _ => {}
            }
        }
        let size = size.ok_or("missing LUT_3D_SIZE")?;
        let expected = (size * size * size) as usize;
        if data.len() != expected {
            return Err(format!(
                "LUT_3D_SIZE {size} needs {expected} entries, found {}",
                data.len()
            ));
        }
        Ok(Self { size, data })
    }

    /// A LUT that leaves colours unchanged.
    pub fn identity(size: u32) -> Self {
        let size = size.max(2);
        let scale = 1.0 / (size - 1) as f32;
        let mut data = Vec::with_capacity((size * size * size) as usize);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push([r as f32 * scale, g as f32 * scale, b as f32 * scale]);
                }
            }
        }
        Self { size, data }
    }

    fn to_rgba16f(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.data.len() * 8);
        for [r, g, b] in &self.data {
            for value in [*r, *g, *b, 1.0] {
                bytes.extend_from_slice(&f16::from_f32(value).to_le_bytes());
            }
        }
        bytes
    }
}

/// The LUT texture the composite pass samples, reloaded when the settings name another
/// file. Without a usable file it holds an identity LUT.
pub struct LutTexture {
    // CRITICAL: view must drop before texture
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
    /// The settings path the texture was loaded for ("" for none).
    source: String,
    /// Whether `source` loaded; a failed path is not retried every frame.
    loaded: bool,
}

impl LutTexture {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let (texture, view) = upload(device, queue, &CubeLut::identity(2));
        Self {
            view,
            _texture: texture,
            source: String::new(),
            loaded: false,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Whether a LUT from a file is in place.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Load `path` (resolved against the project) if it differs from what is loaded.
    /// Returns true when the texture was replaced, so bind groups must be rebuilt.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, path: &str) -> bool {
        if path == self.source {
            return false;
        }
        self.source = path.to_string();
        let lut = if path.is_empty() {
            None
        } else {
            match load(&crate::project::resolve_project_path(path)) {
                Ok(lut) => Some(lut),
                Err(err) => {
                    log::warn!("Colour LUT {path}: {err}");
                    None
                }
            }
        };
        self.loaded = lut.is_some();
        let lut = lut.unwrap_or_else(|| CubeLut::identity(2));
        let (texture, view) = upload(device, queue, &lut);
        self._texture = texture;
        self.view = view;
        true
    }
}

fn load(path: &Path) -> Result<CubeLut, String> {
    let text = crate::io::read_to_string(&PathBuf::from(path)).map_err(|err| err.to_string())?;
    CubeLut::parse(&text)
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    lut: &CubeLut,
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: lut.size,
        height: lut.size,
        depth_or_array_layers: lut.size,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ColorLut"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &lut.to_rgba16f(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(lut.size * 8),
            rows_per_image: Some(lut.size),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headers_comments_and_entries() {
        let text = "# graded in resolve\nTITLE \"Warm\"\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
                    LUT_3D_SIZE 2\n\n0 0 0\n1 0 0 # red\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n0.9 0.9 0.9\n";
        let lut = CubeLut::parse(text).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8);
        assert_eq!(lut.data[7], [0.9, 0.9, 0.9]);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(CubeLut::parse("0 0 0\n").is_err());
        assert!(CubeLut::parse("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(CubeLut::parse("LUT_1D_SIZE 16\n").is_err());
        assert!(CubeLut::parse("LUT_3D_SIZE 2\nDOMAIN_MAX 2 2 2\n").is_err());
        assert!(CubeLut::parse("LUT_3D_SIZE 2\n0 0\n").is_err());
    }

    #[test]
    fn identity_maps_grid_points_to_themselves() {
        let lut = CubeLut::identity(3);
        assert_eq!(lut.data.len(), 27);
        // Index = r + g * size + b * size * size.
        assert_eq!(lut.data[1 + 2 * 3 + 9], [0.5, 1.0, 0.5]);
        assert_eq!(lut.to_rgba16f().len(), 27 * 8);
    }
}
