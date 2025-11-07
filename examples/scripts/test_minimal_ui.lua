-- @editor
-- Minimal UI test

function on_created(self_entity)
    log_info("Minimal UI test created")
end

function on_ui(self_entity, ui)
    ui:heading("Test")
    ui:label("Hello World")

    if ui:button("Test Button") then
        log_info("Button clicked!")
    end
end
