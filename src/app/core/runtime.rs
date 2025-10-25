use std::sync::{Arc, Mutex};

use crate::scene::{Scene, SceneSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    Editor,
    Playing,
}

#[derive(Clone)]
pub struct RuntimeStateHandle {
    inner: Arc<Mutex<RuntimeState>>,
}

struct RuntimeState {
    desired_mode: RuntimeMode,
    active_mode: RuntimeMode,
}

impl RuntimeStateHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeState {
                desired_mode: RuntimeMode::Editor,
                active_mode: RuntimeMode::Editor,
            })),
        }
    }

    pub fn request_mode(&self, mode: RuntimeMode) {
        let mut state = self.lock();
        state.desired_mode = mode;
    }

    pub fn desired_mode(&self) -> RuntimeMode {
        self.lock().desired_mode
    }

    pub fn active_mode(&self) -> RuntimeMode {
        self.lock().active_mode
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RuntimeState> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for RuntimeStateHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeTransition {
    EnteredEditor,
    EnteredPlaying,
}

pub struct RuntimeController {
    mode: RuntimeMode,
    state: RuntimeStateHandle,
    editor_snapshot: Option<SceneSnapshot>,
    gizmos_enabled: bool,
    editor_gizmos_enabled: bool,
}

impl RuntimeController {
    pub fn new() -> Self {
        Self {
            mode: RuntimeMode::Editor,
            state: RuntimeStateHandle::new(),
            editor_snapshot: None,
            gizmos_enabled: true,
            editor_gizmos_enabled: true,
        }
    }

    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    pub fn state_handle(&self) -> RuntimeStateHandle {
        self.state.clone()
    }

    pub fn gizmos_enabled(&self) -> bool {
        self.gizmos_enabled
    }

    pub fn toggle_editor_gizmos(&mut self) {
        if self.mode == RuntimeMode::Editor {
            self.editor_gizmos_enabled = !self.editor_gizmos_enabled;
        }

        self.gizmos_enabled = match self.mode {
            RuntimeMode::Editor => self.editor_gizmos_enabled,
            RuntimeMode::Playing => false,
        };
    }

    pub fn sync_runtime_state(&mut self, scene: &mut Scene) -> Option<RuntimeTransition> {
        let desired_mode = {
            let mut state = self.state.lock();

            if state.desired_mode == self.mode {
                if state.active_mode != self.mode {
                    state.active_mode = self.mode;
                }
                return None;
            }

            state.desired_mode
        };

        match desired_mode {
            RuntimeMode::Editor => {
                self.stop_playback(scene);
                self.state.lock().active_mode = RuntimeMode::Editor;
                Some(RuntimeTransition::EnteredEditor)
            }
            RuntimeMode::Playing => {
                self.start_playback(scene);
                self.state.lock().active_mode = RuntimeMode::Playing;
                Some(RuntimeTransition::EnteredPlaying)
            }
        }
    }

    fn start_playback(&mut self, scene: &mut Scene) {
        if self.mode == RuntimeMode::Playing {
            return;
        }

        let snapshot = SceneSnapshot::capture(scene);
        self.editor_snapshot = Some(snapshot);
        scene.reset_script_runtime();
        scene.set_animation_playback(true);
        scene.set_time(0.0);
        scene.init_timer();
        self.gizmos_enabled = false;
        self.mode = RuntimeMode::Playing;
    }

    fn stop_playback(&mut self, scene: &mut Scene) {
        if self.mode == RuntimeMode::Editor {
            return;
        }

        if let Some(snapshot) = self.editor_snapshot.take() {
            *scene = snapshot.into_scene();
            scene.init_timer();
            scene.reset_script_runtime();
            scene.set_animation_playback(false);
            scene.update(0.0);
            if !scene.has_any_lights() {
                scene.add_default_lighting();
            }
        }

        self.gizmos_enabled = self.editor_gizmos_enabled;
        self.mode = RuntimeMode::Editor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_editor_gizmos_only_affects_editor_mode() {
        let mut controller = RuntimeController::new();
        assert!(controller.gizmos_enabled());

        controller.toggle_editor_gizmos();
        assert!(!controller.gizmos_enabled());

        controller.toggle_editor_gizmos();
        assert!(controller.gizmos_enabled());
    }

    #[test]
    fn transition_updates_modes() {
        let mut controller = RuntimeController::new();
        let mut scene = Scene::new();

        controller.state_handle().request_mode(RuntimeMode::Playing);
        let transition = controller.sync_runtime_state(&mut scene);
        assert_eq!(transition, Some(RuntimeTransition::EnteredPlaying));
        assert_eq!(controller.mode(), RuntimeMode::Playing);

        controller.state_handle().request_mode(RuntimeMode::Editor);
        let transition = controller.sync_runtime_state(&mut scene);
        assert_eq!(transition, Some(RuntimeTransition::EnteredEditor));
        assert_eq!(controller.mode(), RuntimeMode::Editor);
    }
}
