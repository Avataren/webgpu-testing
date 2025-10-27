use crate::scripting::{RuneScriptComponent, RuneScriptSource, ScriptingState};
use crate::time::Instant;
use hecs::World;
use log::error;

pub(crate) struct SceneRuntime {
    time: f64,
    last_frame: Option<Instant>,
    scripting: ScriptingState,
}

impl SceneRuntime {
    pub(crate) fn new() -> Self {
        Self {
            time: 0.0,
            last_frame: None,
            scripting: ScriptingState::default(),
        }
    }

    pub(crate) fn time(&self) -> f64 {
        self.time
    }

    pub(crate) fn set_time(&mut self, time: f64) {
        self.time = time;
    }

    pub(crate) fn init_timer(&mut self) {
        self.last_frame = Some(Instant::now());
    }

    pub(crate) fn last_frame(&self) -> Instant {
        self.last_frame
            .expect("Scene timer not initialized - call init_timer() first")
    }

    pub(crate) fn last_frame_instant(&self) -> Option<Instant> {
        self.last_frame
    }

    pub(crate) fn set_last_frame(&mut self, instant: Instant) {
        self.last_frame = Some(instant);
    }

    pub(crate) fn scripting(&self) -> &ScriptingState {
        &self.scripting
    }

    pub(crate) fn scripting_mut(&mut self) -> &mut ScriptingState {
        &mut self.scripting
    }

    pub(crate) fn reset_script_runtime(&mut self, world: &mut World) {
        {
            let mut query = world.query::<&mut RuneScriptComponent>();
            for (_, component) in query.iter() {
                let should_skip = match component.source() {
                    RuneScriptSource::Inline { name, .. } => {
                        let name_ref = name.as_ref();
                        name_ref == "editor_startup.rn"
                            || name_ref.starts_with("editor_import_gltf::")
                    }
                    RuneScriptSource::File { .. } => false,
                };

                if should_skip {
                    continue;
                }

                component.set_created_called(false);
            }
        }

        self.scripting.reset_runtime();
    }

    pub(crate) fn advance_time(&mut self, dt: f64) -> f64 {
        self.time += dt;
        self.time
    }

    pub(crate) fn run_scripts(&mut self, world: &mut World, dt: f64) {
        if let Err(err) = self.scripting.update_scripts(world, dt) {
            error!("Rune scripting error: {err}");
        }
    }
}

impl Default for SceneRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SceneRuntime;
    use crate::scene::components::TransformComponent;
    use crate::scene::transform::Transform;
    use crate::scripting::RuneScriptComponent;
    use glam::Vec3;
    use hecs::World;

    #[test]
    fn runtime_advances_time_and_runs_scripts() {
        let mut runtime = SceneRuntime::new();
        assert_eq!(runtime.time(), 0.0);

        let mut world = World::new();
        let entity = world.spawn((
            TransformComponent(Transform::default()),
            RuneScriptComponent::new_inline(
                "RuntimeTest",
                r#"
                pub fn on_created(self_entity) {
                    set_translation(self_entity, 1.0, 2.0, 3.0);
                }

                pub fn update(self_entity, dt) {
                    set_translation(self_entity, dt, dt, dt);
                }
                "#,
            ),
        ));

        runtime.run_scripts(&mut world, 0.0);
        {
            let transform = world.get::<&TransformComponent>(entity).unwrap();
            assert_eq!(transform.0.translation, Vec3::new(1.0, 2.0, 3.0));
        }

        let absolute_time = runtime.advance_time(0.5);
        assert!((absolute_time - 0.5).abs() < f64::EPSILON);

        runtime.run_scripts(&mut world, 0.5);
        let transform = world.get::<&TransformComponent>(entity).unwrap();
        assert_eq!(transform.0.translation, Vec3::splat(0.5));
    }
}
