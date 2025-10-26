use egui::{
    color_picker::color_edit_button_rgb, color_picker::color_edit_button_rgba, color_picker::Alpha,
    ComboBox, DragValue, Grid, Slider,
};
use glam::{EulerRot, Quat, Vec3};
use std::f32::consts::PI;
use std::ops::RangeInclusive;

use hecs::Entity;

use wgpu_cube::renderer::Material;
use wgpu_cube::scene::{
    CameraComponent, CameraProjection, CanCastShadow, DirectionalLight, EnvironmentComponent,
    MaterialComponent, MeshComponent, ParticleBehaviorPreset, ParticleSystemComponent, PointLight,
    SpotLight, Transform,
};
use wgpu_cube::scripting::{RuneScriptComponent, RuneScriptSource};
use wgpu_cube::{SceneEntityComponentsSummary, SceneEntityInspectorData};

#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;

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
    UpdateCamera {
        entity: Entity,
        component: CameraComponent,
    },
    UpdateMaterial {
        entity: Entity,
        material: Material,
    },
    UpdatePointLight {
        entity: Entity,
        light: PointLight,
    },
    UpdateDirectionalLight {
        entity: Entity,
        light: DirectionalLight,
    },
    UpdateSpotLight {
        entity: Entity,
        light: SpotLight,
    },
    UpdateEnvironment {
        entity: Entity,
        component: EnvironmentComponent,
    },
    SetCanCastShadow {
        entity: Entity,
        casts_shadow: bool,
    },
    UpdateParticleSystem {
        entity: Entity,
        component: ParticleSystemComponent,
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

    let mut first_section = true;

    if let Some(transform) = data.components.transform {
        begin_section(ui, &mut first_section);
        show_transform_section(ui, data.entity, transform, &mut actions);
    }

    if let Some(camera) = data.components.camera {
        begin_section(ui, &mut first_section);
        show_camera_section(ui, data.entity, camera, &mut actions);
    }

    if let Some(mesh) = data.components.mesh {
        begin_section(ui, &mut first_section);
        show_mesh_section(ui, mesh);
    }

    if let Some(material) = data.components.material {
        begin_section(ui, &mut first_section);
        show_material_section(ui, data.entity, material, &mut actions);
    }

    let has_light = data.components.point_light.is_some()
        || data.components.directional_light.is_some()
        || data.components.spot_light.is_some();
    if has_light {
        begin_section(ui, &mut first_section);
        show_light_sections(ui, data.entity, &data.components, &mut actions);
    }

    if let Some(environment) = data.components.environment.clone() {
        begin_section(ui, &mut first_section);
        show_environment_section(ui, data.entity, environment, &mut actions);
    }

    if let Some(component) = data.components.particle_system.clone() {
        begin_section(ui, &mut first_section);
        show_particle_system_section(ui, data.entity, component, &mut actions);
    }

    if let Some(script) = data.components.script.clone() {
        begin_section(ui, &mut first_section);
        if let Some(action) = show_script_section(ui, data.entity, &script) {
            actions.push(action);
        }
    }

    actions
}

fn begin_section(ui: &mut egui::Ui, first_section: &mut bool) {
    if *first_section {
        *first_section = false;
    } else {
        ui.add_space(6.0);
    }
}

fn show_transform_section(
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

fn show_camera_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: CameraComponent,
    actions: &mut Vec<InspectorAction>,
) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ProjectionMode {
        Perspective,
        Orthographic,
    }

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

fn show_mesh_section(ui: &mut egui::Ui, mesh: MeshComponent) {
    ui.collapsing("Mesh", |ui| {
        ui.label(format!("Handle index: {}", mesh.0.index()));
    });
}

fn show_material_section(
    ui: &mut egui::Ui,
    entity: Entity,
    material_component: MaterialComponent,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Material", |ui| {
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
                changed |=
                    material_float_editor(ui, "Roughness", material.roughness_factor, |value| {
                        material.roughness_factor = value
                    });
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
    });
}

