-- Comprehensive integration test for Lua scripting
-- Tests multiple API categories and complex interactions

function on_created(self_entity)
    log_info("=== Comprehensive Integration Test Started ===")

    -- Test State Management API
    set_state("test_phase", "initialization")
    set_f64("pi_value", 3.14159)
    set_bool("is_active", true)
    set_string("test_name", "Comprehensive Test")

    -- Test Entity Management API
    set_name(self_entity, "TestEntity_Main")
    local name = get_name(self_entity)
    log_info("Entity name: " .. (name or "unknown"))

    -- Test Transform API
    set_translation(self_entity, 0.0, 0.0, 0.0)
    set_scale(self_entity, 1.0, 1.0, 1.0)

    -- Initialize test tracking
    set_state("frame_count", 0)
    set_state("test_results", {
        logging = "pending",
        state = "pending",
        transform = "pending",
        entity = "pending",
        input = "pending",
        events = "pending"
    })

    log_info("Comprehensive Test: Initialization complete")
end

function update(self_entity, dt)
    local frame_count = get_state("frame_count", 0)
    frame_count = frame_count + 1
    set_state("frame_count", frame_count)

    local test_phase = get_state("test_phase", "initialization")

    -- Phase 1: Test State Management (frames 1-60)
    if frame_count >= 1 and frame_count < 60 then
        if test_phase ~= "state_test" then
            log_info("Comprehensive Test: Phase 1 - State Management")
            set_state("test_phase", "state_test")
        end

        -- Test various state operations
        if frame_count == 10 then
            local pi = get_f64("pi_value", 0.0)
            local is_active = get_bool("is_active", false)
            local name = get_string("test_name", "")

            local success = (math.abs(pi - 3.14159) < 0.0001) and is_active and (name == "Comprehensive Test")
            if success then
                log_info("✓ State Management API: PASSED")
                local results = get_state("test_results", {})
                results.state = "passed"
                set_state("test_results", results)
            else
                log_error("✗ State Management API: FAILED")
            end
        end
    end

    -- Phase 2: Test Transform API (frames 60-120)
    if frame_count >= 60 and frame_count < 120 then
        if test_phase ~= "transform_test" then
            log_info("Comprehensive Test: Phase 2 - Transform API")
            set_state("test_phase", "transform_test")
        end

        -- Animate to test transforms
        local t = (frame_count - 60) / 60.0
        local angle = t * 3.14159 * 2.0  -- One full rotation
        set_rotation(self_entity, 0.0, angle, 0.0)

        local y = math.sin(t * 3.14159 * 2.0) * 0.5
        translate(self_entity, 0.0, y * 0.01, 0.0)

        if frame_count == 90 then
            local pos = get_world_translation(self_entity)
            local rot = get_world_rotation(self_entity)

            if pos and rot then
                log_info(string.format("✓ Transform API: PASSED (pos: %.2f,%.2f,%.2f)",
                                      pos.x, pos.y, pos.z))
                local results = get_state("test_results", {})
                results.transform = "passed"
                set_state("test_results", results)
            else
                log_error("✗ Transform API: FAILED")
            end
        end
    end

    -- Phase 3: Test Entity Management (frames 120-180)
    if frame_count >= 120 and frame_count < 180 then
        if test_phase ~= "entity_test" then
            log_info("Comprehensive Test: Phase 3 - Entity Management")
            set_state("test_phase", "entity_test")
        end

        if frame_count == 130 then
            -- Test spawning an entity
            local new_entity = spawn_entity("TestChild")
            if new_entity ~= 0 then
                set_state("spawned_entity", new_entity)
                set_translation(new_entity, 1.0, 0.0, 0.0)
                log_info(string.format("✓ Entity Management: Spawned entity %d", new_entity))

                local results = get_state("test_results", {})
                results.entity = "passed"
                set_state("test_results", results)
            else
                log_error("✗ Entity Management: Failed to spawn entity")
            end
        end

        -- Clean up spawned entity
        if frame_count == 170 then
            local spawned = get_state("spawned_entity", 0)
            if spawned ~= 0 then
                despawn_entity(spawned)
                log_info("Comprehensive Test: Cleaned up spawned entity")
            end
        end
    end

    -- Phase 4: Test Input API (frames 180-240)
    if frame_count >= 180 and frame_count < 240 then
        if test_phase ~= "input_test" then
            log_info("Comprehensive Test: Phase 4 - Input API (Press W/A/S/D or Space)")
            set_state("test_phase", "input_test")
        end

        -- Test input API (passive test - logs if keys are pressed)
        if is_key_pressed("w") then
            log_info("✓ Input API: W key detected")
            translate(self_entity, 0.0, 0.1, 0.0)
        end

        if is_key_just_pressed("space") then
            log_info("✓ Input API: Space key just pressed")
            local results = get_state("test_results", {})
            results.input = "passed"
            set_state("test_results", results)
        end

        local mouse_pos = get_mouse_position()
        if mouse_pos and frame_count == 200 then
            log_info(string.format("✓ Input API: Mouse position (%.0f, %.0f)", mouse_pos.x, mouse_pos.y))
        end
    end

    -- Phase 5: Test Events (frames 240-300)
    if frame_count >= 240 and frame_count < 300 then
        if test_phase ~= "event_test" then
            log_info("Comprehensive Test: Phase 5 - Event System")
            set_state("test_phase", "event_test")
            subscribe_event("comprehensive_test_event")
        end

        if frame_count == 250 then
            emit_event("comprehensive_test_event", {
                source = "comprehensive_test",
                frame = frame_count,
                message = "Test event emission"
            })
            log_info("✓ Event API: Emitted test event")

            local results = get_state("test_results", {})
            results.events = "passed"
            set_state("test_results", results)
        end
    end

    -- Phase 6: Final Report (frame 300+)
    if frame_count == 300 then
        log_info("=== Comprehensive Test: FINAL REPORT ===")

        local results = get_state("test_results", {})
        local all_passed = true

        for category, status in pairs(results) do
            local symbol = (status == "passed") and "✓" or "✗"
            log_info(string.format("%s %s: %s", symbol, category, status))
            if status ~= "passed" then
                all_passed = false
            end
        end

        if all_passed then
            log_info("=== Comprehensive Test: ALL TESTS PASSED ===")
        else
            log_warn("=== Comprehensive Test: SOME TESTS INCOMPLETE ===")
        end
    end

    -- Continue gentle animation after tests complete
    if frame_count > 300 then
        local time = frame_count * dt
        local angle = time * 0.5
        set_rotation(self_entity, 0.0, angle, 0.0)
    end
end
