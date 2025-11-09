use crate::scripting::{RuneScriptComponent, RuneScriptSource, ScriptingState};
use crate::time::Instant;
use hecs::{Entity, World};
use log::error;
use std::collections::HashMap;

pub(crate) struct SceneRuntime {
    time: f64,
    last_frame: Option<Instant>,
    scripting: ScriptingState,
    lua_scripting: crate::scripting::lua::state::ScriptingState,
}

impl SceneRuntime {
    pub(crate) fn new() -> Self {
        Self {
            time: 0.0,
            last_frame: None,
            scripting: ScriptingState::default(),
            lua_scripting: crate::scripting::lua::state::ScriptingState::new()
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

    pub(crate) fn lua_scripting(&self) -> &crate::scripting::lua::state::ScriptingState {
        &self.lua_scripting
    }

    pub(crate) fn lua_scripting_mut(
        &mut self,
    ) -> &mut crate::scripting::lua::state::ScriptingState {
        &mut self.lua_scripting
    }

    pub(crate) fn reset_script_runtime(&mut self, world: &mut World) {
        // Extract all state before reset for Rune scripts
        let saved_state = self.scripting.extract_all_state();

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

        // Restore state will be called automatically after instances are recreated
        // We store the state in the runtime for restoration after on_created()
        self.scripting.set_pending_state_restoration(saved_state);

        // Reset Lua scripts
        let lua_saved_state = self.lua_scripting.extract_state_for_world(world);

        {
            use crate::scripting::LuaScriptComponent;
            let mut query = world.query::<&mut LuaScriptComponent>();
            for (_, component) in query.iter() {
                component.set_created_called(false);
            }
        }

        self.lua_scripting.reset_runtime();
        self.lua_scripting.restore_state(world, lua_saved_state);
    }

    pub(crate) fn advance_time(&mut self, dt: f64) -> f64 {
        self.time += dt;
        self.time
    }

    pub(crate) fn run_scripts(&mut self, world: &mut World, dt: f64, editor_mode: bool) {
        // Run Rune scripts
        if let Err(err) = self.scripting.update_scripts(world, dt, editor_mode) {
            error!("Rune scripting error: {err}");
        }

        // Run Lua scripts
        if let Err(err) = self.lua_scripting.process_scripts(world, dt, editor_mode) {
            error!("Lua scripting error: {err}");
        }
    }

    /// Process UI for all scripts (both Rune and Lua) and return their UI commands.
    pub(crate) fn process_script_ui(
        &mut self,
        world: &World,
        editor_mode: bool,
        viewport_width: Option<f32>,
        viewport_height: Option<f32>,
        pixels_per_point: Option<f32>,
    ) -> HashMap<Entity, Vec<crate::scripting::rune::api::ui::UiCommand>> {
        // Collect Rune UI commands (Rune doesn't support viewport info yet)
        let mut all_commands = self.scripting.process_ui(world, editor_mode);

        // Collect Lua UI commands and convert them to Rune UiCommand type
        let lua_commands = self.lua_scripting.process_ui(world, editor_mode, viewport_width, viewport_height, pixels_per_point);
        for (entity, cmds) in lua_commands {
            // Convert Lua UiCommands to Rune UiCommands
            let converted_cmds: Vec<crate::scripting::rune::api::ui::UiCommand> =
                cmds.into_iter().map(|cmd| cmd.into()).collect();
            all_commands
                .entry(entity)
                .or_insert_with(Vec::new)
                .extend(converted_cmds);
        }

        all_commands
    }

    /// Set UI responses from the previous frame to feed back to scripts.
    pub(crate) fn set_ui_responses_for_world(
        &mut self,
        world_id: usize,
        responses: HashMap<Entity, HashMap<String, crate::scripting::rune::api::ui::UiResponse>>,
        include_rune: bool,
    ) {
        if include_rune {
            self.scripting.set_ui_responses(responses.clone());
        }

        let lua_responses: HashMap<
            Entity,
            HashMap<String, crate::scripting::lua::api::ui::UiResponse>,
        > = responses
            .into_iter()
            .map(|(entity, widget_responses)| {
                let converted: HashMap<String, crate::scripting::lua::api::ui::UiResponse> =
                    widget_responses
                        .into_iter()
                        .map(|(id, resp)| (id, resp.into()))
                        .collect();
                (entity, converted)
            })
            .collect();
        self.lua_scripting.set_ui_responses(world_id, lua_responses);
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
