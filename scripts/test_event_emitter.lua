-- Test script for emitting custom events
-- This script will emit various events to test the event system

function on_created(self_entity)
    log_info("=== Event Emitter Test: Initialized ===")

    set_state("frame_count", 0)
    set_translation(self_entity, -3.0, 0.0, 0.0)
end

function update(self_entity, dt)
    local frame_count = get_state("frame_count", 0)
    frame_count = frame_count + 1
    set_state("frame_count", frame_count)

    -- Emit a simple event every 60 frames
    if frame_count % 60 == 0 then
        log_info(string.format("Event Emitter: Emitting 'test_tick' event at frame %d", frame_count))
        emit_event("test_tick", {
            frame = frame_count,
            sender = self_entity,
            timestamp = frame_count * dt
        })
    end

    -- Emit a position update event every 30 frames
    if frame_count % 30 == 0 then
        local pos = get_world_translation(self_entity)
        if pos then
            log_info("Event Emitter: Emitting 'position_update' event")
            emit_event("position_update", {
                entity = self_entity,
                x = pos.x,
                y = pos.y,
                z = pos.z
            })
        end
    end

    -- Emit a start event once at frame 10
    if frame_count == 10 then
        log_info("Event Emitter: Emitting 'test_start' event")
        emit_event("test_start", {
            message = "Event system test started",
            emitter = self_entity
        })
    end

    -- Emit a milestone event at frame 120
    if frame_count == 120 then
        log_info("Event Emitter: Emitting 'milestone_reached' event")
        emit_event("milestone_reached", {
            milestone = "2 seconds elapsed",
            frame = frame_count
        })
    end

    -- Visual feedback: bounce up and down
    local time = frame_count * dt
    local y_offset = math.sin(time * 3.0) * 0.3
    set_translation(self_entity, -3.0, y_offset, 0.0)
end
