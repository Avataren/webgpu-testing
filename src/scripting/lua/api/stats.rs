//! # Frame statistics API
//!
//! `frame_stats()` reports what the last complete frame cost, so a game can show its own
//! performance overlay or a demo can run as a benchmark:
//!
//! - `frame_ms`, `fps`: wall time of the frame (including waiting for the display)
//! - `cpu`: milliseconds per stage of the engine's frame loop (`scripts`, `animations`,
//!   `transforms`, `render_collect`, `render_gpu`, `gpu_wait`, `present`, ...), the same
//!   stages as `engine_stats`' `frame_breakdown`
//! - `gpu_ms` and `gpu` (milliseconds per render stage) from GPU timestamp queries, a
//!   frame or two old; `nil` on devices without them (and in browsers)
//! - `draw_calls`, `instances`, `culled_instances`, `skinned_instances`,
//!   `skinned_vertices`: what the renderer drew
//! - `entities`: how many entities the scene holds

use mlua::{Lua, Result as LuaResult};

use crate::app::core::frame_timings;
use crate::scripting::lua::guards::with_active_world;

pub(crate) fn register_stats_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    globals.set(
        "frame_stats",
        lua.create_function(|lua, ()| {
            let rendered = frame_timings::rendered();
            let table = lua.create_table()?;
            table.set("frame_ms", rendered.dt_ms)?;
            table.set(
                "fps",
                if rendered.dt_ms > 0.0 {
                    1000.0 / rendered.dt_ms
                } else {
                    0.0
                },
            )?;

            let cpu = lua.create_table()?;
            for (stage, last, _) in frame_timings::snapshot() {
                cpu.set(stage, last)?;
            }
            table.set("cpu", cpu)?;

            if let Some(gpu) = &rendered.gpu {
                table.set("gpu_ms", gpu.total_ms)?;
                let stages = lua.create_table()?;
                for (stage, ms) in &gpu.stages {
                    stages.set(*stage, *ms)?;
                }
                table.set("gpu", stages)?;
            }

            table.set("draw_calls", rendered.draw_calls)?;
            table.set("instances", rendered.instances)?;
            table.set("culled_instances", rendered.culled_instances)?;
            table.set("skinned_instances", rendered.skinned_instances)?;
            table.set("skinned_vertices", rendered.skinned_vertices)?;
            table.set("entities", with_active_world(|world| Ok(world.len()))?)?;
            Ok(table)
        })?,
    )?;
    Ok(())
}
