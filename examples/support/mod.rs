//! Shared presentation defaults for the GPU smoke scenes.

use wgpu_cube::renderer::postprocess::{AntiAliasing, PostProcessSettings, Tonemapper};
use wgpu_cube::scene::{Name, PostProcessVolume, Scene};

pub fn configure(
    scene: &mut Scene,
    background: [f64; 3],
    ambient: f32,
    tune: impl FnOnce(&mut PostProcessSettings),
) {
    let environment = scene.environment_mut();
    environment.set_clear_color(wgpu::Color {
        r: background[0],
        g: background[1],
        b: background[2],
        a: 1.0,
    });
    environment.set_ambient_intensity(ambient);
    environment.disable_hdr_background();

    // Store the look in a scene volume: renderer settings alone are replaced each frame.
    // Fixed exposure also keeps material comparisons and smoke captures consistent.
    let mut settings = PostProcessSettings {
        tonemapping: Tonemapper::Aces,
        anti_aliasing: AntiAliasing::Fxaa,
        ..PostProcessSettings::default()
    };
    settings.exposure.compensation = 0.7;
    settings.bloom.enabled = false;
    settings.ssao.radius = 0.35;
    settings.ssao.intensity = 0.6;
    settings.ssao.power = 1.2;
    tune(&mut settings);
    scene.world_mut().spawn((
        Name::new("Showcase Post Processing"),
        PostProcessVolume::infinite(settings),
    ));
}
