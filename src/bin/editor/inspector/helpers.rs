use egui::{color_picker::color_edit_button_rgb, DragValue};
use glam::Vec3;
use hecs::Entity;
use std::ops::RangeInclusive;
use wgpu_cube::scene::CanCastShadow;

use super::actions::InspectorAction;

/// Add spacing between inspector sections (only if not the first section)
pub fn begin_section(ui: &mut egui::Ui, first_section: &mut bool) {
    if *first_section {
        *first_section = false;
    } else {
        ui.add_space(6.0);
    }
}

/// Generic Vec3 editor with X/Y/Z drag values
pub fn vec3_editor(ui: &mut egui::Ui, label: &str, value: &mut Vec3) -> bool {
    let mut changed = false;
    ui.label(label);
    ui.horizontal(|ui| {
        changed |= ui.add(DragValue::new(&mut value.x).speed(0.1)).changed();
        changed |= ui.add(DragValue::new(&mut value.y).speed(0.1)).changed();
        changed |= ui.add(DragValue::new(&mut value.z).speed(0.1)).changed();
    });
    ui.end_row();
    changed
}

/// Editor for material float values (encoded as u8, displayed as 0.0-1.0)
pub fn material_float_editor(
    ui: &mut egui::Ui,
    label: &str,
    encoded_value: u8,
    mut setter: impl FnMut(u8),
) -> bool {
    let mut value = encoded_value as f32 / 255.0;
    ui.label(label);
    let response = ui
        .add(
            DragValue::new(&mut value)
                .speed(0.01)
                .range(0.0..=1.0)
                .fixed_decimals(3),
        )
        .changed();
    ui.end_row();
    if response {
        let encoded = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        setter(encoded);
        true
    } else {
        false
    }
}

/// Generic float drag value editor with optional range
pub fn float_drag_value(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    speed: f32,
    range: Option<RangeInclusive<f32>>,
) -> bool {
    ui.label(label);
    let mut drag = DragValue::new(value).speed(speed);
    if let Some(range) = range {
        drag = drag.range(range);
    }
    let changed = ui.add(drag).changed();
    ui.end_row();
    changed
}

/// Generic u32 drag value editor with optional range
pub fn u32_drag_value(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u32,
    speed: f32,
    range: Option<RangeInclusive<u32>>,
) -> bool {
    ui.label(label);
    let mut drag = DragValue::new(value).speed(speed);
    if let Some(range) = range {
        drag = drag.range(range);
    }
    let changed = ui.add(drag).changed();
    ui.end_row();
    changed
}

/// RGB color picker for light colors
pub fn light_color_editor(ui: &mut egui::Ui, label: &str, color: &mut Vec3) -> bool {
    let mut rgb = [color.x, color.y, color.z];
    ui.label(label);
    let changed = color_edit_button_rgb(ui, &mut rgb).changed();
    ui.end_row();
    if changed {
        *color = Vec3::new(
            rgb[0].clamp(0.0, 1.0),
            rgb[1].clamp(0.0, 1.0),
            rgb[2].clamp(0.0, 1.0),
        );
    }
    changed
}

/// Angle editor (stores radians, displays degrees)
pub fn light_angle_editor(ui: &mut egui::Ui, label: &str, radians: &mut f32) -> bool {
    let mut degrees = radians.to_degrees();
    let changed = float_drag_value(ui, label, &mut degrees, 1.0, Some(0.0..=179.0));
    if changed {
        *radians = degrees.clamp(0.0, 179.0).to_radians();
    }
    changed
}

/// Checkbox for enabling/disabling shadow casting
pub fn shadow_checkbox_row(
    ui: &mut egui::Ui,
    entity: Entity,
    flag: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) {
    let mut casts_shadow = flag.map(|flag| flag.0).unwrap_or(false);
    ui.label("Cast Shadows");
    if ui.checkbox(&mut casts_shadow, "Enabled").changed() {
        actions.push(InspectorAction::SetCanCastShadow {
            entity,
            casts_shadow,
        });
    }
    ui.end_row();
}
