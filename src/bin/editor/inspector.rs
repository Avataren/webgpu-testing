use egui::{color_picker::color_edit_button_rgba, color_picker::Alpha, DragValue, Grid};
use glam::{EulerRot, Quat, Vec3};
use std::f32::consts::PI;

use hecs::Entity;

use wgpu_cube::renderer::Material;
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::{RuneScriptComponent, RuneScriptSource};
use wgpu_cube::{SceneEntityComponentsSummary, SceneEntityInspectorData};

#[derive(Clone)]
pub enum InspectorAction {
    EditScript {
        entity: Entity,
        component: RuneScriptComponent,
    },
    UpdateTransform {
        entity: Entity,
        transform: Transform,
    },
    UpdateMaterial {
        entity: Entity,
        material: Material,
    },
}

pub fn show_entity_inspector(
    ui: &mut egui::Ui,
    data: &SceneEntityInspectorData,
) -> Vec<InspectorAction> {
    let mut actions = Vec::new();
    ui.label(format!("Name: {}", data.name));
    ui.label(format!("Entity: {:?}", data.entity));
    ui.add_space(8.0);

    show_transform_section(ui, data.entity, &data.components, &mut actions);
    ui.add_space(6.0);
    show_mesh_section(ui, &data.components);
    ui.add_space(6.0);
    show_material_section(ui, data.entity, &data.components, &mut actions);
    ui.add_space(6.0);
    if let Some(action) = show_script_section(ui, data) {
        actions.push(action);
    }

    actions
}

fn show_transform_section(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Transform", |ui| {
        if let Some(transform) = components.transform {
            let mut updated = transform;
            let mut changed = false;

            Grid::new("transform_component_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    if vec3_editor(ui, "Translation", &mut updated.translation) {
                        changed = true;
                    }

                    let (yaw, pitch, roll) = updated.rotation.to_euler(EulerRot::YXZ);
                    let mut rotation_deg = Vec3::new(yaw, pitch, roll) * (180.0 / PI);
                    if vec3_editor(ui, "Rotation (deg)", &mut rotation_deg) {
                        let rotation_rad = rotation_deg * (PI / 180.0);
                        updated.rotation = Quat::from_euler(
                            EulerRot::YXZ,
                            rotation_rad.x,
                            rotation_rad.y,
                            rotation_rad.z,
                        );
                        changed = true;
                    }

                    ui.label("Rotation (quat)");
                    ui.monospace(format!(
                        "{:.3}, {:.3}, {:.3}, {:.3}",
                        updated.rotation.x,
                        updated.rotation.y,
                        updated.rotation.z,
                        updated.rotation.w
                    ));
                    ui.end_row();

                    if vec3_editor(ui, "Scale", &mut updated.scale) {
                        changed = true;
                    }
                });

            if changed {
                actions.push(InspectorAction::UpdateTransform {
                    entity,
                    transform: updated,
                });
            }
        } else {
            ui.label("No Transform component on this entity.");
        }
    });
}

fn show_mesh_section(ui: &mut egui::Ui, components: &SceneEntityComponentsSummary) {
    ui.collapsing("Mesh", |ui| {
        if let Some(mesh) = components.mesh {
            ui.label(format!("Handle index: {}", mesh.0.index()));
        } else {
            ui.label("No Mesh component on this entity.");
        }
    });
}

fn show_material_section(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Material", |ui| {
        if let Some(material_component) = components.material {
            let mut material = material_component.0;
            let mut changed = false;

            Grid::new("material_component_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Base Color");
                    let mut color = egui::Rgba::from_rgba_unmultiplied(
                        material.base_color[0] as f32 / 255.0,
                        material.base_color[1] as f32 / 255.0,
                        material.base_color[2] as f32 / 255.0,
                        material.base_color[3] as f32 / 255.0,
                    );
                    if color_edit_button_rgba(ui, &mut color, Alpha::BlendOrAdditive).changed() {
                        let array = color.to_array();
                        material.base_color = [
                            (array[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                            (array[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                            (array[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                            (array[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                        ];
                        changed = true;
                    }
                    ui.end_row();

                    changed |=
                        material_float_editor(ui, "Metallic", material.metallic_factor, |value| {
                            material.metallic_factor = value
                        });
                    changed |= material_float_editor(
                        ui,
                        "Roughness",
                        material.roughness_factor,
                        |value| material.roughness_factor = value,
                    );
                    changed |= material_float_editor(
                        ui,
                        "Emissive strength",
                        material.emissive_strength,
                        |value| material.emissive_strength = value,
                    );

                    ui.label("Flags");
                    ui.monospace(format!("0x{:08X}", material.flags.bits()));
                    ui.end_row();

                    ui.label("Base color texture");
                    ui.monospace(format!("{}", material.base_color_texture));
                    ui.end_row();

                    ui.label("Metallic/Roughness texture");
                    ui.monospace(format!("{}", material.metallic_roughness_texture));
                    ui.end_row();

                    ui.label("Normal texture");
                    ui.monospace(format!("{}", material.normal_texture));
                    ui.end_row();

                    ui.label("Emissive texture");
                    ui.monospace(format!("{}", material.emissive_texture));
                    ui.end_row();

                    ui.label("Occlusion texture");
                    ui.monospace(format!("{}", material.occlusion_texture));
                    ui.end_row();
                });

            if changed {
                actions.push(InspectorAction::UpdateMaterial { entity, material });
            }
        } else {
            ui.label("No Material component on this entity.");
        }
    });
}

fn show_script_section(
    ui: &mut egui::Ui,
    data: &SceneEntityInspectorData,
) -> Option<InspectorAction> {
    let mut action = None;
    ui.collapsing("Script", |ui| {
        if let Some(script) = data.components.script.as_ref() {
            let source_label = match script.source() {
                RuneScriptSource::Inline { name, .. } => {
                    format!("Inline script: {}", name)
                }
                RuneScriptSource::File { path } => {
                    format!("File: {}", path.display())
                }
            };
            ui.label(source_label);

            if ui.button("Edit").clicked() {
                action = Some(InspectorAction::EditScript {
                    entity: data.entity,
                    component: script.clone(),
                });
            }
        } else {
            ui.label("No script component on this entity.");
        }
    });
    action
}

fn vec3_editor(ui: &mut egui::Ui, label: &str, value: &mut Vec3) -> bool {
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

fn material_float_editor(
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
