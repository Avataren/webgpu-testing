-- A launch pad: its collider is a sensor, so characters and props walking onto it are
-- reported here instead of being blocked.

local LAUNCH_SPEED = 15 -- m/s upward

function on_trigger_enter(self, other)
    local p = get_world_translation(self)
    if character_state(other) then
        -- On a character an impulse adds straight to its velocity (m/s).
        apply_impulse(other, 0, LAUNCH_SPEED, 0)
    else
        -- On a body it is a push in kg·m/s, so lighter props fly further.
        apply_impulse(other, 0, 60, 0)
    end
    emit_particles(p.x, p.y + 0.1, p.z, {
        count = 40, shape = "ring", speed = 6, life = 0.4, color = { 0.3, 0.85, 1.0 }, intensity = 3,
    })
    play_sound("content/audio/boing.wav", { volume = 0.6 })
end
