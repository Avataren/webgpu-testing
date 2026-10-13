use super::system::{EditorContext, EditorSystem};
use wgpu_cube::gpu_particles::ParticleRuntime;
use wgpu_cube::renderer::CustomRenderContext;

/// Runs the scene's GPU particles (entity particle systems and script bursts) in the
/// editor; the player uses the same [`ParticleRuntime`].
#[derive(Default)]
pub(crate) struct EditorParticleSystem {
    runtime: ParticleRuntime,
}

impl EditorParticleSystem {
    pub(crate) fn render(&mut self, ctx: &mut CustomRenderContext<'_>) {
        self.runtime.render(ctx);
    }

    pub(crate) fn has_shadow_casters(&self) -> bool {
        self.runtime.has_shadow_casters()
    }
}

impl EditorSystem for EditorParticleSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let Some(gpu_ctx) = ctx.gpu_context_mut() else {
            return;
        };

        self.runtime.update(gpu_ctx);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