fn show_particle_system_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: ParticleSystemComponent,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Particle System", |ui| {
        let mut updated = component;
        let mut changed = false;

        Grid::new("particle_system_component_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Spawn rate");
                if ui
                    .add(
                        DragValue::new(&mut updated.spawn_rate)
                            .range(0.0..=10_000.0)
                            .speed(0.1),
                    )
                    .changed()
                {
                    updated.spawn_rate = updated.spawn_rate.max(0.0);
                    changed = true;
                }
                ui.end_row();

                ui.label("Behavior");
                let mut behavior = updated.behavior;
                ComboBox::from_id_salt("particle_behavior_combo")
                    .selected_text(behavior.display_name())
                    .show_ui(ui, |ui| {
                        for preset in ParticleBehaviorPreset::variants() {
                            ui.selectable_value(&mut behavior, preset, preset.display_name());
                        }
                    });
                if behavior != updated.behavior {
                    updated.set_behavior(behavior);
                    changed = true;
                }
                ui.end_row();
            });

        if changed {
            actions.push(InspectorAction::UpdateParticleSystem {
                entity,
                component: updated,
            });
        }
    });
}

fn show_light_sections(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    if let Some(light) = components.point_light {
        show_point_light_section(ui, entity, light, components.can_cast_shadow, actions);
    }

    if let Some(light) = components.directional_light {
        ui.add_space(6.0);
        show_directional_light_section(ui, entity, light, components.can_cast_shadow, actions);
    }

    if let Some(light) = components.spot_light {
        ui.add_space(6.0);
        show_spot_light_section(ui, entity, light, components.can_cast_shadow, actions);
    }
}

fn show_environment_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: EnvironmentComponent,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Environment", |ui| {
        let mut edited = component.clone();
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("Clear color");
            let mut color = [
                edited.clear_color[0].clamp(0.0, 1.0),
                edited.clear_color[1].clamp(0.0, 1.0),
                edited.clear_color[2].clamp(0.0, 1.0),
            ];
            if color_edit_button_rgb(ui, &mut color).changed() {
                edited.clear_color[0] = color[0];
                edited.clear_color[1] = color[1];
                edited.clear_color[2] = color[2];
                changed = true;
            }
        });

        let mut hdr_settings = edited.hdr.clone().unwrap_or_default();
        let mut hdr_enabled = hdr_settings.enabled;
        let mut grading = edited.color_grading;

        Grid::new("environment_component_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Ambient intensity");
                changed |= ui
                    .add(Slider::new(&mut edited.ambient_intensity, 0.0..=5.0))
                    .changed();
                ui.end_row();

                ui.label("HDR enabled");
                changed |= ui.checkbox(&mut hdr_enabled, "").changed();
                ui.end_row();

                ui.label("HDR intensity");
                let intensity_response = ui.add_enabled(
                    hdr_settings.path.is_some(),
                    Slider::new(&mut hdr_settings.intensity, 0.0..=5.0),
                );
                if intensity_response.changed() {
                    changed = true;
                }
                ui.end_row();

                ui.label("HDR file");
                ui.horizontal(|ui| {
                    let path_text = hdr_settings
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "None".to_string());
                    ui.label(path_text);
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if ui.button("Select...").clicked() {
                            if let Some(path) = FileDialog::new()
                                .add_filter("HDR images", &["hdr"])
                                .pick_file()
                            {
                                hdr_settings.path = Some(path);
                                hdr_enabled = true;
                                changed = true;
                            }
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        ui.add_enabled(false, egui::Button::new("Select..."));
                    }
                    if ui.button("Clear").clicked() {
                        hdr_settings.path = None;
                        changed = true;
                    }
                });
                ui.end_row();

                ui.label("Exposure");
                changed |= ui
                    .add(Slider::new(&mut grading.exposure, 0.0..=4.0))
                    .changed();
                ui.end_row();

                ui.label("Saturation");
                changed |= ui
                    .add(Slider::new(&mut grading.saturation, 0.0..=2.0))
                    .changed();
                ui.end_row();

                ui.label("Contrast");
                changed |= ui
                    .add(Slider::new(&mut grading.contrast, 0.0..=3.0))
                    .changed();
                ui.end_row();
            });

        hdr_settings.enabled = hdr_enabled;
        if hdr_settings.path.is_some() || hdr_enabled {
            edited.hdr = Some(hdr_settings);
        } else {
            edited.hdr = None;
        }
        edited.color_grading = grading;

        if changed && edited != component {
            actions.push(InspectorAction::UpdateEnvironment {
                entity,
                component: edited,
            });
        }
    });
}

