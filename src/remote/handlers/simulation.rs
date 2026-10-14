//! Explicit simulation steps; window rendering and wall-clock waits remain independent.

use super::super::hub::InputEvent;
use super::{input, RemoteContext, RemoteResult};
use crate::app::RuntimeMode;
use crate::scene::InputState;
use serde_json::{json, Value};

pub(crate) fn apply_event(input: &mut InputState, event: &InputEvent) {
    match event {
        InputEvent::KeyDown(key) => input.press_key(key.clone()),
        InputEvent::KeyUp(key) => input.release_key(key),
        InputEvent::MouseDown(button) => input.press_mouse_button(*button),
        InputEvent::MouseUp(button) => input.release_mouse_button(*button),
        InputEvent::MouseMove(position) => {
            input.add_mouse_delta(*position - input.mouse_position());
            input.set_mouse_position(*position);
        }
        InputEvent::Scroll(delta) => input.add_scroll_delta(*delta),
    }
}

pub(super) fn step(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let handle = ctx.runtime_handle().ok_or("runtime unavailable")?;
    if handle.active_mode() != RuntimeMode::Playing || !handle.manual_simulation() {
        return Err("runtime.step requires runtime.set {mode: play, manual: true}; wait for the transition first".into());
    }
    let steps = params
        .get("steps")
        .map_or(Ok(1), |v| v.as_u64().ok_or("steps must be an integer"))?;
    let dt = params
        .get("dt")
        .map_or(Ok(1.0 / 60.0), |v| v.as_f64().ok_or("dt must be a number"))?;
    if !(1..=10_000).contains(&steps) || !dt.is_finite() || !(0.000001..=1.0).contains(&dt) {
        return Err("steps must be 1..=10000 and dt must be finite in 0.000001..=1 seconds".into());
    }
    let mut events = Vec::new();
    if let Some(values) = params.get("input") {
        for value in values.as_array().ok_or("input must be an array")? {
            events.extend(input::parse_event(value)?);
        }
    }
    if events.len() > 20_000 || events.iter().any(|(at, _)| *at > 100_000) {
        return Err(
            "input needs at most 20000 scheduled events within 100000 simulation steps".into(),
        );
    }
    let initial = handle.simulation_steps();
    for (delay, event) in events {
        ctx.hub.schedule_simulation_input(initial + delay, event);
    }
    for at in 0..steps {
        ctx.hub
            .apply_simulation_input(initial + at, ctx.gpu.scene.input_state_mut());
        ctx.gpu.scene.update_with_mode(dt, false);
        ctx.gpu.scene.apply_script_asset_commands(ctx.gpu.renderer);
        ctx.gpu.scene.propagate_transforms();
        ctx.gpu.scene.input_state_mut().update();
    }
    handle.add_simulation_steps(steps);
    Ok(
        json!({"steps": steps, "dt": dt, "simulation_steps": handle.simulation_steps(),
        "game_time": ctx.gpu.scene.time(), "manual": true,
        "scope": "Lua gameplay and scene animation; rendering, GPU particles and custom application systems are not stepped"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;
    use crate::scripting::LuaScriptComponent;

    #[test]
    fn seeded_gameplay_replays_the_same_random_stream_and_time() {
        fn run(seed: u64) -> Value {
            let mut scene = Scene::new();
            let e = scene.main_world_mut().spawn((LuaScriptComponent::new_inline("step", r#"
                GAME = {sum = 0, ticks = 0}
                function on_created() GAME.initial = math.random() end
                function update(_, dt) GAME.sum = GAME.sum + math.random(); GAME.ticks = GAME.ticks + 1 end
            "#),));
            scene.lua_scripting_mut().seed_random(seed).unwrap();
            scene.update_with_mode(0.0, false);
            for _ in 0..30 {
                scene.update_with_mode(1.0 / 60.0, false);
            }
            scene
                .eval_lua(
                    "return {GAME.initial, GAME.sum, GAME.ticks, get_time()}",
                    Some(e),
                )
                .unwrap()
                .values[0]
                .clone()
        }
        assert_eq!(run(123), run(123));
        assert_ne!(run(123), run(124));
    }
}
