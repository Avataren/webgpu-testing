use rune::runtime::VmResult;
use rune::Value;

use crate::scripting::rune::guards::{get_active_entity, with_active_event_queue, ACTIVE_COMMANDS};
use crate::scripting::rune::types::ScriptEvent;

/// Emit an event that can be received by subscribed scripts.
///
/// # Arguments
/// * `event_name` - The name of the event to emit
/// * `data` - The event data (can be any Rune value: string, number, object, etc.)
///
/// Events are queued during script execution and dispatched after all scripts
/// have finished updating. Subscribed scripts will have their registered callback
/// function called with the event data.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_pressed("Space") {
///         // Emit an event when space is pressed
///         emit_event("player_jumped", #{
///             entity: self_entity,
///             height: 5.0,
///             timestamp: 12345.0
///         });
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn emit_event(event_name: String, data: Value) -> VmResult<()> {
    with_active_event_queue(|queue| {
        queue.push(ScriptEvent {
            name: event_name,
            data,
        });
        VmResult::Ok(())
    })
}

/// Subscribe to an event with a callback function.
///
/// # Arguments
/// * `event_name` - The name of the event to subscribe to
/// * `callback_name` - The name of the function to call when the event is received
///
/// The callback function must accept one parameter: the event data.
/// When an event with the matching name is emitted, the callback will be called
/// with the event data.
///
/// # Example
/// ```rune
/// pub fn on_created(self_entity) {
///     // Subscribe to player_jumped event
///     subscribe_event("player_jumped", "on_player_jumped");
/// }
///
/// pub fn on_player_jumped(event_data) {
///     log_info(`Player jumped! Height: ${event_data.height}`);
/// }
/// ```
#[rune::function]
pub(crate) fn subscribe_event(event_name: String, callback_name: String) -> VmResult<()> {
    // Get the current entity from the active context
    let entity_bits = match get_active_entity() {
        VmResult::Ok(bits) => bits,
        VmResult::Err(err) => return VmResult::Err(err),
    };

    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut().with(|commands| {
            commands.subscribe_event(entity_bits, event_name, callback_name)
        })
    })
}

/// Unsubscribe from an event.
///
/// # Arguments
/// * `event_name` - The name of the event to unsubscribe from
///
/// Removes the subscription for the current entity from the specified event.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_pressed("U") {
///         unsubscribe_event("player_jumped");
///         log_info("Unsubscribed from player_jumped event");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn unsubscribe_event(event_name: String) -> VmResult<()> {
    // Get the current entity from the active context
    let entity_bits = match get_active_entity() {
        VmResult::Ok(bits) => bits,
        VmResult::Err(err) => return VmResult::Err(err),
    };

    ACTIVE_COMMANDS.with(|cell| {
        cell.borrow_mut()
            .with(|commands| commands.unsubscribe_event(entity_bits, event_name))
    })
}
