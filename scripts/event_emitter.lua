-- Event Emitter Script (Lua version)
-- Demonstrates event emission with Lua scripting

function on_created(self_entity)
    log_info("Event emitter initialized")
    set_f64("emit_timer", 0.0)
    set_f64("event_count", 0.0)
end

function update(self_entity, dt)
    -- Accumulate time
    local timer = get_f64("emit_timer", 0.0) + dt
    set_f64("emit_timer", timer)

    -- Emit event every 2 seconds
    if timer >= 2.0 then
        local count = get_f64("event_count", 0.0) + 1.0
        set_f64("event_count", count)
        set_f64("emit_timer", 0.0)

        -- Emit custom event with data
        emit_event("periodic_update", {
            count = count,
            timestamp = timer,
            message = string.format("Event #%.0f from Lua", count)
        })

        log_info(string.format("Emitted periodic_update event #%.0f", count))
    end

    -- Check for key press to emit immediate event
    if is_key_just_pressed("E") then
        emit_event("key_pressed_event", {
            key = "E",
            entity = self_entity,
            message = "E key pressed!"
        })
        log_info("Emitted key_pressed_event")
    end
end
