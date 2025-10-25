use crate::scene::{components::EditorEntityId, Scene};
use hecs::Entity;

/// Hash multiplier used when combining 64-bit pick identifiers into a
/// single 32-bit value for GPU readback.
pub const PICK_HASH_MULTIPLIER: u32 = 0x9E37_79B1;

/// Encodes a 64-bit pick identifier into the 32-bit value emitted by the
/// renderer's pick attachment.
#[inline]
pub fn encode_pick_value(pick_id: u64) -> u32 {
    let lower = pick_id as u32;
    let upper = (pick_id >> 32) as u32;
    lower ^ upper.wrapping_mul(PICK_HASH_MULTIPLIER)
}

/// Attempts to resolve a pick value back to the originating entity in the
/// provided scene.
#[inline]
pub fn entity_for_pick_value(scene: &Scene, pick_value: u32) -> Option<Entity> {
    if pick_value == 0 {
        return None;
    }

    let world = scene.main_world();
    world
        .query::<&EditorEntityId>()
        .iter()
        .find_map(|(entity, editor_id)| {
            let pick_id = editor_id.pick_identifier();
            (encode_pick_value(pick_id) == pick_value).then_some(entity)
        })
}
