-- Test script for verifying script isolation
-- Attach this to MULTIPLE entities to verify each has isolated state
-- Each entity should maintain its own counter without interference

function on_created(self_entity)
    log_info(string.format("=== Isolation Test: Entity %d created ===", self_entity))

    -- Each entity gets its own unique starting position
    -- We'll use the entity ID to offset the position
    local offset = (self_entity % 10) * 0.5  -- Spread entities out
    set_translation(self_entity, offset, 0.0, 0.0)

    -- Initialize state unique to this entity
    set_state("entity_id", self_entity)
    set_state("counter", 0)
    set_state("spin_speed", 1.0 + (self_entity % 5) * 0.2)  -- Different speeds
    set_state("created_at_frame", 0)

    -- Test: Try to access a global variable
    -- Each script instance should have its own isolated environment
    if my_global_counter == nil then
        my_global_counter = 0
        log_info(string.format("Entity %d: Initialized my_global_counter to 0", self_entity))
    else
        log_warn(string.format("Entity %d: my_global_counter already exists = %d (ISOLATION FAILURE!)",
                               self_entity, my_global_counter))
    end
end

function update(self_entity, dt)
    -- Increment this entity's counter
    local counter = get_state("counter", 0)
    counter = counter + 1
    set_state("counter", counter)

    -- Also increment the global (should be isolated per script instance)
    my_global_counter = my_global_counter + 1

    -- Log every 60 frames
    if counter % 60 == 0 then
        local entity_id = get_state("entity_id", -1)
        log_info(string.format("Entity %d: Counter = %d, Global = %d",
                               entity_id, counter, my_global_counter))

        -- Verification: counters should match (proving isolation)
        if counter ~= my_global_counter then
            log_error(string.format("Entity %d: ISOLATION FAILURE! State counter (%d) != global counter (%d)",
                                   entity_id, counter, my_global_counter))
        end
    end

    -- Rotate at entity-specific speed
    local spin_speed = get_state("spin_speed", 1.0)
    local angle = counter * dt * spin_speed
    set_rotation(self_entity, 0.0, angle, 0.0)

    -- Test state isolation: each entity should have different values
    if counter == 100 then
        local entity_id = get_state("entity_id", -1)
        log_info(string.format("Entity %d: Isolation Test checkpoint - Counter: %d, Speed: %.2f",
                               entity_id, counter, spin_speed))
    end
end

-- Helper function to demonstrate function isolation
function get_entity_info()
    local entity_id = get_state("entity_id", -1)
    local counter = get_state("counter", 0)
    return {id = entity_id, count = counter}
end
