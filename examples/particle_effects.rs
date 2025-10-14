// examples/particle_effects.rs - With floor and shadows
use glam::{Quat, Vec3};

use wgpu_cube::gpu_particles::behaviors::PhysicsBehavior;
use wgpu_cube::gpu_particles::{
    ColorGradient, EmissionShape, GpuParticleSystem, ParticleEmitter, SizeCurve,
};
use wgpu_cube::renderer::{CustomRenderContext, Material};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};
use wgpu_cube::{
    render_application::RenderApplication, run_application, AppBuilder, GpuUpdateContext,
    StartupContext,
};

const MAX_PARTICLES_FOUNTAIN: u32 = 25000;
const MAX_PARTICLES_FIREWORKS: u32 = 25000;
const MAX_PARTICLES_SMOKE: u32 = 25000;

const FOUNTAIN_RATE: f32 = 250.0;
const SMOKE_RATE: f32 = 120.0;
const FIREWORK_BURST_SIZE: u32 = 12500;

struct ParticleEffectsApp {
    fountain_system: Option<GpuParticleSystem>,
    fireworks_system: Option<GpuParticleSystem>,
    smoke_system: Option<GpuParticleSystem>,
    cube_mesh_handle: Option<wgpu_cube::asset::Handle<wgpu_cube::asset::Mesh>>,
    quad_mesh_handle: Option<wgpu_cube::asset::Handle<wgpu_cube::asset::Mesh>>,
    fountain_behavior: PhysicsBehavior,
    fireworks_behavior: PhysicsBehavior,
    smoke_behavior: PhysicsBehavior,
    firework_timer: f32,
    next_firework_time: f32,
}

impl ParticleEffectsApp {
    fn new() -> Self {
        Self {
            fountain_system: None,
            fireworks_system: None,
            smoke_system: None,
            cube_mesh_handle: None,
            quad_mesh_handle: None,
            fountain_behavior: PhysicsBehavior::default()
                .with_gravity(Vec3::new(0.0, -15.0, 0.0))
                .with_drag(0.2)
                .with_ground_collision(0.0, 0.3, 0.7),
            fireworks_behavior: PhysicsBehavior::default()
                .with_gravity(Vec3::new(0.0, -8.0, 0.0))
                .with_drag(0.15),
            smoke_behavior: PhysicsBehavior::default()
                .with_gravity(Vec3::new(0.0, 1.5, 0.0))
                .with_turbulence(0.0001, 10.0)
                .with_drag(1.5),
            firework_timer: 0.0,
            next_firework_time: 2.0,
        }
    }
}

impl RenderApplication for ParticleEffectsApp {
    fn name(&self) -> &str {
        "GPU Particle Effects - With Shadows"
    }

    fn configure(&self, _builder: &mut AppBuilder) {}

    fn setup(&mut self, ctx: &mut StartupContext) {
        // Create meshes
        let (cube_vertices, cube_indices) = wgpu_cube::renderer::cube_mesh();
        let cube_mesh = ctx.renderer.create_mesh(&cube_vertices, &cube_indices);
        let cube_mesh_handle = ctx.scene.assets.meshes.insert(cube_mesh);
        self.cube_mesh_handle = Some(cube_mesh_handle);

        let (quad_vertices, quad_indices) = wgpu_cube::renderer::quad_mesh();
        let quad_mesh = ctx.renderer.create_mesh(&quad_vertices, &quad_indices);
        let quad_mesh_handle = ctx.scene.assets.meshes.insert(quad_mesh);
        self.quad_mesh_handle = Some(quad_mesh_handle);

        // Scene setup
        ctx.scene.environment_mut().set_clear_color(wgpu::Color {
            r: 0.1,
            g: 0.15,
            b: 0.2,
            a: 1.0,
        });
        ctx.scene.environment_mut().disable_hdr_background();

        ctx.scene.set_camera(wgpu_cube::scene::Camera {
            eye: Vec3::new(0.0, 8.0, 25.0),
            target: Vec3::new(0.0, 3.0, 0.0),
            up: Vec3::Y,
            fov_y_radians: 60f32.to_radians(),
            near: 0.1,
            far: 100.0,
        });

        // ====================================================================
        // LIGHTING with shadows
        // ====================================================================
        let sun_direction = Vec3::new(0.3, -1.0, -0.5).normalize();
        let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);

        ctx.scene.world_mut().spawn((
            Name::new("Main Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, sun_rotation, Vec3::ONE)),
            DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 3.0),
            CanCastShadow(true), // ✅ Enable shadow casting
        ));

