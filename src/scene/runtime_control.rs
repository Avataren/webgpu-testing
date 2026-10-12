use super::runtime_state::SceneRuntime;
use crate::scripting::lua::api::ui::{UiCommand, UiResponse};
use crate::scripting::lua::state::UiViewportInfo;
use crate::scripting::ScriptingState;
use crate::time::Instant;
use hecs::{Entity, World};
use std::collections::HashMap;

/// Coordinates time-keeping and script execution for a [`Scene`](super::Scene).
pub(crate) struct SceneRuntimeController {
    runtime: SceneRuntime,
}

impl SceneRuntimeController {
    pub(crate) fn new() -> Self {
        Self {
            runtime: SceneRuntime::new(),
        }
    }

    pub(crate) fn scripting(&self) -> &ScriptingState {
        self.runtime.scripting()
    }

    pub(crate) fn scripting_mut(&mut self) -> &mut ScriptingState {
        self.runtime.scripting_mut()
    }

    pub(crate) fn init_timer(&mut self) {
        self.runtime.init_timer();
    }

    pub(crate) fn reset_script_runtime(&mut self) {
        self.runtime.reset_script_runtime();
    }

    pub(crate) fn time(&self) -> f64 {
        self.runtime.time()
    }

    pub(crate) fn set_time(&mut self, time: f64) {
        self.runtime.set_time(time);
    }

    pub(crate) fn last_frame(&self) -> Instant {
        self.runtime.last_frame()
    }

    pub(crate) fn last_frame_instant(&self) -> Option<Instant> {
        self.runtime.last_frame_instant()
    }

    pub(crate) fn set_last_frame(&mut self, instant: Instant) {
        self.runtime.set_last_frame(instant);
    }

    pub(crate) fn advance_time(&mut self, dt: f64) -> f64 {
        self.runtime.advance_time(dt)
    }

    pub(crate) fn run_scripts(&mut self, world: &mut World, dt: f64, editor_mode: bool) {
        self.runtime.run_scripts(world, dt, editor_mode);
    }

    pub(crate) fn process_script_ui(
        &mut self,
        world: &World,
        editor_mode: bool,
        viewport: Option<UiViewportInfo>,
    ) -> HashMap<Entity, Vec<UiCommand>> {
        self.runtime.process_script_ui(world, editor_mode, viewport)
    }

    pub(crate) fn set_ui_responses(
        &mut self,
        responses: HashMap<Entity, HashMap<String, UiResponse>>,
    ) {
        self.runtime.set_ui_responses(responses);
    }
}

impl Default for SceneRuntimeController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SceneRuntimeController;
    use crate::scene::components::TransformComponent;
    use crate::scene::transform::Transform;
    use crate::scripting::LuaScriptComponent;
    use glam::Vec3;
    use hecs::World;

    #[test]
    fn controller_advances_time_and_runs_scripts() {
        let mut controller = SceneRuntimeController::new();
        let mut world = World::new();
        let entity = world.spawn((
            TransformComponent(Transform::default()),
            LuaScriptComponent::new_inline(
                "RuntimeControllerTest",
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

        controller.run_scripts(&mut world, 0.0, false);
        {
            let transform = world.get::<&TransformComponent>(entity).unwrap();
            assert_eq!(transform.0.translation, Vec3::new(1.0, 2.0, 3.0));
        }

        let absolute_time = controller.advance_time(0.5);
        assert!((absolute_time - 0.5).abs() < f64::EPSILON);

        controller.run_scripts(&mut world, 0.5, false);
        let transform = world.get::<&TransformComponent>(entity).unwrap();
        assert_eq!(transform.0.translation, Vec3::splat(0.5));
    }
}
