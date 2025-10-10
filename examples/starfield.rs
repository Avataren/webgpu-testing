use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::scene::{
    Camera, MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const STAR_COUNT: usize = 50_000;
const FIELD_HALF_SIZE: f32 = 60.0;
const NEAR_PLANE: f32 = 0.01;
const FAR_PLANE: f32 = 250.0;
const FAR_RESET_BAND: f32 = 25.0;
const STAR_SPEED_RANGE: std::ops::Range<f32> = 5.0..25.0;
const SPIN_SPEED_RANGE: std::ops::Range<f32> = 0.0..3.5;
const STAR_SCALE_RANGE: std::ops::Range<f32> = 0.25..0.75;

#[derive(Clone, Copy)]
struct StarfieldMotion {
    speed: f32,
    angular_axis: Vec3,
    angular_speed: f32,
}

impl StarfieldMotion {
    fn random(rng: &mut SmallRng) -> Self {
        let speed = rng.gen_range(STAR_SPEED_RANGE.clone());
        let angular_speed = rng.gen_range(SPIN_SPEED_RANGE.clone());
        let axis = random_unit_vector(rng);

        Self {
            speed,
            angular_axis: axis,
            angular_speed,
        }
    }
}

struct StarfieldApp {
    rng: SmallRng,
}

impl Default for StarfieldApp {
    fn default() -> Self {
        Self {
            rng: SmallRng::seed_from_u64(0x5EED_CAFE),
        }
    }
}

impl RenderApplication for StarfieldApp {
    fn name(&self) -> &str {
        "3D Starfield"
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        log::info!("Spawning {} cubes for starfield", STAR_COUNT);

        let (verts, idx) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&verts, &idx);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        let material = Material::white();
        ctx.scene.set_camera(Camera {
            eye: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            near: NEAR_PLANE,
            far: FAR_PLANE,
            ..Camera::default()
        });

        let sun1_direction = Vec3::new(0.3, -1.0, -1.1).normalize();
        let sun1_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun1_direction);

        ctx.scene.world.spawn((
            Name::new("Default Sky Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, sun1_rotation, Vec3::ONE)),
            DirectionalLight::new(Vec3::new(0.49, 0.95, 0.85), 2.5),
            CanCastShadow(false),
        ));

        for _ in 0..STAR_COUNT {
            let mut transform = Transform::from_trs(
                random_initial_position(&mut self.rng),
                random_rotation(&mut self.rng),
                Vec3::splat(random_scale(&mut self.rng)),
            );
            transform.rotation = transform.rotation.normalize();

            let motion = StarfieldMotion::random(&mut self.rng);

            ctx.scene.world.spawn((
                TransformComponent(transform),
                MeshComponent(mesh_handle),
                MaterialComponent(material),
                Visible(true),
                motion,
            ));
        }
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let dt = ctx.dt as f32;
        if dt <= f32::EPSILON {
            return;
        }

        let query = ctx
            .scene
            .world
            .query_mut::<(&mut TransformComponent, &mut StarfieldMotion)>();

        let rng = &mut self.rng;

        for (_, (transform, motion)) in query.into_iter() {
            transform.0.translation.z += motion.speed * dt;

            if motion.angular_speed > 0.0 {
                let delta = Quat::from_axis_angle(motion.angular_axis, motion.angular_speed * dt);
                transform.0.rotation = (delta * transform.0.rotation).normalize();
            }

            if transform.0.translation.z > -NEAR_PLANE {
                respawn_star(&mut transform.0, motion, rng);
            }
        }
    }
}

fn random_initial_position(rng: &mut SmallRng) -> Vec3 {
    let x = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
    let y = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
    let z = -rng.gen_range(NEAR_PLANE..FAR_PLANE);
    Vec3::new(x, y, z)
}

fn random_far_position(rng: &mut SmallRng) -> Vec3 {
    let x = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
    let y = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
    let z = -FAR_PLANE - rng.gen_range(0.0..FAR_RESET_BAND);
    Vec3::new(x, y, z)
}

fn random_rotation(rng: &mut SmallRng) -> Quat {
    let axis = random_unit_vector(rng);
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    Quat::from_axis_angle(axis, angle)
}

fn random_unit_vector(rng: &mut SmallRng) -> Vec3 {
    let mut v = Vec3::new(
        rng.gen_range(-1.0..=1.0),
        rng.gen_range(-1.0..=1.0),
        rng.gen_range(-1.0..=1.0),
    );
    if v.length_squared() < 1e-6 {
        v = Vec3::Y;
    }
    v.normalize()
}

fn random_scale(rng: &mut SmallRng) -> f32 {
    rng.gen_range(STAR_SCALE_RANGE.clone())
}

fn respawn_star(transform: &mut Transform, motion: &mut StarfieldMotion, rng: &mut SmallRng) {
    transform.translation = random_far_position(rng);
    transform.scale = Vec3::splat(random_scale(rng));
    transform.rotation = random_rotation(rng);

    *motion = StarfieldMotion::random(rng);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(StarfieldApp::default()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(StarfieldApp::default()).unwrap();
}