        // ====================================================================
        // FLOOR - Scaled cube with checker material
        // ====================================================================
        let floor_material = Material::checker().with_metallic(0.1).with_roughness(0.8);

        ctx.scene.world_mut().spawn((
            Name::new("Floor"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, -0.5, 0.0), // Position: just below y=0
                Quat::IDENTITY,
                Vec3::new(50.0, 0.1, 50.0), // Scale: 50x50 wide, 0.1 thick
            )),
            MeshComponent(cube_mesh_handle),
            MaterialComponent(floor_material),
            Visible(true),
        ));

        log::info!("Floor added at y=-0.5 with checker material");

        // ====================================================================
        // FOUNTAIN: 3D Cubes
        // ====================================================================
        let fountain_material = Material::new([76, 128, 255, 204]).with_alpha().with_unlit();

        let mut fountain_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            MAX_PARTICLES_FOUNTAIN,
            fountain_material,
            &self.fountain_behavior,
        );

        fountain_system.set_casts_shadows(true); // ✅ Enable shadows
        fountain_system.set_depth_write_enabled(true);

        let fountain_emitter = ParticleEmitter::new(Vec3::new(-8.0, 0.0, 0.0), FOUNTAIN_RATE)
            .with_emission_shape(EmissionShape::Cone {
                angle: std::f32::consts::PI / 8.0,
                radius: 0.3,
            })
            .with_velocity(Vec3::ZERO, Vec3::new(0.5, 0.5, 0.5))
            .with_radial_velocity(8.0, 12.0)
            .with_lifetime(2.0, 3.0)
            .with_scale(Vec3::splat(0.05), Vec3::splat(0.15))
            .with_color_gradient(
                ColorGradient::new()
                    .with_keyframe([0.3, 0.5, 1.0, 0.9], 0.0)
                    .with_keyframe([0.3, 0.5, 1.0, 0.6], 1.5)
                    .with_keyframe([0.2, 0.4, 0.9, 0.0], 2.0),
            )
            .with_size_curve(
                SizeCurve::new(1.0)
                    .with_keyframe(1.0, 1.0)
                    .with_keyframe(0.3, 2.0),
            );

        fountain_system.add_emitter(fountain_emitter);
        log::info!("Fountain: 3D cubes with shadows enabled");
        self.fountain_system = Some(fountain_system);

        // ====================================================================
        // FIREWORKS: Billboarded
        // ====================================================================
        let fireworks_material = Material::new([255, 204, 51, 255])
            .with_unlit()
            .with_billboarding();

        let mut fireworks_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            MAX_PARTICLES_FIREWORKS,
            fireworks_material,
            &self.fireworks_behavior,
        );

        fireworks_system.set_casts_shadows(true); // ✅ Enable shadows
        fireworks_system.set_depth_write_enabled(false);
        log::info!("Fireworks: Billboarded quads with shadows enabled");
        self.fireworks_system = Some(fireworks_system);

        // ====================================================================
        // SMOKE: Billboarded
        // ====================================================================
        let smoke_material = Material::new([128, 128, 128, 153])
            //.with_alpha()
            .with_unlit()
            .with_billboarding();

        let mut smoke_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            MAX_PARTICLES_SMOKE,
            smoke_material,
            &self.smoke_behavior,
        );

        smoke_system.set_casts_shadows(true); // ✅ Enable shadows
        smoke_system.set_depth_write_enabled(false);

        let smoke_emitter = ParticleEmitter::new(Vec3::new(8.0, 0.0, 0.0), SMOKE_RATE)
            .with_emission_shape(EmissionShape::Sphere { radius: 0.15 })
            .with_velocity(Vec3::new(-0.1, 0.2, -0.1), Vec3::new(0.1, 0.5, 0.1))
            .with_lifetime(5.0, 12.0)
            .with_scale(Vec3::splat(0.3), Vec3::splat(0.5))
            .with_color_gradient(
                ColorGradient::new()
                    .with_keyframe([0.7, 0.7, 0.7, 0.9], 0.0)
                    .with_keyframe([0.5, 0.5, 0.5, 0.6], 0.8)
                    .with_keyframe([0.3, 0.3, 0.3, 0.0], 1.0),
            )
            .with_size_curve(
                SizeCurve::new(0.5)
                    .with_keyframe(1.0, 0.5)
                    .with_keyframe(2.8, 1.0),
            );

        smoke_system.add_emitter(smoke_emitter);
        log::info!("Smoke: Billboarded quads with shadows enabled");
        self.smoke_system = Some(smoke_system);

        log::info!("=== Particle Effects Ready ===");
        log::info!("- Floor: 50x50 checker pattern at y=-0.5");
        log::info!("- Light: Directional with shadows");
        log::info!("- Left: Fountain (3D cubes, casts shadows)");
        log::info!("- Center: Fireworks (billboarded, casts shadows)");
        log::info!("- Right: Smoke (billboarded, casts shadows)");
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        let mut encoder =
            ctx.renderer
                .get_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ParticleUpdateEncoder"),
                });

        // Update fountain
        if let Some(system) = &mut self.fountain_system {
            system.update(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                &self.fountain_behavior,
                ctx.dt as f32,
            );
        }

        // Spawn fireworks
        self.firework_timer += ctx.dt as f32;
        if self.firework_timer >= self.next_firework_time {
            self.firework_timer = 0.0;
            self.next_firework_time = 2.0 + (rand::random::<f32>() * 2.0);

            let x = -4.0 + rand::random::<f32>() * 8.0;
            let y = 8.0 + rand::random::<f32>() * 4.0;
            let z = -2.0 + rand::random::<f32>() * 4.0;

            let colors = [
                ColorGradient::new()
                    .with_keyframe([1.0, 0.8, 0.2, 1.0], 0.0)
                    .with_keyframe([1.0, 0.5, 0.0, 1.0], 0.9)
                    .with_keyframe([1.0, 0.2, 0.0, 0.3], 1.0),
                ColorGradient::new()
                    .with_keyframe([0.2, 0.8, 1.0, 1.0], 0.0)
                    .with_keyframe([0.0, 0.5, 1.0, 1.0], 0.9)
                    .with_keyframe([0.0, 0.2, 1.0, 0.3], 1.0),
                ColorGradient::new()
                    .with_keyframe([0.2, 1.0, 0.3, 1.0], 0.0)
                    .with_keyframe([0.0, 1.0, 0.5, 1.0], 0.9)
                    .with_keyframe([0.0, 1.0, 0.2, 0.3], 1.0),
            ];

            let color = colors[(rand::random::<f32>() * 3.0) as usize % 3].clone();

            let firework = ParticleEmitter::new(Vec3::new(x, y, z), 0.0)
                .with_burst(FIREWORK_BURST_SIZE)
                .with_emission_shape(EmissionShape::RadialBurst)
                .with_radial_velocity(10.0, 15.0)
                .with_lifetime(1.5, 2.5)
                .with_scale(Vec3::splat(0.18), Vec3::splat(0.22))
                .with_color_gradient(color);
            

            if let Some(system) = &mut self.fireworks_system {
                system.add_emitter(firework);
            }
        }

        // Update fireworks
        if let Some(system) = &mut self.fireworks_system {
            system.update(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                &self.fireworks_behavior,
                ctx.dt as f32,
            );
        }

        // Update smoke
        if let Some(system) = &mut self.smoke_system {
            system.update(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                &self.smoke_behavior,
                ctx.dt as f32,
            );
        }

        ctx.renderer.get_queue().submit(Some(encoder.finish()));
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        // Render fountain with cube mesh
        if let Some(cube_handle) = self.cube_mesh_handle {
            if let Some(mesh) = ctx.scene.assets.meshes.get(cube_handle) {
                if let Some(system) = self.fountain_system.as_mut() {
                    system.render(ctx, mesh);
                }
            }
        }

        // Render fireworks and smoke with quad mesh
        if let Some(quad_handle) = self.quad_mesh_handle {
            if let Some(mesh) = ctx.scene.assets.meshes.get(quad_handle) {
                if let Some(system) = self.fireworks_system.as_mut() {
                    system.render(ctx, mesh);
                }
                if let Some(system) = self.smoke_system.as_mut() {
                    system.render(ctx, mesh);
                }
            }
        }
    }

    // ✅ CRITICAL: Tell the renderer that custom render includes shadow passes
    fn custom_render_includes_shadows(&self) -> bool {
        // Return true if any particle system has shadows enabled
        let fountain_shadows = self
            .fountain_system
            .as_ref()
            .is_some_and(|s| s.casts_shadows());
        let fireworks_shadows = self
            .fireworks_system
            .as_ref()
            .is_some_and(|s| s.casts_shadows());
        let smoke_shadows = self
            .smoke_system
            .as_ref()
            .is_some_and(|s| s.casts_shadows());

        fountain_shadows || fireworks_shadows || smoke_shadows
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ParticleEffectsApp::new()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_app() {
    run_application(ParticleEffectsApp::new()).unwrap();
}
