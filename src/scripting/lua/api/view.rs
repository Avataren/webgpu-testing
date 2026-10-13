//! # View API
//!
//! Conversions between the game view and the world, in the same window-pixel
//! coordinates `get_mouse_position()` reports, so a script can aim at the mouse:
//!
//! - `get_viewport_size()` - `{x, y, width, height}` of the game view in window pixels
//!   (in the editor the view is a part of the window)
//! - `world_to_screen(x, y, z)` - `{x, y, on_screen}` in window pixels, or `nil` when the
//!   point is behind the camera
//! - `screen_to_world_ray(x, y)` - `{ox, oy, oz, dx, dy, dz}`: where the ray through that
//!   pixel starts and its unit direction
//! - `screen_to_ground(x, y[, height])` - `{x, y, z}` where that ray meets the horizontal
//!   plane `y = height` (default 0), or `nil` when it never does
//!
//! All of them use the camera and view of the previous rendered frame, and return `nil`
//! before the first frame has been drawn.

use glam::{Mat4, Vec3, Vec4};
use mlua::{Lua, Result as LuaResult};

/// The camera and screen area scripts convert through, stored as Lua app data by the
/// scripting state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScriptView {
    /// View-projection matrix of the camera, for the aspect ratio of `rect`.
    pub view_proj: Mat4,
    /// Game view in window pixels: x, y, width, height.
    pub rect: [f32; 4],
}

impl ScriptView {
    /// Window pixel to normalized device coordinates (`x` right, `y` up, both -1..1).
    fn ndc_of(&self, x: f32, y: f32) -> (f32, f32) {
        let [left, top, width, height] = self.rect;
        (
            (x - left) / width * 2.0 - 1.0,
            1.0 - (y - top) / height * 2.0,
        )
    }

    /// Where `point` lands in window pixels, and whether that is inside the view.
    /// `None` if the point is at or behind the camera plane.
    pub fn world_to_screen(&self, point: Vec3) -> Option<(f32, f32, bool)> {
        let clip = self.view_proj * Vec4::new(point.x, point.y, point.z, 1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        let [left, top, width, height] = self.rect;
        let on_screen = (-1.0..=1.0).contains(&ndc.x)
            && (-1.0..=1.0).contains(&ndc.y)
            && (0.0..=1.0).contains(&ndc.z);
        Some((
            left + (ndc.x + 1.0) * 0.5 * width,
            top + (1.0 - ndc.y) * 0.5 * height,
            on_screen,
        ))
    }

    /// The world-space ray through a window pixel: origin on the near plane, unit direction.
    pub fn ray(&self, x: f32, y: f32) -> Option<(Vec3, Vec3)> {
        let inverse = self.view_proj.inverse();
        if !inverse.is_finite() {
            return None;
        }
        let (ndc_x, ndc_y) = self.ndc_of(x, y);
        let unproject = |depth: f32| {
            let world = inverse * Vec4::new(ndc_x, ndc_y, depth, 1.0);
            world.truncate() / world.w
        };
        let near = unproject(0.0);
        let far = unproject(1.0);
        let direction = (far - near).try_normalize()?;
        Some((near, direction))
    }

    /// Where the ray through a window pixel meets the plane `y = height`.
    pub fn ground_point(&self, x: f32, y: f32, height: f32) -> Option<Vec3> {
        let (origin, direction) = self.ray(x, y)?;
        if direction.y.abs() < 1e-6 {
            return None;
        }
        let distance = (height - origin.y) / direction.y;
        (distance >= 0.0).then(|| origin + direction * distance)
    }
}

fn with_view<T>(lua: &Lua, read: impl FnOnce(&ScriptView) -> T) -> Option<T> {
    lua.app_data_ref::<ScriptView>().map(|view| read(&view))
}

pub(crate) fn register_view_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // get_viewport_size() -> {x, y, width, height} | nil
    globals.set(
        "get_viewport_size",
        lua.create_function(|lua, ()| {
            let Some([x, y, width, height]) = with_view(lua, |view| view.rect) else {
                return Ok(None);
            };
            let table = lua.create_table()?;
            table.set("x", x as f64)?;
            table.set("y", y as f64)?;
            table.set("width", width as f64)?;
            table.set("height", height as f64)?;
            Ok(Some(table))
        })?,
    )?;

