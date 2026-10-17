//! The audio components of a saved entity (saved as its `audio_source` and
//! `audio_listener` fields).

use crate::audio::{AudioListener, AudioSource};

#[derive(Debug, Clone, Default)]
pub struct AudioData {
    pub audio_source: Option<AudioSource>,
    pub audio_listener: Option<AudioListener>,
}

impl AudioData {
    /// The audio components `entity` has.
    pub fn capture(world: &hecs::World, entity: hecs::Entity) -> Self {
        Self {
            audio_source: world.get::<&AudioSource>(entity).ok().map(|c| (*c).clone()),
            audio_listener: world
                .get::<&AudioListener>(entity)
                .ok()
                .map(|c| (*c).clone()),
        }
    }

    /// Add them to an entity being built.
    pub fn add_to(&self, builder: &mut hecs::EntityBuilder) {
        if let Some(component) = &self.audio_source {
            builder.add(component.clone());
        }
        if let Some(component) = &self.audio_listener {
            builder.add(component.clone());
        }
    }
}
