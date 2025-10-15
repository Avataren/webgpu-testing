// src/renderer/render_region.rs

/// Describes the rectangular region (in physical pixels) that the renderer
/// should target when drawing into the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl RenderRegion {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }

        Some(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub fn full(width: u32, height: u32) -> Option<Self> {
        Self::new(0, 0, width, height)
    }

    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn clamp(self, max_width: u32, max_height: u32) -> Option<Self> {
        let x = self.x.min(max_width);
        let y = self.y.min(max_height);

        let available_width = max_width.saturating_sub(x);
        let available_height = max_height.saturating_sub(y);

        let width = self.width.min(available_width);
        let height = self.height.min(available_height);

        if width == 0 || height == 0 {
            return None;
        }

        Some(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub fn apply_to_pass<'a>(&self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_viewport(
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(self.x, self.y, self.width, self.height);
    }
}