    // world_to_screen(x, y, z) -> {x, y, on_screen} | nil
    globals.set(
        "world_to_screen",
        lua.create_function(|lua, (x, y, z): (f64, f64, f64)| {
            let point = Vec3::new(x as f32, y as f32, z as f32);
            let Some(Some((sx, sy, on_screen))) =
                with_view(lua, |view| view.world_to_screen(point))
            else {
                return Ok(None);
            };
            let table = lua.create_table()?;
            table.set("x", sx as f64)?;
            table.set("y", sy as f64)?;
            table.set("on_screen", on_screen)?;
            Ok(Some(table))
        })?,
    )?;

    // screen_to_world_ray(x, y) -> {ox, oy, oz, dx, dy, dz} | nil
    globals.set(
        "screen_to_world_ray",
        lua.create_function(|lua, (x, y): (f64, f64)| {
            let Some(Some((origin, direction))) =
                with_view(lua, |view| view.ray(x as f32, y as f32))
            else {
                return Ok(None);
            };
            let table = lua.create_table()?;
            for (key, value) in [
                ("ox", origin.x),
                ("oy", origin.y),
                ("oz", origin.z),
                ("dx", direction.x),
                ("dy", direction.y),
                ("dz", direction.z),
            ] {
                table.set(key, value as f64)?;
            }
            Ok(Some(table))
        })?,
    )?;

    // screen_to_ground(x, y[, height]) -> {x, y, z} | nil
    globals.set(
        "screen_to_ground",
        lua.create_function(|lua, (x, y, height): (f64, f64, Option<f64>)| {
            let height = height.unwrap_or(0.0) as f32;
            let Some(Some(point)) =
                with_view(lua, |view| view.ground_point(x as f32, y as f32, height))
            else {
                return Ok(None);
            };
            let table = lua.create_table()?;
            table.set("x", point.x as f64)?;
            table.set("y", point.y as f64)?;
            table.set("z", point.z as f64)?;
            Ok(Some(table))
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Camera;

    /// A camera 10 m above the origin looking down, in a view that starts at (100, 50).
    fn top_down_view() -> ScriptView {
        let mut camera = Camera::default();
        camera.eye = Vec3::new(0.0, 10.0, 0.0);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::NEG_Z;
        let rect = [100.0, 50.0, 800.0, 600.0];
        ScriptView {
            view_proj: camera.view_proj(rect[2] / rect[3]),
            rect,
        }
    }

    #[test]
    fn the_view_centre_maps_to_the_point_under_the_camera() {
        let view = top_down_view();
        let ground = view.ground_point(500.0, 350.0, 0.0).unwrap();
        assert!(ground.x.abs() < 1e-3 && ground.z.abs() < 1e-3, "{ground:?}");
        let (x, y, on_screen) = view.world_to_screen(Vec3::ZERO).unwrap();
        assert!((x - 500.0).abs() < 1e-2 && (y - 350.0).abs() < 1e-2);
        assert!(on_screen);
    }

    #[test]
    fn ground_points_round_trip_through_the_screen() {
        let view = top_down_view();
        for target in [Vec3::new(2.0, 0.0, 1.5), Vec3::new(-3.0, 0.0, -2.0)] {
            let (x, y, _) = view.world_to_screen(target).unwrap();
            let back = view.ground_point(x, y, 0.0).unwrap();
            assert!((back - target).length() < 1e-2, "{target:?} -> {back:?}");
        }
        // The right of the view is +X, and the top of the view is -Z for this camera.
        let right = view.ground_point(700.0, 350.0, 0.0).unwrap();
        let top = view.ground_point(500.0, 100.0, 0.0).unwrap();
        assert!(right.x > 0.5);
        assert!(top.z < -0.5);
    }

    #[test]
    fn points_behind_the_camera_and_parallel_rays_have_no_answer() {
        let view = top_down_view();
        assert!(view.world_to_screen(Vec3::new(0.0, 20.0, 0.0)).is_none());
        // A plane above the camera is never hit by rays that point down.
        assert!(view.ground_point(500.0, 350.0, 20.0).is_none());
        let (_, _, on_screen) = view.world_to_screen(Vec3::new(500.0, 0.0, 0.0)).unwrap();
        assert!(!on_screen);
    }
}
