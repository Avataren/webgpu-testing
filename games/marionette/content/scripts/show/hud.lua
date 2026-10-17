-- Everything drawn over the show: the title screen, loading progress, part title cards,
-- the live statistics and the benchmark's results table.

local H = {}

local WHITE = { 1, 1, 1, 1 }
local DIM = { 0.72, 0.76, 0.86, 1 }
local ACCENT = { 0.35, 0.95, 1.0, 1 }
local WARM = { 1.0, 0.55, 0.85, 1 }

local function with_alpha(color, alpha)
    return { color[1], color[2], color[3], (color[4] or 1) * alpha }
end

local function thousands(n)
    local text = tostring(math.floor(n + 0.5))
    local out = text:reverse():gsub("(%d%d%d)", "%1,"):reverse()
    return (out:gsub("^,", ""))
end
H.thousands = thousands

local function center(ui, y, text, size, color)
    ui:screen_label("center", 0, y, text, { size = size, color = color })
end

function H.title(ui, info)
    local pulse = 0.75 + 0.25 * math.sin(info.time * 2.2)
    center(ui, -150, "M A R I O N E T T E", 64, with_alpha(ACCENT, pulse))
    center(ui, -92, info.subtitle, 18, DIM)
    local lines = {
        { "ENTER", "watch the show" },
        { "B", "benchmark: raster, then raster at 50% upscaled with FSR" },
        { "R", "benchmark: raster only" },
        { "1  2  3", string.format("crowd size: 1x  2x  3x   (now %dx)", info.scale) },
    }
    local size = ui:get_viewport_size()
    local left = size.width / 2 - 250
    for i, line in ipairs(lines) do
        local y = size.height / 2 - 30 + (i - 1) * 32
        ui:screen_label("top_left", left, y, line[1], { size = 20, color = WARM })
        ui:screen_label("top_left", left + 90, y + 1, line[2], { size = 18, color = WHITE })
    end
    if info.autostart then
        center(ui, 150, string.format("the show starts in %d", math.ceil(info.autostart)), 16, DIM)
    end
end

function H.message(ui, lines)
    for i, line in ipairs(lines) do
        center(ui, (i - 1 - #lines / 2) * 30, line, i == 1 and 26 or 17, i == 1 and ACCENT or WHITE)
    end
end

function H.loading(ui, done, total, name)
    center(ui, -40, "M A R I O N E T T E", 40, ACCENT)
    local width = 40
    local filled = total > 0 and math.floor(done / total * width + 0.5) or 0
    center(ui, 10, "[" .. string.rep("#", filled) .. string.rep(".", width - filled) .. "]", 18, WHITE)
    center(ui, 42, string.format("loading the cast  %d / %d   %s", done, total, name or ""), 16, DIM)
end

function H.card(ui, number, name, subtitle, alpha, extra)
    if alpha <= 0 then
        return
    end
    ui:screen_label("bottom_left", 48, 150, string.format("%02d", number), {
        size = 56,
        color = with_alpha(ACCENT, alpha),
    })
    ui:screen_label("bottom_left", 128, 158, name, { size = 40, color = with_alpha(WHITE, alpha) })
    if subtitle and subtitle ~= "" then
        ui:screen_label("bottom_left", 130, 128, subtitle, { size = 17, color = with_alpha(DIM, alpha) })
    end
    if extra then
        ui:screen_label("bottom_left", 130, 104, extra, { size = 15, color = with_alpha(WARM, alpha) })
    end
end

function H.stats(ui, s)
    local lines = {}
    if s.pass then
        table.insert(lines, { string.format("PASS %d/%d  %s", s.pass_index, s.pass_count, s.pass), ACCENT, 17 })
    end
    table.insert(lines, {
        string.format("%5.0f FPS   %5.1f ms   GPU %s", s.fps, s.frame_ms, s.gpu_ms and string.format("%.1f ms", s.gpu_ms) or "-"),
        WHITE,
        17,
    })
    table.insert(lines, {
        string.format("CPU  animation %.1f   transforms %.1f   render %.1f   scripts %.1f ms",
            s.cpu.animations or 0, s.cpu.transforms or 0,
            (s.cpu.render_collect or 0) + (s.cpu.render_gpu or 0), s.cpu.scripts or 0),
        DIM,
        14,
    })
    table.insert(lines, {
        string.format("%s characters   %s bones   %s skinned vertices   %s draws",
            thousands(s.characters), thousands(s.bones), thousands(s.skinned_vertices), thousands(s.draw_calls)),
        DIM,
        14,
    })
    local y = 14
    for _, line in ipairs(lines) do
        ui:screen_label("top_left", 16, y, line[1], { size = line[3], color = line[2], backdrop = true })
        y = y + line[3] + 10
    end
end

function H.results(ui, report)
    local size = ui:get_viewport_size()
    local passes = report.passes or {}
    local columns = { 0, 190, 270 }
    local pass_width = 190
    local table_width = columns[3] + pass_width * #passes
    local left = math.max(16, (size.width - table_width) / 2)
    local top = math.max(16, size.height / 2 - 60 - #((passes[1] or {}).parts or {}) * 13)
    local function cell(column_x, row_y, text, color, text_size)
        ui:screen_label("top_left", left + column_x, top + row_y, text, { size = text_size or 16, color = color or WHITE })
    end

    cell(0, -70, "BENCHMARK RESULTS", ACCENT, 30)
    cell(0, -34, string.format("crowd %dx   %s anti-aliasing   %.0f s", report.scale or 1,
        tostring(report.anti_aliasing or "?"), report.seconds or 0), DIM, 15)
    cell(columns[1], 0, "PART", DIM)
    cell(columns[2], 0, "CHARS", DIM)
    for p, pass in ipairs(passes) do
        cell(columns[3] + (p - 1) * pass_width, 0, pass.name, WARM, 15)
        cell(columns[3] + (p - 1) * pass_width, 20, "fps (1% low)  gpu ms", DIM, 12)
    end
    local rows = passes[1] and #passes[1].parts or 0
    for r = 1, rows do
        local y = 26 + r * 26
        local first = passes[1].parts[r]
        cell(columns[1], y, first.name)
        cell(columns[2], y, thousands(first.characters))
        for p, pass in ipairs(passes) do
            local part = pass.parts[r]
            if part then
                local gpu = part.gpu_ms and string.format("%.1f", part.gpu_ms) or "-"
                cell(columns[3] + (p - 1) * pass_width, y,
                    string.format("%4.0f (%3.0f)   %s", part.fps, part.low_1, gpu))
            end
        end
    end
    local y = 26 + (rows + 1) * 26 + 8
    cell(columns[1], y, "WHOLE SHOW", ACCENT)
    for p, pass in ipairs(passes) do
        cell(columns[3] + (p - 1) * pass_width, y, string.format("%4.0f fps", pass.fps or 0), ACCENT)
    end
    cell(0, y + 46, "ENTER: back to the title      the full report is in the log (MARIONETTE_BENCHMARK)", DIM, 14)
end

return H
