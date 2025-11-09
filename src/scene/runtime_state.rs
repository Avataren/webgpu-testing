use crate::scripting::ScriptingState;
use crate::time::Instant;
use hecs::{Entity, World};
use log::error;
use std::collections::HashMap;

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
            scripting: ScriptingState::new()
                .expect("Failed to initialize Lua scripting runtime"),
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
        // Extract all state before reset for Lua scripts
        let saved_state = self.scripting.extract_state_for_world(world);

        {
            use crate::scripting::LuaScriptComponent;
            let mut query = world.query::<&mut LuaScriptComponent>();
            for (_, component) in query.iter() {
                component.set_created_called(false);
            }
        }

        self.scripting.reset_runtime();
        self.scripting.restore_state(world, saved_state);
    }

    pub(crate) fn advance_time(&mut self, dt: f64) -> f64 {
        self.time += dt;
        self.time
    }

    pub(crate) fn run_scripts(&mut self, world: &mut World, dt: f64, editor_mode: bool) {
        // Run Lua scripts
        if let Err(err) = self.scripting.process_scripts(world, dt, editor_mode) {
            error!("Lua scripting error: {err}");
        }
    }

    /// Process UI for all scripts and return their UI commands.
    pub(crate) fn process_script_ui(
        &mut self,
        world: &World,
        editor_mode: bool,
        viewport_width: Option<f32>,
        viewport_height: Option<f32>,
        pixels_per_point: Option<f32>,
        per_entity_viewports: Option<&HashMap<Entity, (f32, f32, f32)>>,
    ) -> HashMap<Entity, Vec<crate::scripting::lua::api::ui::UiCommand>> {
        // Collect Lua UI commands
        self.scripting.process_ui(
            world,
            editor_mode,
            viewport_width,
            viewport_height,
            pixels_per_point,
            per_entity_viewports,
        )
    }

    /// Set UI responses from the previous frame to feed back to scripts.
    pub(crate) fn set_ui_responses_for_world(
        &mut self,
        world_id: usize,
        responses: HashMap<Entity, HashMap<String, crate::scripting::lua::api::ui::UiResponse>>,
    ) {
        self.scripting.set_ui_responses(world_id, responses);
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
    use crate::scripting::LuaScriptComponent;
    use glam::Vec3;
    use hecs::World;

    #[test]
    fn runtime_advances_time_and_runs_scripts() {
        let mut runtime = SceneRuntime::new();
        assert_eq!(runtime.time(), 0.0);

        let mut world = World::new();
        let entity = world.spawn((
            TransformComponent(Transform::default()),
            LuaScriptComponent::new_inline(
                "RuntimeTest",
                r#"
                function on_created(self_entity)
                    set_translation(self_entity, 1.0, 2.0, 3.0)
                end

                function update(self_entity, dt)
                    set_translation(self_entity, dt, dt, dt)
                end
                "#,
            ),
        ));

        runtime.run_scripts(&mut world, 0.0, false);
        {
            let transform = world.get::<&TransformComponent>(entity).unwrap();
            assert_eq!(transform.0.translation, Vec3::new(1.0, 2.0, 3.0));
        }

        let absolute_time = runtime.advance_time(0.5);
        assert!((absolute_time - 0.5).abs() < f64::EPSILON);

        runtime.run_scripts(&mut world, 0.5, false);
        let transform = world.get::<&TransformComponent>(entity).unwrap();
        assert_eq!(transform.0.translation, Vec3::splat(0.5));
    }
}
