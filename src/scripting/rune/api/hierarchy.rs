use hecs::Entity;
use rune::runtime::VmResult;

use crate::scene::components::{Children, Parent};
use crate::scripting::rune::commands::entity_bits;
use crate::scripting::rune::guards::{with_active_world, ACTIVE_COMMANDS};

/// Set the parent of an entity.
///
/// Pass None to unparent the entity.
///
/// # Example
/// ```rune
/// let parent = find_entity_by_name("ParentObject");
/// // Note: Can't easily unwrap Option in Rune yet
/// set_parent(child_entity, parent);
/// ```
#[rune::function]
pub(crate) fn set_parent(entity_bits: i64, parent_bits: Option<i64>) -> VmResult<()> {
    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.set_parent(entity_bits, parent_bits))
    })
}

/// Get the parent of an entity.
///
/// Returns the entity handle of the parent, or None if no parent.
///
/// # Example
/// ```rune
/// let parent = get_parent(entity);
/// if parent != None {
///     log_info("Has parent");
/// }
/// ```
#[rune::function]
pub(crate) fn get_parent(entity_handle: i64) -> VmResult<Option<i64>> {
    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_handle as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        if let Ok(parent) = world.get::<&Parent>(entity) {
            return VmResult::Ok(Some(entity_bits(parent.0)));
        }

        VmResult::Ok(None)
    })
}

/// Get the children of an entity.
///
/// Returns an array of entity handles.
///
/// # Example
/// ```rune
/// let children = get_children(entity);
/// if children != None {
///     log_info("Has children");
/// }
/// ```
#[rune::function]
pub(crate) fn get_children(entity_handle: i64) -> VmResult<Option<rune::alloc::Vec<i64>>> {
    with_active_world(|world| {
        let entity = match Entity::from_bits(entity_handle as u64) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        if let Ok(children) = world.get::<&Children>(entity) {
            let mut vec = rune::alloc::Vec::new();
            for &child in &children.0 {
                if let Err(e) = vec.try_push(entity_bits(child)) {
                    return VmResult::Err(e.into());
                }
            }
            return VmResult::Ok(Some(vec));
        }

        VmResult::Ok(None)
    })
}
