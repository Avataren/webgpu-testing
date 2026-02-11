-- Test script for hot-reload with state preservation
-- Instructions:
-- 1. Attach this script to an entity
-- 2. Let it run for a few seconds to accumulate state
-- 3. Trigger a script reload (reset_script_runtime)
-- 4. Verify the counter continues from where it left off

function on_created(self_entity)
    log_info("=== Hot-Reload Test: on_created() called ===")

    -- Initialize default state
    -- If hot-reload works correctly, these defaults will be overwritten by restored state
    local existing_counter = get_state("reload_counter", -1)

    if existing_counter == -1 then
        -- First time creation
        log_info("Hot-Reload Test: First-time initialization")
        set_state("reload_counter", 0)
        set_state("reload_count", 0)
        set_state("total_frames", 0)
        set_state("creation_time", 0.0)
    else
        -- This is a reload!
        log_warn(string.format("Hot-Reload Test: RELOAD DETECTED! Counter was at %d", existing_counter))
        local reload_count = get_state("reload_count", 0)
        reload_count = reload_count + 1
        set_state("reload_count", reload_count)
        log_info(string.format("Hot-Reload Test: This is reload #%d", reload_count))
    end

    set_translation(self_entity, 0.0, 2.0, 0.0)
end

function update(self_entity, dt)
    -- Increment frame counter
    local counter = get_state("reload_counter", 0)
    counter = counter + 1
    set_state("reload_counter", counter)

    local total_frames = get_state("total_frames", 0)
    total_frames = total_frames + 1
    set_state("total_frames", total_frames)

    local time = get_state("creation_time", 0.0)
    time = time + dt
    set_state("creation_time", time)

    -- Log progress every 60 frames
    if counter % 60 == 0 then
        local reload_count = get_state("reload_count", 0)
        log_info(string.format("Hot-Reload Test: Counter=%d, Reloads=%d, Time=%.2fs",
                               counter, reload_count, time))
    end

    -- Store complex state to test deep preservation
    if counter % 30 == 0 then
        set_state("complex_data", {
            nested = {
                value = counter,
                timestamp = time,
                info = "This should survive reload"
            },
            array = {1, 2, 3, counter}
        })
    end

    -- Visual feedback: height indicates counter value
    local y_pos = 2.0 + (counter % 100) * 0.01
    set_translation(self_entity, 0.0, y_pos, 0.0)

    -- Rotate to show it's running
    local angle = time * 1.0
    set_rotation(self_entity, 0.0, angle, 0.0)

    -- Test state preservation at specific intervals
    if counter == 10 then
        log_info("Hot-Reload Test: Checkpoint 1 - Ready for reload test")
    end

    if counter == 180 then
        log_info("Hot-Reload Test: Checkpoint 2 - If you see this, state persisted for 3 seconds!")
        local complex = get_state("complex_data", {})
        if complex.nested and complex.nested.value then
            log_info(string.format("Hot-Reload Test: Complex state preserved: %d", complex.nested.value))
        end
    end
end
