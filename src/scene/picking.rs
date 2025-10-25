use crate::scene::{components::EditorEntityId, Scene};
use hecs::Entity;

/// Attempts to resolve a pick value back to the originating entity in the
/// provided scene.
#[inline]
pub fn entity_for_pick_value(scene: &Scene, pick_value: u64) -> Option<Entity> {
    if pick_value == 0 {
        return None;
    }

    let world = scene.main_world();
    world
        .query::<&EditorEntityId>()
        .iter()
        .find_map(|(entity, editor_id)| {
            let pick_id = editor_id.pick_identifier();
            (pick_id == pick_value).then_some(entity)
        })
}
