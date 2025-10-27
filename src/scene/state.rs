//! State helpers for the scene module.
//!
//! This module isolates subsystems that manage editor-facing scene state such as
//! the active environment and transform gizmo configuration. Splitting these
//! concerns away from `scene_core` keeps the graph/node management code focused
//! while still providing a single entry point (`Scene`) that coordinates all
//! subsystems.

use super::internal::{gizmos, transform_gizmos};
use crate::asset::Assets;
use crate::environment::Environment;
use crate::renderer::Renderer;
use crate::scene::components::EnvironmentComponent;
use hecs::{Entity, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformGizmoMode {
    Translate,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformGizmoSpace {
    Local,
    World,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformGizmoAxis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformGizmoHandle {
    TranslateAxis(TransformGizmoAxis),
    TranslateCenter,
    TranslatePlane(TransformGizmoAxis, TransformGizmoAxis),
    RotateAxis(TransformGizmoAxis),
    RotateScreen,
    ScaleAxis(TransformGizmoAxis),
    ScaleUniform,
}

/// Tracks the active lighting environment for a scene and keeps environment
/// components within the ECS world synchronised with the canonical values.
#[derive(Clone, Debug)]
pub struct EnvironmentState {
    environment: Environment,
    component_active: bool,
    dirty: bool,
}

impl EnvironmentState {
    pub fn new() -> Self {
        Self {
            environment: Environment::default(),
            component_active: false,
            dirty: false,
        }
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn environment_mut(&mut self) -> &mut Environment {
        self.dirty = true;
        &mut self.environment
    }

    pub fn set_environment(&mut self, environment: Environment, world: &mut World) {
        self.environment = environment;
        self.dirty = false;
        let snapshot = self.environment.clone();
        self.write_environment_to_components(world, &snapshot);
    }

    pub fn refresh(&mut self, world: &mut World) {
        if self.dirty {
            let snapshot = self.environment.clone();
            self.write_environment_to_components(world, &snapshot);
            self.dirty = false;
            return;
        }

        let component = Self::first_component(world);
        if let Some(component) = component {
            self.environment = component.to_environment();
            self.component_active = true;
        } else if self.component_active {
            self.environment = Environment::default();
            self.component_active = false;
        }
    }

    fn write_environment_to_components(&mut self, world: &mut World, environment: &Environment) {
        let entities = Self::environment_component_entities(world);
        self.component_active = !entities.is_empty();
        if !self.component_active {
            return;
        }

        let component = EnvironmentComponent::from_environment(environment);
        for entity in entities {
            if let Ok(mut existing) = world.get::<&mut EnvironmentComponent>(entity) {
                *existing = component.clone();
            }
        }
    }

    fn environment_component_entities(world: &World) -> Vec<Entity> {
        world
            .query::<&EnvironmentComponent>()
            .iter()
            .map(|(entity, _)| entity)
            .collect()
    }

    fn first_component(world: &World) -> Option<EnvironmentComponent> {
        world
            .query::<&EnvironmentComponent>()
            .iter()
            .next()
            .map(|(_, component)| component.clone())
    }
}

impl Default for EnvironmentState {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks editor gizmo configuration and GPU resources used to draw gizmos.
#[derive(Clone)]
pub struct GizmoState {
    gizmo_resources: Option<gizmos::GizmoResources>,
    transform_gizmo_resources: Option<transform_gizmos::TransformGizmoResources>,
    mode: TransformGizmoMode,
    space: TransformGizmoSpace,
    hover: Option<TransformGizmoHandle>,
}

impl GizmoState {
    pub fn new() -> Self {
        Self {
            gizmo_resources: None,
            transform_gizmo_resources: None,
            mode: TransformGizmoMode::Translate,
            space: TransformGizmoSpace::Local,
            hover: None,
        }
    }

    pub fn mode(&self) -> TransformGizmoMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: TransformGizmoMode) {
        self.mode = mode;
    }

    pub fn space(&self) -> TransformGizmoSpace {
        self.space
    }

    pub fn set_space(&mut self, space: TransformGizmoSpace) {
        self.space = space;
    }

    pub fn hover(&self) -> Option<TransformGizmoHandle> {
        self.hover
    }

    pub fn set_hover(&mut self, hover: Option<TransformGizmoHandle>) {
        self.hover = hover;
    }

    pub(crate) fn gizmo_resources(&self) -> Option<gizmos::GizmoResources> {
        self.gizmo_resources
    }

    pub(crate) fn set_gizmo_resources(&mut self, resources: Option<gizmos::GizmoResources>) {
        self.gizmo_resources = resources;
    }

    pub(crate) fn transform_gizmo_resources(
        &self,
    ) -> Option<transform_gizmos::TransformGizmoResources> {
        self.transform_gizmo_resources
    }

    pub(crate) fn set_transform_gizmo_resources(
        &mut self,
        resources: Option<transform_gizmos::TransformGizmoResources>,
    ) {
        self.transform_gizmo_resources = resources;
    }

    pub(crate) fn ensure_gizmo_resources(
        &mut self,
        renderer: &mut Renderer,
        assets: &mut Assets,
    ) -> gizmos::GizmoResources {
        if self.gizmo_resources.is_none() {
            let resources = gizmos::create_resources(renderer, assets);
            self.gizmo_resources = Some(resources);
        }
        self.gizmo_resources
            .expect("gizmo resources must be initialised")
    }

    pub(crate) fn ensure_transform_gizmo_resources(
        &mut self,
        renderer: &mut Renderer,
        assets: &mut Assets,
    ) -> transform_gizmos::TransformGizmoResources {
        if self.transform_gizmo_resources.is_none() {
            let resources = transform_gizmos::create_resources(renderer, assets);
            self.transform_gizmo_resources = Some(resources);
        }
        self.transform_gizmo_resources
            .expect("transform gizmo resources must be initialised")
    }

    pub(crate) fn clear_resources(&mut self) {
        self.gizmo_resources = None;
        self.transform_gizmo_resources = None;
    }
}

impl Default for GizmoState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::components::EnvironmentComponent;

    #[test]
    fn environment_syncs_components() {
        let mut world = World::new();
        let entity = world.spawn((EnvironmentComponent::default(),));
        let mut state = EnvironmentState::new();
        let mut environment = Environment::default();
        environment.set_ambient_intensity(4.2);

        state.set_environment(environment.clone(), &mut world);

        let component = world.get::<&EnvironmentComponent>(entity).unwrap().clone();
        assert_eq!(component.to_environment().ambient_intensity(), 4.2);
        drop(component);

        // Mutate the component directly and ensure the state pulls the data back.
        world
            .get::<&mut EnvironmentComponent>(entity)
            .unwrap()
            .ambient_intensity = 6.0;
        state.refresh(&mut world);
        assert_eq!(state.environment().ambient_intensity(), 6.0);
    }

    #[test]
    fn gizmo_state_tracks_settings() {
        let mut state = GizmoState::new();
        assert_eq!(state.mode(), TransformGizmoMode::Translate);
        state.set_mode(TransformGizmoMode::Rotate);
        assert_eq!(state.mode(), TransformGizmoMode::Rotate);

        assert_eq!(state.space(), TransformGizmoSpace::Local);
        state.set_space(TransformGizmoSpace::World);
        assert_eq!(state.space(), TransformGizmoSpace::World);

        assert!(state.hover().is_none());
        state.set_hover(Some(TransformGizmoHandle::RotateScreen));
        assert_eq!(state.hover(), Some(TransformGizmoHandle::RotateScreen));
    }
}
