-- @editor
-- Test script that should ONLY run in editor mode
-- This script will log messages to verify it's running in the correct mode

function on_created(self_entity)
    log_info("=== Editor Mode Test: on_created() ===")
    log_warn("This script should ONLY run in EDITOR mode")

    set_state("mode", "editor")
    set_state("frame_count", 0)

    -- Visual indicator: tint entity green for editor mode
    set_translation(self_entity, -2.0, 0.0, 0.0)
end

function update(self_entity, dt)
    local frame_count = get_state("frame_count", 0)
    frame_count = frame_count + 1
    set_state("frame_count", frame_count)

    -- Log every 60 frames
    if frame_count % 60 == 0 then
        log_info(string.format("Editor Mode Test: Frame %d (should only run in editor)", frame_count))
    end

    -- Gentle oscillation to show it's active
    local time = frame_count * dt
    local y_offset = math.sin(time * 2.0) * 0.5
    set_translation(self_entity, -2.0, y_offset, 0.0)
end
