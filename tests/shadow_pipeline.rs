use glam::{Mat4, Vec2, Vec3, Vec4};

const EPSILON: f32 = 1e-5;

#[derive(Clone, Copy)]
struct VertexInput {
    pos: Vec3,
    normal: Vec3,
    uv: Vec2,
    tangent: Vec4,
    instance: u32,
}

#[derive(Clone, Copy, Debug)]
struct VertexOutput {
    clip_position: Vec4,
    world_position: Vec3,
}

fn run_vertex_shader(input: VertexInput, model: Mat4, globals_view_proj: Mat4) -> VertexOutput {
    let world_position4 = model * input.pos.extend(1.0);
    let world_position = world_position4.truncate();

    let _ = input.uv;
    let _ = input.instance;
    let _normal = (model * input.normal.extend(0.0)).truncate().normalize();
    let _tangent = (model * Vec4::new(input.tangent.x, input.tangent.y, input.tangent.z, 0.0))
        .truncate()
        .normalize();
    let _bitangent = _normal.cross(_tangent) * input.tangent.w;

    let clip_position = globals_view_proj * world_position4;

    VertexOutput {
        clip_position,
        world_position,
    }
}

fn project_shadow_cpu(matrix: Mat4, world_pos: Vec3) -> Vec3 {
    let clip = matrix * world_pos.extend(1.0);
    if clip.w <= 0.0 {
        return Vec3::splat(-1.0);
    }
    let ndc = clip.truncate() / clip.w;
    Vec3::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5, ndc.z)
}

fn compute_ndc(matrix: Mat4, world_pos: Vec3) -> Vec3 {
    let clip = matrix * world_pos.extend(1.0);
    clip.truncate() / clip.w
}

#[derive(Clone, Copy)]
struct DirectionalShadow {
    view_proj: Mat4,
    up: Vec3,
}

fn build_directional_shadow_matrix(direction: Vec3) -> DirectionalShadow {
    const SHADOW_DISTANCE: f32 = 30.0;
    const SHADOW_SIZE: f32 = 15.0;

    let focus = Vec3::ZERO;
    let light_pos = focus - direction * SHADOW_DISTANCE;

    let up = if direction.abs().dot(Vec3::Y) > 0.95 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    let view = Mat4::look_at_rh(light_pos, focus, up);

    let left = -SHADOW_SIZE;
    let right = SHADOW_SIZE;
    let bottom = -SHADOW_SIZE;
    let top = SHADOW_SIZE;
    let near = 0.1;
    let far = SHADOW_DISTANCE * 2.0;

    let projection = Mat4::from_cols(
        Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / (top - bottom), 0.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0 / (far - near), 0.0),
        Vec4::new(
            -(right + left) / (right - left),
            -(top + bottom) / (top - bottom),
            -near / (far - near),
            1.0,
        ),
    );

    DirectionalShadow {
        view_proj: projection * view,
        up,
    }
}

fn approx_eq(a: Vec3, b: Vec3) {
    assert!(a.abs_diff_eq(b, EPSILON), "{a:?} != {b:?}");
}

#[test]
fn directional_shadow_pipeline_matches_manual_projection() {
    let model = Mat4::IDENTITY;

    let camera_view = Mat4::look_at_rh(Vec3::new(8.0, 10.0, 8.0), Vec3::ZERO, Vec3::Y);
    let camera_proj = Mat4::perspective_rh(45_f32.to_radians(), 1.0, 0.1, 100.0);
    let globals_view_proj = camera_proj * camera_view;

    let light_direction = Vec3::new(0.4, -1.0, 0.2).normalize();
    let light_shadow = build_directional_shadow_matrix(light_direction);
    let light_view_proj = light_shadow.view_proj;

    let inputs = [
        VertexInput {
            pos: Vec3::new(-3.5, 0.0, -2.0),
            normal: Vec3::Y,
            uv: Vec2::new(0.0, 0.0),
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            instance: 0,
        },
        VertexInput {
            pos: Vec3::new(2.0, 1.0, 4.0),
            normal: Vec3::Y,
            uv: Vec2::new(0.5, 0.5),
            tangent: Vec4::new(0.0, 0.0, 1.0, -1.0),
            instance: 0,
        },
        VertexInput {
            pos: Vec3::new(4.5, -0.5, -3.0),
            normal: Vec3::Z,
            uv: Vec2::new(1.0, 1.0),
            tangent: Vec4::new(0.0, 1.0, 0.0, 1.0),
            instance: 0,
        },
    ];

    for input in inputs {
        let vs_out = run_vertex_shader(input, model, globals_view_proj);
        let ndc = compute_ndc(light_view_proj, vs_out.world_position);
        let expected = Vec3::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
        let projected = project_shadow_cpu(light_view_proj, vs_out.world_position);

        approx_eq(projected, expected);
        assert!(projected.x >= -EPSILON && projected.x <= 1.0 + EPSILON);
        assert!(projected.y >= -EPSILON && projected.y <= 1.0 + EPSILON);
        assert!(projected.z >= 0.0 - EPSILON && projected.z <= 1.0 + EPSILON);
        assert!((projected.z - ndc.z).abs() < EPSILON);
        assert!(vs_out.clip_position.w > 0.0);
    }
}

#[test]
fn directional_shadow_texture_axis_is_flipped_from_clip_space() {
    let light_direction = Vec3::new(0.0, -1.0, 0.0);
    let light_shadow = build_directional_shadow_matrix(light_direction);
    let light_view_proj = light_shadow.view_proj;

    let top_world = light_shadow.up * 5.0;
    let bottom_world = -light_shadow.up * 5.0;

    let ndc_top = compute_ndc(light_view_proj, top_world);
    let ndc_bottom = compute_ndc(light_view_proj, bottom_world);
    assert!(ndc_top.y > ndc_bottom.y);

    let tex_top = project_shadow_cpu(light_view_proj, top_world);
    let tex_bottom = project_shadow_cpu(light_view_proj, bottom_world);

    assert!(tex_top.y < tex_bottom.y);
    assert!((tex_top.z - ndc_top.z).abs() < EPSILON);
    assert!((tex_bottom.z - ndc_bottom.z).abs() < EPSILON);
}

#[test]
fn spot_shadow_projection_rejects_points_behind_light() {
    let view = Mat4::look_at_rh(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, Vec3::Z);
    let projection = Mat4::perspective_rh(60_f32.to_radians(), 1.0, 0.1, 30.0);
    let spot_view_proj = projection * view;

    let behind_light = Vec3::new(0.0, 15.0, 0.0);
    let clip = spot_view_proj * behind_light.extend(1.0);
    assert!(clip.w <= 0.0);

    let projected = project_shadow_cpu(spot_view_proj, behind_light);
    assert_eq!(projected, Vec3::splat(-1.0));

    let in_front = Vec3::new(0.0, 0.0, 0.0);
    let projected_front = project_shadow_cpu(spot_view_proj, in_front);
    assert!(projected_front.x >= -EPSILON && projected_front.x <= 1.0 + EPSILON);
    assert!(projected_front.y >= -EPSILON && projected_front.y <= 1.0 + EPSILON);
    assert!(projected_front.z >= 0.0 - EPSILON && projected_front.z <= 1.0 + EPSILON);
}
