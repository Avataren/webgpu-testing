function on_created(self_entity)
    local cube = spawn_entity("Editor Cube")
    set_translation(cube, 0.0, 0.5, 0.0)
    attach_script(cube, "scripts/editor_cube_spin.lua")
end
