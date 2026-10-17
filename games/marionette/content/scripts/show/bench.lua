-- The benchmark: the show played once per graphics configuration (raster at native
-- resolution, then rendered at half resolution and upscaled with FSR), with each part's
-- frame times, GPU time and CPU stage times recorded after a short warm-up. The show's
-- settings come back when the run ends.

local B = {}

B.PASSES = {
    {
        id = "raster",
        name = "RASTER",
        note = "shadow maps, screen-space reflections and AO, baked light probes",
        settings = {
            ray_traced_shadows = false,
            reflections = "screen_space",
            ambient_occlusion = "screen_space",
            ray_traced_refraction = false,
            global_illumination = "off",
            path_tracing = false,
            render_scale = 1.0,
            vsync = false,
        },
    },
    {
        id = "raster_fsr50",
        name = "RASTER + FSR 50%",
        note = "the same, rendered at half resolution and upscaled with FSR",
        settings = {
            ray_traced_shadows = false,
            reflections = "screen_space",
            ambient_occlusion = "screen_space",
            ray_traced_refraction = false,
            global_illumination = "off",
            path_tracing = false,
            render_scale = 0.5,
            upscaler = "fsr",
            vsync = false,
        },
    },
}

-- What the show itself runs with (the benchmark's first pass).
B.SHOW_SETTINGS = B.PASSES[1].settings

-- Seconds at the start of each part that are not measured (spawning, binding clips).
B.WARMUP = 1.5
local CPU_STAGES = { "scripts", "animations", "transforms", "render_collect", "render_gpu", "gpu_wait" }

B.active = false
B.results = {}

-- Every pass, or only the first (raster at native resolution).
function B.passes(full)
    if full then
        return B.PASSES
    end
    return { B.PASSES[1] }
end

function B.start(passes, scale)
    local original = graphics_settings()
    original.capabilities = nil
    B.original = original
    B.active = true
    B.passes_run = passes
    B.scale = scale
    B.results = {}
    B.started = get_time()
end

function B.begin_pass(pass)
    if B.pass then
        B.end_pass()
    end
    set_graphics_settings(pass.settings, { save = false })
    B.pass = { id = pass.id, name = pass.name, note = pass.note, parts = {} }
    table.insert(B.results, B.pass)
end

function B.begin_part(name)
    B.part = {
        name = name,
        frames = {},
        gpu_sum = 0,
        gpu_frames = 0,
        cpu = {},
        characters = 0,
        bones = 0,
        skinned_vertices = 0,
        draw_calls = 0,
    }
    for _, stage in ipairs(CPU_STAGES) do
        B.part.cpu[stage] = 0
    end
end

-- Record one frame: `stats` is this frame's frame_stats(), the costs of the frame before,
-- which began `stats.frame_ms` before `t`. Frames that began in the warm-up are left out,
-- also when one long frame (spawning a crowd) carried the part's clock past it.
function B.sample(t, stats, characters, bones)
    local part = B.part
    if not part or stats.frame_ms <= 0 or t - stats.frame_ms / 1000 < B.WARMUP then
        return
    end
    table.insert(part.frames, stats.frame_ms)
    if stats.gpu_ms then
        part.gpu_sum = part.gpu_sum + stats.gpu_ms
        part.gpu_frames = part.gpu_frames + 1
    end
    for _, stage in ipairs(CPU_STAGES) do
        part.cpu[stage] = part.cpu[stage] + (stats.cpu[stage] or 0)
    end
    part.characters = math.max(part.characters, characters)
    part.bones = math.max(part.bones, bones)
    part.skinned_vertices = math.max(part.skinned_vertices, stats.skinned_vertices or 0)
    part.draw_calls = math.max(part.draw_calls, stats.draw_calls or 0)
end

local function percentile(sorted, p)
    if #sorted == 0 then
        return 0
    end
    local index = math.max(1, math.min(#sorted, math.ceil(#sorted * p)))
    return sorted[index]
end

function B.end_part()
    local part = B.part
    B.part = nil
    if not part or not B.pass then
        return
    end
    local frames = #part.frames
    local total = 0
    for _, ms in ipairs(part.frames) do
        total = total + ms
    end
    table.sort(part.frames)
    local summary = {
        name = part.name,
        frames = frames,
        seconds = total / 1000,
        fps = frames > 0 and frames * 1000 / total or 0,
        low_1 = frames > 0 and 1000 / percentile(part.frames, 0.99) or 0,
        median_ms = percentile(part.frames, 0.5),
        gpu_ms = part.gpu_frames > 0 and part.gpu_sum / part.gpu_frames or nil,
        cpu_ms = {},
        characters = part.characters,
        bones = part.bones,
        skinned_vertices = part.skinned_vertices,
        draw_calls = part.draw_calls,
    }
    for _, stage in ipairs(CPU_STAGES) do
        summary.cpu_ms[stage] = frames > 0 and part.cpu[stage] / frames or 0
    end
    table.insert(B.pass.parts, summary)
end

function B.end_pass()
    local pass = B.pass
    if not pass then
        return
    end
    local frames, seconds = 0, 0
    for _, part in ipairs(pass.parts) do
        frames = frames + part.frames
        seconds = seconds + part.seconds
    end
    pass.fps = seconds > 0 and frames / seconds or 0
    B.pass = nil
end

-- A minimal JSON encoder for the report.
local function json(value, out)
    local kind = type(value)
    if kind == "table" then
        if #value > 0 or next(value) == nil then
            out[#out + 1] = "["
            for i, item in ipairs(value) do
                if i > 1 then
                    out[#out + 1] = ","
                end
                json(item, out)
            end
            out[#out + 1] = "]"
        else
            local keys = {}
            for key in pairs(value) do
                keys[#keys + 1] = tostring(key)
            end
            table.sort(keys)
            out[#out + 1] = "{"
            for i, key in ipairs(keys) do
                if i > 1 then
                    out[#out + 1] = ","
                end
                out[#out + 1] = string.format("%q:", key)
                json(value[key], out)
            end
            out[#out + 1] = "}"
        end
    elseif kind == "number" then
        if value ~= value or value == math.huge or value == -math.huge then
            out[#out + 1] = "null"
        elseif math.type(value) == "integer" then
            out[#out + 1] = tostring(value)
        else
            out[#out + 1] = string.format("%.3f", value)
        end
    elseif kind == "string" then
        out[#out + 1] = string.format("%q", value)
    elseif kind == "boolean" then
        out[#out + 1] = tostring(value)
    else
        out[#out + 1] = "null"
    end
end

function B.encode(value)
    local out = {}
    json(value, out)
    return table.concat(out)
end

-- Restore the player's settings and write the report (log and save data).
function B.finish(extra)
    if B.part then
        B.end_part()
    end
    if B.pass then
        B.end_pass()
    end
    if B.original then
        set_graphics_settings(B.original, { save = false })
    end
    B.active = false
    local settings = B.original or {}
    B.report = {
        scale = B.scale,
        seconds = get_time() - (B.started or 0),
        anti_aliasing = settings.anti_aliasing,
        passes = B.results,
    }
    for key, value in pairs(extra or {}) do
        B.report[key] = value
    end
    local text = B.encode(B.report)
    log_info("MARIONETTE_BENCHMARK " .. text)
    save_data("benchmark_last", text)
    return B.report
end

-- Stop early (Escape, or the script reloading): put the player's settings back.
function B.abort()
    if B.active and B.original then
        set_graphics_settings(B.original, { save = false })
    end
    B.active = false
    B.part, B.pass = nil, nil
end

return B
