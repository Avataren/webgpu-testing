//! Tests for screen-space depth/position math used by SSAO and z-buffer debug.
//!
//! Conventions used in this codebase:
//! - Right-handed view space (camera looks down -Z).
//! - Clip/NDC depth range is [0, 1] (wgpu/D3D). Near -> 0, Far -> 1.
//! - Fullscreen UVs have origin at top-left (v = 0 at top, v = 1 at bottom).
//!
use glam::{Mat4, Vec2, Vec3, Vec4};

fn uv_to_ndc_xy(uv: Vec2) -> Vec2 {
    // Map UV (origin top-left) to NDC (+Y up)
    Vec2::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0)
}

fn ndc_xy_to_uv(ndc_xy: Vec2) -> Vec2 {
    // Map NDC (+Y up) to UV (origin top-left)
    Vec2::new(ndc_xy.x * 0.5 + 0.5, 0.5 - ndc_xy.y * 0.5)
}

fn project_view_to_uv_depth(P: Mat4, view_pos: Vec3) -> (Vec2, f32) {
    let clip: Vec4 = P * view_pos.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let uv = ndc_xy_to_uv(ndc.truncate());
    let depth = ndc.z; // wgpu depth in [0, 1]
    (uv, depth)
}

fn reconstruct_view_position(uv: Vec2, depth: f32, proj_inv: Mat4) -> Vec3 {
    let ndc = Vec3::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let clip = ndc.extend(1.0);
    let view = proj_inv * clip;
    (view.truncate() / view.w)
}

fn approx_eq3(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a - b).abs().max_element() <= eps
}

#[test]
fn uv_ndc_y_flip_roundtrip_is_consistent() {
    let samples = [
        Vec2::new(0.0, 0.0),
        Vec2::new(0.25, 0.25),
        Vec2::new(0.5, 0.5),
        Vec2::new(0.75, 0.75),
        Vec2::new(1.0, 1.0),
    ];
    for &uv in &samples {
        let ndc_xy = uv_to_ndc_xy(uv);
        let uv_rt = ndc_xy_to_uv(ndc_xy);
        assert!((uv - uv_rt).abs().max_element() < 1e-6, "uv {:?} -> {:?} -> {:?}", uv, ndc_xy, uv_rt);
    }
}

#[test]
fn perspective_maps_near_far_to_wgpu_depth_range() {
    let fov = 60f32.to_radians();
    let aspect = 16.0 / 9.0;
    let near = 0.1;
    let far = 100.0;
    let P = Mat4::perspective_rh(fov, aspect, near, far);

    let (_, depth_near) = project_view_to_uv_depth(P, Vec3::new(0.0, 0.0, -near));
    let (_, depth_far) = project_view_to_uv_depth(P, Vec3::new(0.0, 0.0, -far));

    assert!((depth_near - 0.0).abs() < 1e-5, "near -> depth {}, expected 0.0", depth_near);
    assert!((depth_far - 1.0).abs() < 1e-5, "far -> depth {}, expected 1.0", depth_far);
}

#[test]
fn reconstruct_view_position_roundtrips_through_projection() {
    let fov = 60f32.to_radians();
    let aspect = 16.0 / 9.0;
    let near = 0.1;
    let far = 50.0;
    let P = Mat4::perspective_rh(fov, aspect, near, far);
    let P_inv = P.inverse();

    // Test a few positions inside the frustum (z < -near)
    let points = [
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(0.2, -0.1, -2.5),
        Vec3::new(1.0, 0.5, -3.0),
        Vec3::new(-0.75, 0.25, -5.0),
    ];

    for &p_view in &points {
        let (uv, depth) = project_view_to_uv_depth(P, p_view);
        // Make sure the point is actually in front of the near plane
        assert!(depth > 0.0 && depth < 1.0, "depth out of range: {} for {:?}", depth, p_view);

        let recon = reconstruct_view_position(uv, depth, P_inv);
        assert!(
            approx_eq3(recon, p_view, 1e-4),
            "reconstruct mismatch: orig={:?}, recon={:?}, uv={:?}, depth={}",
            p_view,
            recon,
            uv,
            depth
        );
    }
}

