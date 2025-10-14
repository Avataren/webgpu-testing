// examples/particle_effects.rs - Updated for your Material system
use glam::{Quat, Vec3};

use wgpu_cube::gpu_particles::behaviors::PhysicsBehavior;
use wgpu_cube::gpu_particles::{ColorGradient, GpuParticleSystem, ParticleEmitter};
use wgpu_cube::renderer::{CustomRenderContext, Material};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::{
    render_application::RenderApplication, run_application, AppBuilder, GpuUpdateContext,
    StartupContext,
};

const MAX_PARTICLES: u32 = 5000;

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
            
            // Fountain: Water with gravity and bounce
            fountain_behavior: PhysicsBehavior::default()
                .with_gravity(Vec3::new(0.0, -15.0, 0.0))
                .with_drag(0.2)
                .with_ground_collision(0.0, 0.3, 0.7),
            
            // Fireworks: Explosive motion with gravity
            fireworks_behavior: PhysicsBehavior::default()
                .with_gravity(Vec3::new(0.0, -8.0, 0.0))
                .with_drag(0.15),
            
            // Smoke: Gentle upward drift, NO turbulence for smooth motion
            smoke_behavior: PhysicsBehavior::default()
                .with_gravity(Vec3::new(0.0, 1.5, 0.0))
                .with_drag(1.5),
            
            firework_timer: 0.0,
            next_firework_time: 2.0,
        }
    }
}

impl RenderApplication for ParticleEffectsApp {
    fn name(&self) -> &str {
        "GPU Particle Effects - Billboarded & 3D"
    }

    fn configure(&self, _builder: &mut AppBuilder) {}

    fn setup(&mut self, ctx: &mut StartupContext) {
        // Create cube mesh for 3D particles (fountain)
        let (cube_vertices, cube_indices) = wgpu_cube::renderer::cube_mesh();
        let cube_mesh = ctx.renderer.create_mesh(&cube_vertices, &cube_indices);
        self.cube_mesh_handle = Some(ctx.scene.assets.meshes.insert(cube_mesh));

        // Create quad mesh for billboarded particles (smoke, fireworks)
        let (quad_vertices, quad_indices) = wgpu_cube::renderer::quad_mesh();
        let quad_mesh = ctx.renderer.create_mesh(&quad_vertices, &quad_indices);
        self.quad_mesh_handle = Some(ctx.scene.assets.meshes.insert(quad_mesh));

        // Scene setup
        ctx.scene.environment_mut().set_clear_color(wgpu::Color {
            r: 0.02,
            g: 0.02,
            b: 0.05,
            a: 1.0,
        });
        ctx.scene.environment_mut().disable_hdr_background();

        ctx.scene.set_camera(wgpu_cube::scene::Camera {
            eye: Vec3::new(0.0, 5.0, 20.0),
            target: Vec3::new(0.0, 5.0, 0.0),
            up: Vec3::Y,
            fov_y_radians: 60f32.to_radians(),
            near: 0.1,
            far: 100.0,
            ..Default::default()
        });

        // Lighting
        let sun_direction = Vec3::new(0.3, -1.0, -0.5).normalize();
        let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);

        ctx.scene.world_mut().spawn((
            wgpu_cube::scene::Name::new("Main Light"),
            wgpu_cube::scene::TransformComponent(wgpu_cube::scene::Transform::from_trs(
                Vec3::ZERO,
                sun_rotation,
                Vec3::ONE,
            )),
            DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 2.0),
            CanCastShadow(true),
        ));

        // ====================================================================
        // FOUNTAIN: 3D Cube Particles (no billboarding)
        // ====================================================================
        let fountain_material = Material::new([76, 128, 255, 204])
            .with_alpha()  // ✅ Your API uses with_alpha_blend()
            .with_unlit();
        // Note: NO .with_billboarding() - uses 3D rotation

        let mut fountain_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            MAX_PARTICLES,
            fountain_material,
            &self.fountain_behavior,
        );

        fountain_system.add_emitter(ParticleEmitter::fountain(Vec3::new(-8.0, 0.0, 0.0)));
        log::info!("Fountain: 3D cubes with physics");
        self.fountain_system = Some(fountain_system);

        // ====================================================================
        // FIREWORKS: Billboarded Quad Particles
        // ====================================================================
        let fireworks_material = Material::new([255, 204, 51, 255])
            .with_unlit()
            .with_billboarding();  // ✅ Enable billboarding

        let fireworks_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            MAX_PARTICLES,
            fireworks_material,
            &self.fireworks_behavior,
        );

        log::info!("Fireworks: Billboarded quads");
        self.fireworks_system = Some(fireworks_system);

        // ====================================================================
        // SMOKE: Billboarded Quad Particles
        // ====================================================================
        let smoke_material = Material::new([128, 128, 128, 153])
            .with_alpha()
            .with_unlit()
            .with_billboarding();  // ✅ Enable billboarding

        let mut smoke_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            1000,
            smoke_material,
            &self.smoke_behavior,
        );

        smoke_system.add_emitter(ParticleEmitter::smoke(Vec3::new(8.0, 0.0, 0.0)));
        log::info!("Smoke: Billboarded quads");
        self.smoke_system = Some(smoke_system);

        log::info!("=== Particle Effects Ready ===");
        log::info!("Left: Fountain (3D cubes)");
        log::info!("Center: Fireworks (billboarded)");
        log::info!("Right: Smoke (billboarded)");
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        let mut encoder = ctx.renderer.get_device()
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

        // Spawn fireworks periodically
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
                    .with_keyframe([1.0, 0.5, 0.0, 1.0], 0.3)
                    .with_keyframe([1.0, 0.2, 0.0, 0.3], 1.0),
                ColorGradient::new()
                    .with_keyframe([0.2, 0.8, 1.0, 1.0], 0.0)
                    .with_keyframe([0.0, 0.5, 1.0, 1.0], 0.3)
                    .with_keyframe([0.0, 0.2, 1.0, 0.3], 1.0),
                ColorGradient::new()
                    .with_keyframe([0.2, 1.0, 0.3, 1.0], 0.0)
                    .with_keyframe([0.0, 1.0, 0.5, 1.0], 0.3)
                    .with_keyframe([0.0, 1.0, 0.2, 0.3], 1.0),
            ];
            
            let color = colors[(rand::random::<f32>() * 3.0) as usize % 3].clone();
            let firework = ParticleEmitter::firework(Vec3::new(x, y, z))
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
        // Render fountain with cube mesh (3D geometry)
        if let Some(cube_handle) = self.cube_mesh_handle {
            if let Some(mesh) = ctx.scene.assets.meshes.get(cube_handle) {
                if let Some(system) = self.fountain_system.as_mut() {
                    system.render(ctx, mesh);
                }
            }
        }

        // Render fireworks and smoke with quad mesh (billboarded)
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