-- Test script for verifying lifecycle hooks work correctly
-- This script should be attached to an entity in the editor

function on_created(self_entity)
    log_info("=== Lifecycle Test: on_created() called ===")

    -- Initialize state
    set_state("created_count", 1)
    set_state("update_count", 0)
    set_state("last_update_time", 0.0)

    -- Set initial position
    set_translation(self_entity, 0.0, 0.0, 0.0)

    log_info("Lifecycle Test: State initialized")
end

function update(self_entity, dt)
    -- Check if on_created was called
    local created_count = get_state("created_count", 0)
    if created_count == 0 then
        log_error("Lifecycle Test FAILED: update() called before on_created()")
        return
    end

    -- Increment update counter
    local update_count = get_state("update_count", 0)
    update_count = update_count + 1
    set_state("update_count", update_count)

    -- Track time
    local last_time = get_state("last_update_time", 0.0)
    local new_time = last_time + dt
    set_state("last_update_time", new_time)

    -- Log progress every 60 frames (~1 second at 60fps)
    if update_count % 60 == 0 then
        log_info(string.format("Lifecycle Test: Frame %d, Time: %.2fs, dt: %.4fs",
                               update_count, new_time, dt))
    end

    -- Animate the entity (rotate slowly)
    local angle = new_time * 0.5  -- 0.5 radians per second
    set_rotation(self_entity, 0.0, angle, 0.0)

    -- Test state persistence
    if update_count == 10 then
        log_info("Lifecycle Test: Verifying state persists across frames")
        set_state("test_value", {x = 10, y = 20, z = 30})
    end

    if update_count == 20 then
        local test_value = get_state("test_value", {x = 0, y = 0, z = 0})
        if test_value.x == 10 and test_value.y == 20 and test_value.z == 30 then
            log_info("Lifecycle Test PASSED: State persists correctly across frames")
        else
            log_error("Lifecycle Test FAILED: State not persisting correctly")
        end
    end
end

-- Note: on_ui() is not yet implemented for Lua scripts
-- function on_ui(self_entity, ui)
--     log_info("Lifecycle Test: on_ui() called")
-- end
