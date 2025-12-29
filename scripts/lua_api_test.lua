-- Comprehensive Lua API Test Script
-- Tests all major API categories for the Lua scripting integration

function on_created(self_entity)
    log_info("=== Lua API Test Script Started ===")

    -- Test 1: Logging API
    log_debug("Debug message test")
    log_info("Info message test")
    log_warn("Warning message test")
    log_error("Error message test")

    -- Test 2: State Management API
    log_info("Testing state management...")
    set_state("test_value", {x = 10, y = 20})
    set_f64("test_number", 3.14159)
    set_bool("test_bool", true)
    set_string("test_string", "Hello from Lua!")

    -- Initialize test state
    set_state("test_initialized", true)

    log_info("=== API Test Initialization Complete ===")
end

function update(self_entity, dt)
    -- Only run tests once
    local initialized = get_state("test_initialized", false)
    if not initialized then
        return
    end

    -- Clear the flag so tests only run once
    set_state("test_initialized", false)

    log_info("=== Running API Tests ===")

    -- Test 3: State Retrieval
    log_info("Testing state retrieval...")
    local test_value = get_state("test_value", {x = 0, y = 0})
    log_info(string.format("Retrieved value: x=%f, y=%f", test_value.x, test_value.y))

    local test_number = get_f64("test_number")
    log_info(string.format("Retrieved number: %f", test_number))

    local test_bool = get_bool("test_bool", false)
    log_info(string.format("Retrieved bool: %s", tostring(test_bool)))

    local test_string = get_string("test_string", "")
    log_info(string.format("Retrieved string: %s", test_string))

    -- Test 4: Entity Management
    log_info("Testing entity management...")
    local new_entity = spawn_entity("test_entity")
    log_info(string.format("Spawned entity: %d", new_entity))
    set_name(new_entity, "renamed_test_entity")

    -- Test 5: Transform API
    log_info("Testing transform API...")
    set_translation(new_entity, 5.0, 0.0, 0.0)
    translate(new_entity, 1.0, 1.0, 1.0)
    set_rotation(new_entity, 0.0, 1.57, 0.0)  -- 90 degrees in radians
    set_scale(new_entity, 2.0, 2.0, 2.0)

    local pos = get_world_translation(new_entity)
    if pos then
        log_info(string.format("Entity position: x=%f, y=%f, z=%f", pos.x, pos.y, pos.z))
    end

    local rot = get_world_rotation(new_entity)
    if rot then
        log_info(string.format("Entity rotation: yaw=%f, pitch=%f, roll=%f", rot.yaw, rot.pitch, rot.roll))
    end

    -- Test 6: Hierarchy API
    log_info("Testing hierarchy API...")
    local parent_entity = spawn_entity("parent")
    local child_entity = spawn_entity("child")
    set_parent(child_entity, parent_entity)

    local parent = get_parent(child_entity)
    if parent then
        log_info(string.format("Child's parent: %d", parent))
    end

    local children = get_children(parent_entity)
    if children then
        log_info(string.format("Parent has %d children", #children))
    end

    -- Test 7: Input API (just test the functions are callable)
    log_info("Testing input API...")
    local w_pressed = is_key_pressed("W")
    local mouse_pos = get_mouse_position()
    log_info(string.format("W key pressed: %s, Mouse at: x=%f, y=%f",
        tostring(w_pressed), mouse_pos.x, mouse_pos.y))

    -- Test 8: Query API
    log_info("Testing query API...")
    local entities_near = get_entities_in_radius(0.0, 0.0, 0.0, 100.0)
    log_info(string.format("Found %d entities within radius", #entities_near))

    -- Test 9: Component API
    log_info("Testing component API...")
    local has_transform = has_component(self_entity, "TransformComponent")
    log_info(string.format("Self has TransformComponent: %s", tostring(has_transform)))

    -- Test 10: Events API
    log_info("Testing events API...")
    emit_event("test_event", {
        message = "Hello from Lua!",
        value = 42
    })

    -- Test 11: File I/O API
    log_info("Testing file I/O API...")
    local test_content = "Hello from Lua file I/O test!"
    local write_result = write_file("scripts/lua_test_output.txt", test_content)
    if write_result == "" then
        log_info("File write successful")
        local read_content = read_file("scripts/lua_test_output.txt")
        log_info(string.format("File read result: %s", read_content))
    else
        log_error(string.format("File write failed: %s", write_result))
    end

    local exists = file_exists("scripts/lua_test_output.txt")
    log_info(string.format("File exists check: %s", tostring(exists)))

    -- Test 12: Script Access API
    log_info("Testing script access API...")
    local scripts = get_entity_scripts(self_entity)
    log_info(string.format("Script count on self: %d", #scripts))
    if #scripts > 0 then
        local script = scripts[1]
        log_info(string.format("Script kind: %s", script.kind))
        if script.name then
            log_info(string.format("Inline script name: %s", script.name))
        end

        local source = read_script_source(self_entity)
        if source then
            log_info(string.format("Script source length: %d", #source))
        else
            log_warn("Script source unavailable")
        end
    end

    log_info("=== All API Tests Complete ===")
end
