use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

fn axis_angle_to_matrix(axis: Vec3, angle: f32) -> Mat3 {
    let rot = Quat::from_axis_angle(axis, angle);
    Mat3::from_quat(rot)
}

#[test]
fn gpu_particle_vertex_world_position_matches_trs() {
    let particle_position = Vec3::new(3.0, 1.5, -6.0);
    let axis = Vec3::new(0.0, 1.0, 0.0);
    let angle = 0.35;
    let scale = Vec3::splat(0.75);

    let vertex_pos = Vec3::new(-0.5, 0.25, 0.0);

    // Shader-style computation
    let rot_mat = axis_angle_to_matrix(axis.normalize(), angle);
    let scaled_pos = vertex_pos * scale;
    let rotated_pos = rot_mat * scaled_pos;
    let world_from_shader = rotated_pos + particle_position;

    // TRS matrix computation
    let trs = Mat4::from_scale_rotation_translation(scale, Quat::from_axis_angle(axis.normalize(), angle), particle_position);
    let world_from_trs = trs * vertex_pos.extend(1.0);

    assert!(
        world_from_shader.abs_diff_eq(world_from_trs.truncate(), 1e-5),
        "world pos mismatch: shader {:?} vs trs {:?}",
        world_from_shader,
        world_from_trs
    );

    // Project both through a camera
    let eye = Vec3::new(0.0, 0.0, 0.0);
    let target = Vec3::new(0.0, 0.0, -1.0);
    let up = Vec3::Y;
    let view = Mat4::look_at_rh(eye, target, up);
    let proj = Mat4::perspective_rh(60f32.to_radians(), 16.0 / 9.0, 0.1, 100.0);
    let vp = proj * view;

    let clip_shader = vp * Vec4::new(world_from_shader.x, world_from_shader.y, world_from_shader.z, 1.0);
    let clip_trs = vp * world_from_trs;

    assert!(
        clip_shader.abs_diff_eq(clip_trs, 1e-5),
        "clip mismatch: shader {:?} vs trs {:?}",
        clip_shader,
        clip_trs
    );
}
