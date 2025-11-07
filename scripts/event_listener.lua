-- Event Listener Script (Lua version)
-- Demonstrates event subscription and handling

function on_created(self_entity)
    log_info("Event listener initialized")

    -- Subscribe to events
    subscribe_event("periodic_update", "on_periodic_update")
    subscribe_event("key_pressed_event", "on_key_pressed")

    log_info("Subscribed to periodic_update and key_pressed_event")
end

-- Event handler for periodic_update events
function on_periodic_update(event_data)
    log_info(string.format("Received periodic update: %s", event_data.message))
    log_info(string.format("  Count: %.0f, Timestamp: %.2f", event_data.count, event_data.timestamp))
end

-- Event handler for key_pressed events
function on_key_pressed(event_data)
    log_info(string.format("Received key press event: %s", event_data.message))
    log_info(string.format("  Key: %s, Entity: %d", event_data.key, event_data.entity))
end

function update(self_entity, dt)
    -- Check if user wants to unsubscribe
    if is_key_just_pressed("U") then
        unsubscribe_event("periodic_update")
        log_info("Unsubscribed from periodic_update event")
    end

    -- Check if user wants to resubscribe
    if is_key_just_pressed("R") then
        subscribe_event("periodic_update", "on_periodic_update")
        log_info("Resubscribed to periodic_update event")
    end
end
