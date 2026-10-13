//! Viewport preview: lay the game out in a smaller view of another shape (16:9, 4:3, a
//! phone screen) inside the real one, so a remote client can check framing at window
//! shapes it cannot set. Unlike a cropped screenshot, everything sees the new shape:
//! the renderer draws into it, scripts get it from `get_viewport_size()` (so cameras
//! fitted to the view adapt) and script HUDs anchor to it.

use std::sync::{Mutex, PoisonError};

use serde_json::{json, Value};

use crate::renderer::RenderRegion;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewShape {
    /// The largest region of this width / height that fits the view.
    Aspect(f32),
    /// Exactly this many pixels, centred (shrunk to fit if the view is smaller).
    Size(u32, u32),
}

static PREVIEW: Mutex<Option<PreviewShape>> = Mutex::new(None);

pub fn preview_shape() -> Option<PreviewShape> {
    *PREVIEW.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn set_preview_shape(shape: Option<PreviewShape>) {
    *PREVIEW.lock().unwrap_or_else(PoisonError::into_inner) = shape;
}

/// `(x, y, width, height)` of the preview inside a view of the given size, or the view
/// itself without a preview. `pixels_per_unit` converts a pixel `size` preview to the
/// view's units (egui points use the display scale; renderer pixels use 1).
pub fn preview_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    pixels_per_unit: f32,
) -> (f32, f32, f32, f32) {
    let (w, h) = match preview_shape() {
        None => return (x, y, width, height),
        Some(PreviewShape::Aspect(aspect)) => {
            if aspect >= width / height {
                (width, width / aspect)
            } else {
                (height * aspect, height)
            }
        }
        Some(PreviewShape::Size(w, h)) => {
            let (w, h) = (w as f32 / pixels_per_unit, h as f32 / pixels_per_unit);
            // Shrink to fit, keeping the shape.
            let scale = (width / w).min(height / h).min(1.0);
            (w * scale, h * scale)
        }
    };
    let (w, h) = (w.floor().max(1.0), h.floor().max(1.0));
    (
        x + ((width - w) / 2.0).floor(),
        y + ((height - h) / 2.0).floor(),
        w,
        h,
    )
}

/// The region to render into, inside `base`.
pub fn preview_region(base: RenderRegion) -> Option<RenderRegion> {
    preview_shape()?;
    let (x, y, width, height) = preview_rect(
        base.x() as f32,
        base.y() as f32,
        base.width() as f32,
        base.height() as f32,
        1.0,
    );
    RenderRegion::new(x as u32, y as u32, width as u32, height as u32)
}

/// Remote method `view.preview`: `{aspect}` (1.78 or "16:9"), `{size: [w, h]}`, or
/// neither to go back to the full view.
pub(crate) fn set_from_params(params: &Value) -> Result<Value, String> {
    let shape = if let Some(size) = params.get("size").filter(|value| !value.is_null()) {
        let pair = size
            .as_array()
            .filter(|pair| pair.len() == 2)
            .and_then(|pair| Some((pair[0].as_u64()?, pair[1].as_u64()?)))
            .filter(|(w, h)| (16..=16384).contains(w) && (16..=16384).contains(h))
            .ok_or("size must be [width, height] in pixels (16..16384)")?;
        Some(PreviewShape::Size(pair.0 as u32, pair.1 as u32))
    } else if let Some(aspect) = params.get("aspect").filter(|value| !value.is_null()) {
        Some(PreviewShape::Aspect(super::hub::parse_aspect(aspect)?))
    } else {
        None
    };
    set_preview_shape(shape);
    Ok(json!({
        "preview": match shape {
            None => json!(null),
            Some(PreviewShape::Aspect(aspect)) => json!({ "aspect": aspect }),
            Some(PreviewShape::Size(w, h)) => json!({ "size": [w, h] }),
        },
        "note": "Applies from the next frame: the scene renders into that shape inside the view, scripts see it as get_viewport_size() and HUDs anchor to it. Call again without aspect or size to restore the full view.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_fit_inside_the_view() {
        set_preview_shape(Some(PreviewShape::Aspect(4.0 / 3.0)));
        assert_eq!(
            preview_rect(10.0, 20.0, 1600.0, 900.0, 1.0),
            (210.0, 20.0, 1200.0, 900.0)
        );
        set_preview_shape(Some(PreviewShape::Size(400, 300)));
        assert_eq!(
            preview_rect(0.0, 0.0, 1000.0, 1000.0, 1.0),
            (300.0, 350.0, 400.0, 300.0)
        );
        // Larger than the view: shrunk, same shape.
        assert_eq!(
            preview_rect(0.0, 0.0, 200.0, 300.0, 1.0),
            (0.0, 75.0, 200.0, 150.0)
        );
        // On a 2x display the same pixels are half as many points.
        assert_eq!(
            preview_rect(0.0, 0.0, 1000.0, 1000.0, 2.0),
            (400.0, 425.0, 200.0, 150.0)
        );
        set_preview_shape(None);
        assert_eq!(preview_rect(1.0, 2.0, 3.0, 4.0, 1.0), (1.0, 2.0, 3.0, 4.0));
    }
}
