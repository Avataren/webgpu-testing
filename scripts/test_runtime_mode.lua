-- Test script that should ONLY run in runtime/game mode (no @editor annotation)
-- This script will NOT run in editor mode, only when the game is playing

function on_created(self_entity)
    log_info("=== Runtime Mode Test: on_created() ===")
    log_warn("This script should ONLY run in RUNTIME/GAME mode")

    set_state("mode", "runtime")
    set_state("frame_count", 0)

    -- Visual indicator: position to the right
    set_translation(self_entity, 2.0, 0.0, 0.0)
end

function update(self_entity, dt)
    local frame_count = get_state("frame_count", 0)
    frame_count = frame_count + 1
    set_state("frame_count", frame_count)

    -- Log every 60 frames
    if frame_count % 60 == 0 then
        log_info(string.format("Runtime Mode Test: Frame %d (should only run in game mode)", frame_count))
    end

    -- Spin faster than editor mode script
    local time = frame_count * dt
    local angle = time * 2.0  -- 2 radians per second
    set_rotation(self_entity, 0.0, angle, 0.0)
end
