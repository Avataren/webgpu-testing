use egui::Grid;
use glam::{EulerRot, Quat, Vec3};
use hecs::Entity;
use std::f32::consts::PI;
use wgpu_cube::scene::Transform;

use crate::inspector::actions::InspectorAction;
use crate::inspector::helpers::vec3_editor;

/// Displays the transform component editor (translation, rotation, scale)
pub fn show_transform_section(
    ui: &mut egui::Ui,
    entity: Entity,
    transform: Transform,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Transform", |ui| {
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
                    "({:.3}, {:.3}, {:.3}, {:.3})",
                    updated.rotation.x, updated.rotation.y, updated.rotation.z, updated.rotation.w
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
    });
}
