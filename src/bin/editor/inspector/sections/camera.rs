use egui::{ComboBox, DragValue, Grid};
use hecs::Entity;
use wgpu_cube::scene::{CameraComponent, CameraProjection};

use crate::inspector::actions::InspectorAction;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProjectionMode {
    Perspective,
    Orthographic,
}

/// Displays the camera component editor (projection mode and parameters)
pub fn show_camera_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: CameraComponent,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Camera", |ui| {
        let mut updated = component;
        let mut changed = false;

        let mut mode = match updated.projection {
            CameraProjection::Perspective { .. } => ProjectionMode::Perspective,
            CameraProjection::Orthographic { .. } => ProjectionMode::Orthographic,
        };
        let previous_mode = mode;

        ComboBox::from_id_salt("camera_projection_mode")
            .selected_text(match mode {
                ProjectionMode::Perspective => "Perspective",
                ProjectionMode::Orthographic => "Orthographic",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, ProjectionMode::Perspective, "Perspective");
                ui.selectable_value(&mut mode, ProjectionMode::Orthographic, "Orthographic");
            });

        if mode != previous_mode {
            let near = updated.near();
            let far = updated.far();
            updated.projection = match mode {
                ProjectionMode::Perspective => {
                    CameraProjection::perspective(60f32.to_radians(), near, far)
                }
                ProjectionMode::Orthographic => {
                    CameraProjection::orthographic(-5.0, 5.0, -5.0, 5.0, near, far)
                }
            };
            changed = true;
        }

        match &mut updated.projection {
            CameraProjection::Perspective {
                fov_y_radians,
                near,
                far,
            } => {
                Grid::new("camera_perspective_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Field of View (deg)");
                        let mut fov_degrees = fov_y_radians.to_degrees();
                        if ui
                            .add(
                                DragValue::new(&mut fov_degrees)
                                    .range(1.0..=179.0)
                                    .speed(0.1),
                            )
                            .changed()
                        {
                            *fov_y_radians = fov_degrees.to_radians();
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Near");
                        if ui
                            .add(DragValue::new(near).range(0.001..=1000.0).speed(0.01))
                            .changed()
                        {
                            *near = near.max(0.001);
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Far");
                        if ui
                            .add(DragValue::new(far).range(0.01..=10000.0).speed(0.1))
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });

                if *far <= *near {
                    *far = *near + 0.001;
                    changed = true;
                }
            }
            CameraProjection::Orthographic {
                left,
                right,
                bottom,
                top,
                near,
                far,
            } => {
                Grid::new("camera_orthographic_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Left");
                        if ui.add(DragValue::new(left).speed(0.1)).changed() {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Right");
                        if ui.add(DragValue::new(right).speed(0.1)).changed() {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Bottom");
                        if ui.add(DragValue::new(bottom).speed(0.1)).changed() {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Top");
                        if ui.add(DragValue::new(top).speed(0.1)).changed() {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Near");
                        if ui.add(DragValue::new(near).speed(0.1)).changed() {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Far");
                        if ui.add(DragValue::new(far).speed(0.1)).changed() {
                            changed = true;
                        }
                        ui.end_row();
                    });

                if *far <= *near {
                    *far = *near + 0.001;
                    changed = true;
                }
            }
        }

        if changed {
            actions.push(InspectorAction::UpdateCamera {
                entity,
                component: updated,
            });
        }
    });
}
