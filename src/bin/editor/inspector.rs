use egui::Grid;
use glam::{EulerRot, Vec3};

use wgpu_cube::{SceneEntityComponentsSummary, SceneEntityInspectorData};

pub fn show_entity_inspector(ui: &mut egui::Ui, data: &SceneEntityInspectorData) {
    ui.label(format!("Name: {}", data.name));
    ui.label(format!("Entity: {:?}", data.entity));
    ui.add_space(8.0);

    show_transform_section(ui, &data.components);
    ui.add_space(6.0);
    show_mesh_section(ui, &data.components);
    ui.add_space(6.0);
    show_material_section(ui, &data.components);
}

fn show_transform_section(ui: &mut egui::Ui, components: &SceneEntityComponentsSummary) {
    ui.collapsing("Transform", |ui| {
        if let Some(transform) = components.transform {
            Grid::new("transform_component_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Translation");
                    ui.monospace(format_vec3(transform.translation));
                    ui.end_row();

                    let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
                    ui.label("Rotation (deg)");
                    ui.monospace(format!(
                        "{:.2}, {:.2}, {:.2}",
                        yaw.to_degrees(),
                        pitch.to_degrees(),
                        roll.to_degrees()
                    ));
                    ui.end_row();

                    ui.label("Rotation (quat)");
                    ui.monospace(format!(
                        "{:.3}, {:.3}, {:.3}, {:.3}",
                        transform.rotation.x,
                        transform.rotation.y,
                        transform.rotation.z,
                        transform.rotation.w
                    ));
                    ui.end_row();

                    ui.label("Scale");
                    ui.monospace(format_vec3(transform.scale));
                    ui.end_row();
                });
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

fn show_material_section(ui: &mut egui::Ui, components: &SceneEntityComponentsSummary) {
    ui.collapsing("Material", |ui| {
        if let Some(material_component) = components.material {
            let material = material_component.0;
            let color = material.base_color;
            ui.label(format!(
                "Base color RGBA: ({}, {}, {}, {})",
                color[0], color[1], color[2], color[3]
            ));
            ui.label(format!(
                "Metallic: {:.2}",
                material.metallic_factor as f32 / 255.0
            ));
            ui.label(format!(
                "Roughness: {:.2}",
                material.roughness_factor as f32 / 255.0
            ));
            ui.label(format!(
                "Emissive strength: {:.2}",
                material.emissive_strength as f32 / 255.0
            ));
            ui.label(format!("Flags: 0x{:08X}", material.flags.bits()));
            ui.label(format!(
                "Base color texture: {}",
                material.base_color_texture
            ));
            ui.label(format!(
                "Metallic/Roughness texture: {}",
                material.metallic_roughness_texture
            ));
            ui.label(format!("Normal texture: {}", material.normal_texture));
            ui.label(format!("Emissive texture: {}", material.emissive_texture));
            ui.label(format!("Occlusion texture: {}", material.occlusion_texture));
        } else {
            ui.label("No Material component on this entity.");
        }
    });
}

fn format_vec3(vec: Vec3) -> String {
    format!("{:.3}, {:.3}, {:.3}", vec.x, vec.y, vec.z)
}
