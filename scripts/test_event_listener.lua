-- Test script for receiving custom events
-- This script subscribes to events emitted by test_event_emitter.lua

function on_created(self_entity)
    log_info("=== Event Listener Test: Initialized ===")

    -- Subscribe to various events
    log_info("Event Listener: Subscribing to 'test_tick' event")
    subscribe_event("test_tick")

    log_info("Event Listener: Subscribing to 'position_update' event")
    subscribe_event("position_update")

    log_info("Event Listener: Subscribing to 'test_start' event")
    subscribe_event("test_start")

    log_info("Event Listener: Subscribing to 'milestone_reached' event")
    subscribe_event("milestone_reached")

    set_state("tick_count", 0)
    set_state("position_updates", 0)
    set_state("received_start", false)
    set_state("received_milestone", false)

    set_translation(self_entity, 3.0, 0.0, 0.0)
end

function update(self_entity, dt)
    -- Check for received events using get_state (events stored there by event system)
    -- Note: The actual event reception mechanism depends on how the Lua event API works
    -- This is a placeholder for the expected behavior

    local tick_count = get_state("tick_count", 0)
    local received_start = get_state("received_start", false)
    local received_milestone = get_state("received_milestone", false)

    -- Visual feedback: rotate based on number of ticks received
    local angle = tick_count * 0.1
    set_rotation(self_entity, 0.0, angle, 0.0)

    -- Summary log every 120 frames
    local frame_count = get_state("frame_count", 0)
    frame_count = frame_count + 1
    set_state("frame_count", frame_count)

    if frame_count % 120 == 0 then
        local pos_updates = get_state("position_updates", 0)
        log_info(string.format("Event Listener Summary: Ticks=%d, PosUpdates=%d, Start=%s, Milestone=%s",
                               tick_count, pos_updates, tostring(received_start), tostring(received_milestone)))
    end
end

-- Event handler functions (if the Lua event API supports per-event callbacks)
-- Note: These may need to be adjusted based on the actual implementation

function on_test_tick(event_data)
    log_info(string.format("Event Listener: Received 'test_tick' at frame %d", event_data.frame or 0))
    local tick_count = get_state("tick_count", 0)
    set_state("tick_count", tick_count + 1)
end

function on_position_update(event_data)
    log_info(string.format("Event Listener: Received position (%.2f, %.2f, %.2f)",
                          event_data.x or 0, event_data.y or 0, event_data.z or 0))
    local pos_updates = get_state("position_updates", 0)
    set_state("position_updates", pos_updates + 1)
end

function on_test_start(event_data)
    log_info("Event Listener: Received 'test_start' event: " .. (event_data.message or ""))
    set_state("received_start", true)
end

function on_milestone_reached(event_data)
    log_info("Event Listener: Received 'milestone_reached' event: " .. (event_data.milestone or ""))
    set_state("received_milestone", true)
end
