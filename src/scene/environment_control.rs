use super::state::EnvironmentState;
use crate::environment::Environment;
use hecs::World;

/// Manages the active environment for a [`Scene`](super::Scene) and keeps the
/// ECS world in sync with the canonical state.
#[derive(Debug)]
pub(crate) struct SceneEnvironment {
    state: EnvironmentState,
}

impl SceneEnvironment {
    pub(crate) fn new() -> Self {
        Self {
            state: EnvironmentState::new(),
        }
    }

    pub(crate) fn environment(&self) -> &Environment {
        self.state.environment()
    }

    pub(crate) fn environment_mut(&mut self) -> &mut Environment {
        self.state.environment_mut()
    }

    pub(crate) fn set_environment(&mut self, environment: Environment, world: &mut World) {
        self.state.set_environment(environment, world);
    }

    pub(crate) fn refresh(&mut self, world: &mut World) {
        self.state.refresh(world);
    }
}

impl Default for SceneEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SceneEnvironment;
    use crate::environment::Environment;
    use crate::scene::components::EnvironmentComponent;
    use hecs::World;

    #[test]
    fn refresh_propagates_dirty_environment_to_components() {
        let mut controller = SceneEnvironment::new();
        let mut world = World::new();
        let entity = world.spawn((EnvironmentComponent::default(),));

        controller.environment_mut().set_ambient_intensity(2.5);
        controller.refresh(&mut world);

        {
            let component = world.get::<&EnvironmentComponent>(entity).unwrap();
            assert!((component.ambient_intensity - 2.5).abs() < f32::EPSILON);
        }

        // Overwrite the component directly to simulate external changes and
        // ensure the controller picks them up on the next refresh.
        {
            let mut component = world.get::<&mut EnvironmentComponent>(entity).unwrap();
            component.ambient_intensity = 4.0;
        }

        controller.refresh(&mut world);
        assert!((controller.environment().ambient_intensity() - 4.0_f32).abs() < f32::EPSILON);
    }

    #[test]
    fn set_environment_replaces_world_state() {
        let mut controller = SceneEnvironment::new();
        let mut world = World::new();
        let entity = world.spawn((EnvironmentComponent::default(),));

        let mut environment = Environment::default();
        environment.set_ambient_intensity(6.25);
        controller.set_environment(environment.clone(), &mut world);

        let component = world.get::<&EnvironmentComponent>(entity).unwrap();
        assert!((component.ambient_intensity - 6.25).abs() < f32::EPSILON);
        assert!((controller.environment().ambient_intensity() - 6.25).abs() < f32::EPSILON);
    }
}
