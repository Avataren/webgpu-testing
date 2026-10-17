//! The directional lights that follow the procedural sky (`DirectionalLight::follow_sky`):
//! the sun's light by day, the moon's by night.

use hecs::{Entity, World};

use crate::scene::components::{CanCastShadow, DirectionalLight, Name, TransformComponent};
use crate::scene::transform::Transform;

/// Make `light` follow the sky, or with None the world's first directional light (a new
/// shadow-casting "Sun" when it has none). Returns the light.
pub fn follow_sky(world: &mut World, light: Option<Entity>) -> Result<Entity, String> {
    let entity = match light {
        Some(entity) => entity,
        None => {
            let first = world
                .query::<&DirectionalLight>()
                .iter()
                .map(|(entity, _)| entity)
                .min_by_key(|entity| entity.id());
            match first {
                Some(entity) => entity,
                None => world.spawn((
                    Name::new("Sun"),
                    TransformComponent(Transform::IDENTITY),
                    DirectionalLight::default(),
                    CanCastShadow(true),
                )),
            }
        }
    };
    let mut component = world
        .get::<&mut DirectionalLight>(entity)
        .map_err(|_| format!("entity {} has no directional light", entity.to_bits().get()))?;
    component.follow_sky = true;
    Ok(entity)
}

/// No directional light follows the sky any more.
pub fn stop_following_sky(world: &mut World) {
    for (_, light) in world.query_mut::<&mut DirectionalLight>() {
        light.follow_sky = false;
    }
}

/// The directional lights that follow the sky.
pub fn sky_following_lights(world: &World) -> Vec<Entity> {
    world
        .query::<&DirectionalLight>()
        .iter()
        .filter(|(_, light)| light.follow_sky)
        .map(|(entity, _)| entity)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_light_follows_or_a_sun_appears() {
        let mut world = World::new();
        let sun = follow_sky(&mut world, None).unwrap();
        assert_eq!(world.get::<&Name>(sun).unwrap().0, "Sun");
        assert_eq!(sky_following_lights(&world), vec![sun]);
        // An existing light is taken rather than a second sun made.
        stop_following_sky(&mut world);
        assert!(sky_following_lights(&world).is_empty());
        assert_eq!(follow_sky(&mut world, None).unwrap(), sun);
        let lamp = world.spawn((Name::new("Lamp"),));
        assert!(follow_sky(&mut world, Some(lamp)).is_err());
    }
}
