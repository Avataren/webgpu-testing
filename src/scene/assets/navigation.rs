//! The navigation components of a saved entity (saved as its `nav_*` fields). Empty
//! when the engine is built without the `navigation` feature (such builds drop them).

#[derive(Debug, Clone, Default)]
pub struct NavigationData {
    #[cfg(feature = "navigation")]
    pub nav_mesh_surface: Option<crate::navigation::NavMeshSurface>,
    #[cfg(feature = "navigation")]
    pub nav_agent: Option<crate::navigation::NavAgent>,
    #[cfg(feature = "navigation")]
    pub nav_obstacle: Option<crate::navigation::NavObstacle>,
    #[cfg(feature = "navigation")]
    pub nav_modifier: Option<crate::navigation::NavModifier>,
    #[cfg(feature = "navigation")]
    pub nav_link: Option<crate::navigation::NavLink>,
}

impl NavigationData {
    /// The navigation components `entity` has.
    #[allow(unused_variables)]
    pub fn capture(world: &hecs::World, entity: hecs::Entity) -> Self {
        #[cfg(feature = "navigation")]
        {
            fn get<T: hecs::Component + Clone>(
                world: &hecs::World,
                entity: hecs::Entity,
            ) -> Option<T> {
                world.get::<&T>(entity).ok().map(|c| (*c).clone())
            }
            Self {
                nav_mesh_surface: get(world, entity),
                nav_agent: get(world, entity),
                nav_obstacle: get(world, entity),
                nav_modifier: get(world, entity),
                nav_link: get(world, entity),
            }
        }
        #[cfg(not(feature = "navigation"))]
        Self::default()
    }

    /// Add them to an entity being built.
    #[allow(unused_variables)]
    pub fn add_to(&self, builder: &mut hecs::EntityBuilder) {
        #[cfg(feature = "navigation")]
        {
            if let Some(component) = &self.nav_mesh_surface {
                builder.add(component.clone());
            }
            if let Some(component) = &self.nav_agent {
                builder.add(component.clone());
            }
            if let Some(component) = &self.nav_obstacle {
                builder.add(component.clone());
            }
            if let Some(component) = &self.nav_modifier {
                builder.add(component.clone());
            }
            if let Some(component) = &self.nav_link {
                builder.add(component.clone());
            }
        }
    }
}