fn show_point_light_section(
    ui: &mut egui::Ui,
    entity: Entity,
    mut light: PointLight,
    casts_shadow: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Point Light", |ui| {
        let mut changed = false;

        Grid::new("point_light_component_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |= light_color_editor(ui, "Color", &mut light.color);
                changed |= float_drag_value(
                    ui,
                    "Intensity",
                    &mut light.intensity,
                    0.1,
                    Some(0.0..=1000.0),
                );
                if light.intensity < 0.0 {
                    light.intensity = 0.0;
                }

                changed |= float_drag_value(ui, "Range", &mut light.range, 0.1, Some(0.0..=1000.0));
                if light.range < 0.0 {
                    light.range = 0.0;
                }

                shadow_checkbox_row(ui, entity, casts_shadow, actions);
            });

        if changed {
            actions.push(InspectorAction::UpdatePointLight { entity, light });
        }
    });
}

fn show_directional_light_section(
    ui: &mut egui::Ui,
    entity: Entity,
    mut light: DirectionalLight,
    casts_shadow: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Directional Light", |ui| {
        let mut changed = false;

        Grid::new("directional_light_component_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |= light_color_editor(ui, "Color", &mut light.color);
                changed |= float_drag_value(
                    ui,
                    "Intensity",
                    &mut light.intensity,
                    0.1,
                    Some(0.0..=1000.0),
                );
                if light.intensity < 0.0 {
                    light.intensity = 0.0;
                }

                changed |= float_drag_value(
                    ui,
                    "Shadow Size",
                    &mut light.shadow_size,
                    0.1,
                    Some(0.0..=500.0),
                );
                if light.shadow_size < 0.0 {
                    light.shadow_size = 0.0;
                }

                shadow_checkbox_row(ui, entity, casts_shadow, actions);
            });

        if changed {
            actions.push(InspectorAction::UpdateDirectionalLight { entity, light });
        }
    });
}

fn show_spot_light_section(
    ui: &mut egui::Ui,
    entity: Entity,
    mut light: SpotLight,
    casts_shadow: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Spot Light", |ui| {
        let mut changed = false;

        Grid::new("spot_light_component_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |= light_color_editor(ui, "Color", &mut light.color);
                changed |= float_drag_value(
                    ui,
                    "Intensity",
                    &mut light.intensity,
                    0.1,
                    Some(0.0..=1000.0),
                );
                if light.intensity < 0.0 {
                    light.intensity = 0.0;
                }

                changed |= float_drag_value(ui, "Range", &mut light.range, 0.1, Some(0.0..=1000.0));
                if light.range < 0.0 {
                    light.range = 0.0;
                }

                changed |= light_angle_editor(ui, "Inner Angle (deg)", &mut light.inner_angle);
                changed |= light_angle_editor(ui, "Outer Angle (deg)", &mut light.outer_angle);

                if light.outer_angle < light.inner_angle {
                    light.outer_angle = light.inner_angle;
                }

                shadow_checkbox_row(ui, entity, casts_shadow, actions);
            });

        if changed {
            actions.push(InspectorAction::UpdateSpotLight { entity, light });
        }
    });
}

fn show_script_section(
    ui: &mut egui::Ui,
    entity: Entity,
    script: &RuneScriptComponent,
) -> Option<InspectorAction> {
    let mut action = None;
    ui.collapsing("Script", |ui| {
        let source_label = match script.source() {
            RuneScriptSource::Inline { name, .. } => format!("Inline script: {}", name),
            RuneScriptSource::File { path } => format!("File: {}", path.display()),
        };
        ui.label(source_label);

        if ui.button("Edit").clicked() {
            action = Some(InspectorAction::EditScript {
                entity,
                component: script.clone(),
            });
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

fn float_drag_value(
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

fn light_color_editor(ui: &mut egui::Ui, label: &str, color: &mut Vec3) -> bool {
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

fn light_angle_editor(ui: &mut egui::Ui, label: &str, radians: &mut f32) -> bool {
    let mut degrees = radians.to_degrees();
    let changed = float_drag_value(ui, label, &mut degrees, 1.0, Some(0.0..=179.0));
    if changed {
        *radians = degrees.clamp(0.0, 179.0).to_radians();
    }
    changed
}

fn shadow_checkbox_row(
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
