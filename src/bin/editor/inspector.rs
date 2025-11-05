use egui::{
    color_picker::color_edit_button_rgb, color_picker::color_edit_button_rgba, color_picker::Alpha,
    ComboBox, DragValue, Grid, Slider,
};
use glam::{EulerRot, Quat, Vec3};
use std::cmp::Ordering;
use std::f32::consts::PI;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hecs::Entity;

use wgpu_cube::asset::{
    Handle, MaterialAsset, MaterialKind, MaterialTextureReference, MaterialTextureSlot,
    ShaderMaterialMetadata,
};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{Billboard, BillboardOrientation, ParticleRenderBlendMode};
use wgpu_cube::scene::{
    BoidsBehaviorConfig, CameraComponent, CameraProjection, CanCastShadow, DirectionalLight,
    EnvironmentComponent, MeshComponent, OptimizedBoidsBehaviorConfig, ParticleBehaviorConfig,
    ParticleBehaviorPreset, ParticleColorGradient, ParticleColorKeyframe, ParticleEmissionShape,
    ParticleEmitterComponent, ParticleFloatRange, ParticleSizeCurve, ParticleSizeKeyframe,
    ParticleSystemComponent, ParticleVec3Range, PhysicsBehaviorConfig, PointLight, SpotLight,
    StarfieldBehaviorConfig, Transform,
};
use wgpu_cube::scripting::{RuneScriptComponent, RuneScriptSource};
use wgpu_cube::{InspectorMaterial, SceneEntityComponentsSummary, SceneEntityInspectorData};

#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;

#[derive(Clone)]
pub enum InspectorAction {
    EditScript {
        entity: Entity,
        component: RuneScriptComponent,
    },
    EditShader {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        metadata: ShaderMaterialMetadata,
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
        handle: Handle<MaterialAsset>,
        material: Material,
    },
    CreateShaderMaterial {
        entity: Entity,
        source: Handle<MaterialAsset>,
    },
    SetMaterialKind {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        kind: MaterialKind,
    },
    AssignShaderSource {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        shader_path: PathBuf,
    },
    CreateShaderSource {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        suggested_stem: String,
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
    UpdateParticleEmitter {
        entity: Entity,
        component: ParticleEmitterComponent,
    },
    UpdateParticleBehavior {
        entity: Entity,
        behavior: ParticleBehaviorPreset,
        config: ParticleBehaviorConfig,
    },
    SetBillboard {
        entity: Entity,
        billboard: Option<Billboard>,
    },
    AddScript {
        entity: Entity,
    },
    ChangeScriptSource {
        entity: Entity,
        script_path: PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MaterialKindChoice {
    Pbr,
    Shader,
}

pub fn show_entity_inspector(
    ui: &mut egui::Ui,
    data: &SceneEntityInspectorData,
    content_root: Option<&Path>,
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

    if let Some(material) = data.components.material.clone() {
        begin_section(ui, &mut first_section);
        show_material_section(
            ui,
            data.entity,
            &data.name,
            material,
            content_root,
            &mut actions,
        );
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
        let emitter = data.components.particle_emitter.clone();
        show_particle_system_section(
            ui,
            data.entity,
            component,
            emitter,
            data.components.billboard,
            &mut actions,
        );
    }

    if let Some(script) = data.components.script.clone() {
        begin_section(ui, &mut first_section);
        if let Some(action) = show_script_section(ui, data.entity, &script) {
            actions.push(action);
        }
    } else {
        begin_section(ui, &mut first_section);
        if let Some(action) = show_add_script_section(ui, data.entity) {
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
    entity_name: &str,
    material_data: InspectorMaterial,
    content_root: Option<&Path>,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Material", |ui| {
        let mut material = material_data.material;
        let mut changed = false;

        let mut selected_kind = if matches!(material_data.kind, MaterialKind::Shader(_)) {
            MaterialKindChoice::Shader
        } else {
            MaterialKindChoice::Pbr
        };
        let initial_kind = selected_kind;

        ui.horizontal(|ui| {
            ui.label("Kind");
            ComboBox::from_id_salt(("material_kind", material_data.handle.index()))
                .selected_text(match selected_kind {
                    MaterialKindChoice::Pbr => "PBR",
                    MaterialKindChoice::Shader => "Shader",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected_kind, MaterialKindChoice::Pbr, "PBR");
                    ui.selectable_value(&mut selected_kind, MaterialKindChoice::Shader, "Shader");
                });

            if matches!(material_data.kind, MaterialKind::Pbr)
                && ui
                    .button("Create shader material")
                    .on_hover_text(
                        "Duplicate this material as a shader asset using the default template.",
                    )
                    .clicked()
            {
                actions.push(InspectorAction::CreateShaderMaterial {
                    entity,
                    source: material_data.handle,
                });
            }
        });

        if selected_kind != initial_kind {
            let kind = match selected_kind {
                MaterialKindChoice::Pbr => MaterialKind::Pbr,
                MaterialKindChoice::Shader => match &material_data.kind {
                    MaterialKind::Shader(metadata) => MaterialKind::Shader(metadata.clone()),
                    MaterialKind::Pbr => {
                        MaterialKind::Shader(ShaderMaterialMetadata::default_template())
                    }
                },
            };
            actions.push(InspectorAction::SetMaterialKind {
                entity,
                handle: material_data.handle,
                kind,
            });
        }

        if matches!(selected_kind, MaterialKindChoice::Shader) {
            ui.add_space(4.0);
            let shader_metadata = match &material_data.kind {
                MaterialKind::Shader(metadata) => Some(metadata),
                MaterialKind::Pbr => None,
            };
            show_shader_controls(
                ui,
                entity,
                entity_name,
                &material_data,
                shader_metadata,
                content_root,
                actions,
            );
        }

        ui.add_space(4.0);

        let reference_for = |slot: MaterialTextureSlot| -> Option<&MaterialTextureReference> {
            material_data
                .textures
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, reference)| reference)
        };

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

                if let Some(reference) = reference_for(MaterialTextureSlot::BaseColor) {
                    if let Some(name) = reference.display_name() {
                        ui.label("   ↳ Name");
                        ui.monospace(name.to_string());
                        ui.end_row();
                    }
                    if let Some(path) = reference.canonical_path() {
                        ui.label("   ↳ Path");
                        ui.monospace(path.display().to_string());
                        ui.end_row();
                    }
                }

                ui.label("Metallic/Roughness texture");
                ui.monospace(format!("{}", material.metallic_roughness_texture));
                ui.end_row();

                if let Some(reference) = reference_for(MaterialTextureSlot::MetallicRoughness) {
                    if let Some(name) = reference.display_name() {
                        ui.label("   ↳ Name");
                        ui.monospace(name.to_string());
                        ui.end_row();
                    }
                    if let Some(path) = reference.canonical_path() {
                        ui.label("   ↳ Path");
                        ui.monospace(path.display().to_string());
                        ui.end_row();
                    }
                }

                ui.label("Normal texture");
                ui.monospace(format!("{}", material.normal_texture));
                ui.end_row();

                if let Some(reference) = reference_for(MaterialTextureSlot::Normal) {
                    if let Some(name) = reference.display_name() {
                        ui.label("   ↳ Name");
                        ui.monospace(name.to_string());
                        ui.end_row();
                    }
                    if let Some(path) = reference.canonical_path() {
                        ui.label("   ↳ Path");
                        ui.monospace(path.display().to_string());
                        ui.end_row();
                    }
                }

                ui.label("Emissive texture");
                ui.monospace(format!("{}", material.emissive_texture));
                ui.end_row();

                if let Some(reference) = reference_for(MaterialTextureSlot::Emissive) {
                    if let Some(name) = reference.display_name() {
                        ui.label("   ↳ Name");
                        ui.monospace(name.to_string());
                        ui.end_row();
                    }
                    if let Some(path) = reference.canonical_path() {
                        ui.label("   ↳ Path");
                        ui.monospace(path.display().to_string());
                        ui.end_row();
                    }
                }

                ui.label("Occlusion texture");
                ui.monospace(format!("{}", material.occlusion_texture));
                ui.end_row();

                if let Some(reference) = reference_for(MaterialTextureSlot::Occlusion) {
                    if let Some(name) = reference.display_name() {
                        ui.label("   ↳ Name");
                        ui.monospace(name.to_string());
                        ui.end_row();
                    }
                    if let Some(path) = reference.canonical_path() {
                        ui.label("   ↳ Path");
                        ui.monospace(path.display().to_string());
                        ui.end_row();
                    }
                }
            });

        if changed {
            actions.push(InspectorAction::UpdateMaterial {
                entity,
                handle: material_data.handle,
                material,
            });
        }
    });
}

fn show_particle_system_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: ParticleSystemComponent,
    emitter: Option<ParticleEmitterComponent>,
    billboard: Option<Billboard>,
    actions: &mut Vec<InspectorAction>,
) {
    ui.collapsing("Particle System", |ui| {
        let mut system_component = component;
        let mut emitter_component = emitter;
        let mut emitter_changed = false;
        let mut system_changed = false;
        let mut behavior_action: Option<(ParticleBehaviorPreset, ParticleBehaviorConfig)> = None;
        let mut billboard_component = billboard;
        let mut billboard_changed = false;

        ui.collapsing("System", |ui| {
            Grid::new("particle_system_component_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Billboard");
                    ui.horizontal(|ui| {
                        let mut enabled = billboard_component.is_some();
                        if ui.checkbox(&mut enabled, "Enabled").changed() {
                            billboard_changed = true;
                            if enabled {
                                let current = billboard_component.unwrap_or_else(|| {
                                    Billboard::new(BillboardOrientation::FaceCamera)
                                });
                                billboard_component = Some(current);
                            } else {
                                billboard_component = None;
                            }
                        }

                        match billboard_component {
                            Some(component) => {
                                ui.label(format!(
                                    "Orientation: {:?}{}",
                                    component.orientation,
                                    if component.lit { ", Lit" } else { "" }
                                ));
                            }
                            None => {
                                ui.label("Disabled");
                            }
                        }
                    });
                    ui.end_row();

                    let mut spawn_rate = emitter_component
                        .as_ref()
                        .map(|component| component.spawn_rate)
                        .unwrap_or(system_component.spawn_rate);
                    if float_drag_value(
                        ui,
                        "Spawn rate",
                        &mut spawn_rate,
                        0.1,
                        Some(0.0..=10_000.0),
                    ) {
                        spawn_rate = spawn_rate.max(0.0);
                        if (spawn_rate - system_component.spawn_rate).abs() > f32::EPSILON {
                            system_changed = true;
                        }
                        system_component.spawn_rate = spawn_rate;
                        if let Some(component) = emitter_component.as_mut() {
                            if (component.spawn_rate - spawn_rate).abs() > f32::EPSILON {
                                component.spawn_rate = spawn_rate;
                                emitter_changed = true;
                            }
                        }
                    } else if let Some(component) = emitter_component.as_ref() {
                        // Keep the system value in sync with the emitter when no edit occurred.
                        system_component.spawn_rate = component.spawn_rate;
                    }

                    ui.end_row();

                    ui.label("Blend mode");
                    let mut render_mode = system_component.render_mode;
                    ComboBox::from_id_salt("particle_render_mode_combo")
                        .selected_text(render_mode.display_name())
                        .show_ui(ui, |ui| {
                            for variant in ParticleRenderBlendMode::variants() {
                                if ui
                                    .selectable_label(
                                        render_mode == variant,
                                        variant.display_name(),
                                    )
                                    .clicked()
                                {
                                    render_mode = variant;
                                }
                            }
                        });
                    if render_mode != system_component.render_mode {
                        system_component.render_mode = render_mode;
                        system_changed = true;
                    }
                    ui.end_row();
                });
        });

        ui.add_space(6.0);
        ui.collapsing("Behavior", |ui| {
            let mut preset = system_component.behavior;
            let mut config = system_component.behavior_config.clone();
            let mut section_changed = false;
            let mut preset_changed = false;
            Grid::new("particle_behavior_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Behavior preset");
                    ComboBox::from_id_salt("particle_behavior_combo")
                        .selected_text(preset.display_name())
                        .show_ui(ui, |ui| {
                            for variant in ParticleBehaviorPreset::variants() {
                                if ui
                                    .selectable_label(preset == variant, variant.display_name())
                                    .clicked()
                                {
                                    preset = variant;
                                    preset_changed = true;
                                }
                            }
                        });
                    ui.end_row();
                });
            section_changed |= preset_changed;

            if preset != system_component.behavior {
                config = ParticleBehaviorConfig::from_preset(preset);
                section_changed = true;
            }

            section_changed |= match &mut config {
                ParticleBehaviorConfig::Physics(physics) => physics_behavior_editor(ui, physics),
                ParticleBehaviorConfig::Starfield(starfield) => {
                    starfield_behavior_editor(ui, starfield)
                }
                ParticleBehaviorConfig::Boids(boids) => {
                    edit_boids_like_config(ui, "boids_behavior_grid", boids)
                }
                ParticleBehaviorConfig::OptimizedBoids(boids) => {
                    edit_boids_like_config(ui, "optimized_boids_behavior_grid", boids)
                }
            };

            if section_changed
                && (preset != system_component.behavior
                    || config != system_component.behavior_config)
            {
                config = config.ensure_variant(preset);
                system_component.behavior = preset;
                system_component.behavior_config = config.clone();
                behavior_action = Some((preset, config));
            }
        });

        ui.add_space(6.0);
        ui.collapsing("Emitter", |ui| match emitter_component.as_mut() {
            Some(component) => {
                Grid::new("particle_emitter_component_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Burst count");
                        let mut burst_enabled = component.burst_count.is_some();
                        let mut burst_value = component.burst_count.unwrap_or(1);
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut burst_enabled, "Enabled").changed() {
                                emitter_changed = true;
                            }
                            if burst_enabled
                                && ui
                                    .add(
                                        DragValue::new(&mut burst_value)
                                            .range(1..=1_000_000)
                                            .speed(1.0),
                                    )
                                    .changed()
                            {
                                emitter_changed = true;
                            }
                        });
                        ui.end_row();
                        if burst_enabled {
                            let new_value = burst_value.max(1);
                            if component.burst_count != Some(new_value) {
                                component.burst_count = Some(new_value);
                                emitter_changed = true;
                            }
                        } else if component.burst_count.is_some() {
                            component.burst_count = None;
                            emitter_changed = true;
                        }

                        let mut position = Vec3::from_array(component.position);
                        if vec3_editor(ui, "Position", &mut position) {
                            component.position = position.to_array();
                            emitter_changed = true;
                        }

                        if emission_shape_editor(ui, &mut component.emission_shape) {
                            emitter_changed = true;
                        }

                        let mut velocity_min =
                            Vec3::from_array(component.initial_velocity_range.min);
                        let mut velocity_max =
                            Vec3::from_array(component.initial_velocity_range.max);
                        let mut velocity_changed = false;
                        if vec3_editor(ui, "Velocity min", &mut velocity_min) {
                            velocity_changed = true;
                        }
                        if vec3_editor(ui, "Velocity max", &mut velocity_max) {
                            velocity_changed = true;
                        }
                        if velocity_changed {
                            let min = velocity_min.to_array();
                            let mut max = velocity_max.to_array();
                            for i in 0..3 {
                                if max[i] < min[i] {
                                    max[i] = min[i];
                                }
                            }
                            component.initial_velocity_range = ParticleVec3Range::new(min, max);
                            emitter_changed = true;
                        }

                        let mut scale_min = Vec3::from_array(component.initial_scale_range.min);
                        let mut scale_max = Vec3::from_array(component.initial_scale_range.max);
                        let mut scale_changed = false;
                        if vec3_editor(ui, "Scale min", &mut scale_min) {
                            scale_changed = true;
                        }
                        if vec3_editor(ui, "Scale max", &mut scale_max) {
                            scale_changed = true;
                        }
                        if scale_changed {
                            let min = scale_min.to_array();
                            let mut max = scale_max.to_array();
                            for i in 0..3 {
                                if max[i] < min[i] {
                                    max[i] = min[i];
                                }
                            }
                            component.initial_scale_range = ParticleVec3Range::new(min, max);
                            emitter_changed = true;
                        }

                        let mut lifetime_min = component.lifetime_range.min;
                        let mut lifetime_max = component.lifetime_range.max;
                        if float_drag_value(
                            ui,
                            "Lifetime min",
                            &mut lifetime_min,
                            0.01,
                            Some(0.0..=1_000.0),
                        ) {
                            emitter_changed = true;
                        }
                        if float_drag_value(
                            ui,
                            "Lifetime max",
                            &mut lifetime_max,
                            0.01,
                            Some(0.0..=1_000.0),
                        ) {
                            emitter_changed = true;
                        }
                        if lifetime_max < lifetime_min {
                            lifetime_max = lifetime_min;
                            emitter_changed = true;
                        }
                        component.lifetime_range =
                            ParticleFloatRange::new(lifetime_min, lifetime_max);

                        ui.label("Auto respawn");
                        if ui
                            .checkbox(&mut component.auto_respawn, "Enabled")
                            .changed()
                        {
                            emitter_changed = true;
                        }
                        ui.end_row();

                        let mut radial_min = component.radial_velocity.min;
                        let mut radial_max = component.radial_velocity.max;
                        if float_drag_value(ui, "Radial velocity min", &mut radial_min, 0.01, None)
                        {
                            emitter_changed = true;
                        }
                        if float_drag_value(ui, "Radial velocity max", &mut radial_max, 0.01, None)
                        {
                            emitter_changed = true;
                        }
                        if radial_max < radial_min {
                            radial_max = radial_min;
                            emitter_changed = true;
                        }
                        component.radial_velocity = ParticleFloatRange::new(radial_min, radial_max);
                    });

                ui.add_space(6.0);
                ui.label("Color gradient");
                if color_gradient_editor(ui, &mut component.color_gradient) {
                    emitter_changed = true;
                }

                ui.add_space(6.0);
                ui.label("Size curve");
                if size_curve_editor(ui, &mut component.size_curve) {
                    emitter_changed = true;
                }
            }
            None => {
                ui.label("No ParticleEmitterComponent on this entity.");
            }
        });

        if system_changed {
            actions.push(InspectorAction::UpdateParticleSystem {
                entity,
                component: system_component.clone(),
            });
        }

        if emitter_changed {
            if let Some(component) = emitter_component {
                actions.push(InspectorAction::UpdateParticleEmitter { entity, component });
            }
        }

        if let Some((behavior, config)) = behavior_action {
            actions.push(InspectorAction::UpdateParticleBehavior {
                entity,
                behavior,
                config,
            });
        }

        if billboard_changed {
            actions.push(InspectorAction::SetBillboard {
                entity,
                billboard: billboard_component,
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

fn physics_behavior_editor(ui: &mut egui::Ui, config: &mut PhysicsBehaviorConfig) -> bool {
    let mut changed = false;
    Grid::new("physics_behavior_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            changed |= float_drag_value(ui, "Drag", &mut config.drag, 0.01, Some(0.0..=10.0));
            changed |= float_drag_value(
                ui,
                "Turbulence strength",
                &mut config.turbulence_strength,
                0.01,
                Some(0.0..=10.0),
            );
            changed |= float_drag_value(
                ui,
                "Turbulence frequency",
                &mut config.turbulence_frequency,
                0.01,
                Some(0.0..=10.0),
            );
            let mut gravity = Vec3::from_array(config.gravity);
            if vec3_editor(ui, "Gravity", &mut gravity) {
                config.gravity = gravity.to_array();
                changed = true;
            }
            changed |= float_drag_value(ui, "Ground level", &mut config.ground_level, 0.01, None);
            changed |= float_drag_value(
                ui,
                "Bounce factor",
                &mut config.bounce_factor,
                0.01,
                Some(0.0..=1.0),
            );
            changed |= float_drag_value(
                ui,
                "Velocity damping",
                &mut config.velocity_damping,
                0.01,
                Some(0.0..=10.0),
            );
        });

    if config.drag < 0.0 {
        config.drag = 0.0;
        changed = true;
    }
    if config.turbulence_strength < 0.0 {
        config.turbulence_strength = 0.0;
        changed = true;
    }
    if config.turbulence_frequency < 0.0 {
        config.turbulence_frequency = 0.0;
        changed = true;
    }
    if config.bounce_factor < 0.0 {
        config.bounce_factor = 0.0;
        changed = true;
    }
    if config.velocity_damping < 0.0 {
        config.velocity_damping = 0.0;
        changed = true;
    }

    changed
}

fn starfield_behavior_editor(ui: &mut egui::Ui, config: &mut StarfieldBehaviorConfig) -> bool {
    let mut changed = false;
    Grid::new("starfield_behavior_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            changed |= float_drag_value(
                ui,
                "Near plane",
                &mut config.near_plane,
                0.01,
                Some(0.0..=1_000.0),
            );
            changed |= float_drag_value(
                ui,
                "Far plane",
                &mut config.far_plane,
                0.01,
                Some(0.0..=10_000.0),
            );
            changed |= float_drag_value(
                ui,
                "Far reset band",
                &mut config.far_reset_band,
                0.01,
                Some(0.0..=10_000.0),
            );
            changed |= float_drag_value(
                ui,
                "Field half size",
                &mut config.field_half_size,
                0.1,
                Some(0.0..=10_000.0),
            );
            changed |= float_drag_value(
                ui,
                "Minimum radius",
                &mut config.min_radius,
                0.01,
                Some(0.0..=100.0),
            );
        });

    if config.near_plane < 0.0 {
        config.near_plane = 0.0;
        changed = true;
    }
    if config.far_plane <= config.near_plane {
        config.far_plane = config.near_plane + 0.01;
        changed = true;
    }
    if config.far_reset_band < 0.0 {
        config.far_reset_band = 0.0;
        changed = true;
    }
    if config.field_half_size < 0.0 {
        config.field_half_size = 0.0;
        changed = true;
    }
    if config.min_radius < 0.0 {
        config.min_radius = 0.0;
        changed = true;
    }

    changed
}

struct BoidsConfigMut<'a> {
    separation_radius: &'a mut f32,
    alignment_radius: &'a mut f32,
    cohesion_radius: &'a mut f32,
    separation_weight: &'a mut f32,
    alignment_weight: &'a mut f32,
    cohesion_weight: &'a mut f32,
    max_speed: &'a mut f32,
    max_force: &'a mut f32,
    bounds: &'a mut f32,
    particle_count: &'a mut u32,
}

trait BoidsConfigAccess {
    fn as_mut_fields(&mut self) -> BoidsConfigMut<'_>;
}

impl BoidsConfigAccess for BoidsBehaviorConfig {
    fn as_mut_fields(&mut self) -> BoidsConfigMut<'_> {
        BoidsConfigMut {
            separation_radius: &mut self.separation_radius,
            alignment_radius: &mut self.alignment_radius,
            cohesion_radius: &mut self.cohesion_radius,
            separation_weight: &mut self.separation_weight,
            alignment_weight: &mut self.alignment_weight,
            cohesion_weight: &mut self.cohesion_weight,
            max_speed: &mut self.max_speed,
            max_force: &mut self.max_force,
            bounds: &mut self.bounds,
            particle_count: &mut self.particle_count,
        }
    }
}

impl BoidsConfigAccess for OptimizedBoidsBehaviorConfig {
    fn as_mut_fields(&mut self) -> BoidsConfigMut<'_> {
        BoidsConfigMut {
            separation_radius: &mut self.separation_radius,
            alignment_radius: &mut self.alignment_radius,
            cohesion_radius: &mut self.cohesion_radius,
            separation_weight: &mut self.separation_weight,
            alignment_weight: &mut self.alignment_weight,
            cohesion_weight: &mut self.cohesion_weight,
            max_speed: &mut self.max_speed,
            max_force: &mut self.max_force,
            bounds: &mut self.bounds,
            particle_count: &mut self.particle_count,
        }
    }
}

fn edit_boids_like_config<T: BoidsConfigAccess>(
    ui: &mut egui::Ui,
    id: &str,
    config: &mut T,
) -> bool {
    let fields = config.as_mut_fields();
    edit_boids_fields(ui, id, fields)
}

fn edit_boids_fields(ui: &mut egui::Ui, id: &str, fields: BoidsConfigMut<'_>) -> bool {
    let mut changed = false;
    Grid::new(id).num_columns(2).striped(true).show(ui, |ui| {
        changed |= float_drag_value(
            ui,
            "Separation radius",
            fields.separation_radius,
            0.01,
            Some(0.0..=1_000.0),
        );
        changed |= float_drag_value(
            ui,
            "Alignment radius",
            fields.alignment_radius,
            0.01,
            Some(0.0..=1_000.0),
        );
        changed |= float_drag_value(
            ui,
            "Cohesion radius",
            fields.cohesion_radius,
            0.01,
            Some(0.0..=1_000.0),
        );
        changed |= float_drag_value(
            ui,
            "Separation weight",
            fields.separation_weight,
            0.01,
            Some(0.0..=10.0),
        );
        changed |= float_drag_value(
            ui,
            "Alignment weight",
            fields.alignment_weight,
            0.01,
            Some(0.0..=10.0),
        );
        changed |= float_drag_value(
            ui,
            "Cohesion weight",
            fields.cohesion_weight,
            0.01,
            Some(0.0..=10.0),
        );
        changed |= float_drag_value(ui, "Max speed", fields.max_speed, 0.01, Some(0.0..=1_000.0));
        changed |= float_drag_value(ui, "Max force", fields.max_force, 0.01, Some(0.0..=1_000.0));
        changed |= float_drag_value(ui, "Bounds", fields.bounds, 0.1, Some(0.0..=10_000.0));
        changed |= u32_drag_value(
            ui,
            "Particle count",
            fields.particle_count,
            1.0,
            Some(1..=1_000_000),
        );
    });

    if *fields.separation_radius < 0.0 {
        *fields.separation_radius = 0.0;
        changed = true;
    }
    if *fields.alignment_radius < 0.0 {
        *fields.alignment_radius = 0.0;
        changed = true;
    }
    if *fields.cohesion_radius < 0.0 {
        *fields.cohesion_radius = 0.0;
        changed = true;
    }
    if *fields.separation_weight < 0.0 {
        *fields.separation_weight = 0.0;
        changed = true;
    }
    if *fields.alignment_weight < 0.0 {
        *fields.alignment_weight = 0.0;
        changed = true;
    }
    if *fields.cohesion_weight < 0.0 {
        *fields.cohesion_weight = 0.0;
        changed = true;
    }
    if *fields.max_speed < 0.0 {
        *fields.max_speed = 0.0;
        changed = true;
    }
    if *fields.max_force < 0.0 {
        *fields.max_force = 0.0;
        changed = true;
    }
    if *fields.bounds < 0.0 {
        *fields.bounds = 0.0;
        changed = true;
    }
    if *fields.particle_count == 0 {
        *fields.particle_count = 1;
        changed = true;
    }

    changed
}

fn u32_drag_value(
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

fn emission_shape_editor(ui: &mut egui::Ui, shape: &mut ParticleEmissionShape) -> bool {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum EmissionShapeKind {
        Point,
        Sphere,
        Box,
        Cone,
        Disc,
        Ring,
        RadialBurst,
    }

    fn shape_kind(shape: &ParticleEmissionShape) -> EmissionShapeKind {
        match shape {
            ParticleEmissionShape::Point => EmissionShapeKind::Point,
            ParticleEmissionShape::Sphere { .. } => EmissionShapeKind::Sphere,
            ParticleEmissionShape::Box { .. } => EmissionShapeKind::Box,
            ParticleEmissionShape::Cone { .. } => EmissionShapeKind::Cone,
            ParticleEmissionShape::Disc { .. } => EmissionShapeKind::Disc,
            ParticleEmissionShape::Ring { .. } => EmissionShapeKind::Ring,
            ParticleEmissionShape::RadialBurst => EmissionShapeKind::RadialBurst,
        }
    }

    fn kind_label(kind: EmissionShapeKind) -> &'static str {
        match kind {
            EmissionShapeKind::Point => "Point",
            EmissionShapeKind::Sphere => "Sphere",
            EmissionShapeKind::Box => "Box",
            EmissionShapeKind::Cone => "Cone",
            EmissionShapeKind::Disc => "Disc",
            EmissionShapeKind::Ring => "Ring",
            EmissionShapeKind::RadialBurst => "Radial Burst",
        }
    }

    fn default_shape(kind: EmissionShapeKind) -> ParticleEmissionShape {
        match kind {
            EmissionShapeKind::Point => ParticleEmissionShape::Point,
            EmissionShapeKind::Sphere => ParticleEmissionShape::Sphere { radius: 1.0 },
            EmissionShapeKind::Box => ParticleEmissionShape::Box {
                half_extents: [1.0, 1.0, 1.0],
            },
            EmissionShapeKind::Cone => ParticleEmissionShape::Cone {
                angle: (45.0_f32).to_radians(),
                radius: 1.0,
            },
            EmissionShapeKind::Disc => ParticleEmissionShape::Disc { radius: 1.0 },
            EmissionShapeKind::Ring => ParticleEmissionShape::Ring {
                radius: 1.0,
                thickness: 0.1,
            },
            EmissionShapeKind::RadialBurst => ParticleEmissionShape::RadialBurst,
        }
    }

    let mut changed = false;
    let mut kind = shape_kind(shape);

    ui.label("Emission shape");
    ComboBox::from_id_salt("particle_emission_shape")
        .selected_text(kind_label(kind))
        .show_ui(ui, |ui| {
            for candidate in [
                EmissionShapeKind::Point,
                EmissionShapeKind::Sphere,
                EmissionShapeKind::Box,
                EmissionShapeKind::Cone,
                EmissionShapeKind::Disc,
                EmissionShapeKind::Ring,
                EmissionShapeKind::RadialBurst,
            ] {
                if ui
                    .selectable_label(kind == candidate, kind_label(candidate))
                    .clicked()
                {
                    kind = candidate;
                }
            }
        });
    ui.end_row();

    if kind != shape_kind(shape) {
        *shape = default_shape(kind);
        changed = true;
    }

    match shape {
        ParticleEmissionShape::Sphere { radius } => {
            if float_drag_value(ui, "Sphere radius", radius, 0.01, Some(0.0..=10_000.0)) {
                if *radius < 0.0 {
                    *radius = 0.0;
                }
                changed = true;
            }
        }
        ParticleEmissionShape::Box { half_extents } => {
            let mut extents = Vec3::from_array(*half_extents);
            if vec3_editor(ui, "Half extents", &mut extents) {
                *half_extents = extents.to_array();
                changed = true;
            }
        }
        ParticleEmissionShape::Cone { angle, radius } => {
            let mut angle_degrees = angle.to_degrees();
            if float_drag_value(
                ui,
                "Cone angle (deg)",
                &mut angle_degrees,
                0.5,
                Some(0.0..=179.0),
            ) {
                *angle = angle_degrees.clamp(0.0, 179.0).to_radians();
                changed = true;
            }
            if float_drag_value(ui, "Cone radius", radius, 0.01, Some(0.0..=10_000.0)) {
                if *radius < 0.0 {
                    *radius = 0.0;
                }
                changed = true;
            }
        }
        ParticleEmissionShape::Disc { radius } => {
            if float_drag_value(ui, "Disc radius", radius, 0.01, Some(0.0..=10_000.0)) {
                if *radius < 0.0 {
                    *radius = 0.0;
                }
                changed = true;
            }
        }
        ParticleEmissionShape::Ring { radius, thickness } => {
            if float_drag_value(ui, "Ring radius", radius, 0.01, Some(0.0..=10_000.0)) {
                if *radius < 0.0 {
                    *radius = 0.0;
                }
                changed = true;
            }
            if float_drag_value(ui, "Ring thickness", thickness, 0.01, Some(0.0..=10_000.0)) {
                if *thickness < 0.0 {
                    *thickness = 0.0;
                }
                changed = true;
            }
        }
        ParticleEmissionShape::Point | ParticleEmissionShape::RadialBurst => {
            // No additional parameters
        }
    }

    changed
}

fn color_gradient_editor(ui: &mut egui::Ui, gradient: &mut ParticleColorGradient) -> bool {
    let mut changed = false;
    let mut remove_index = None;

    for (index, keyframe) in gradient.keyframes.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Keyframe {}", index + 1));
                    if ui.small_button("Remove").clicked() {
                        remove_index = Some(index);
                    }
                });
                Grid::new(format!("color_keyframe_grid_{index}"))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Color");
                        let mut color = egui::Rgba::from_rgba_unmultiplied(
                            keyframe.color[0],
                            keyframe.color[1],
                            keyframe.color[2],
                            keyframe.color[3],
                        );
                        if color_edit_button_rgba(ui, &mut color, Alpha::OnlyBlend).changed() {
                            keyframe.color = color.to_array();
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Time");
                        let mut time = keyframe.time;
                        if ui.add(DragValue::new(&mut time).speed(0.01)).changed() {
                            keyframe.time = time.max(0.0);
                            changed = true;
                        }
                        ui.end_row();
                    });
            });
        });
    }

    if let Some(index) = remove_index {
        if gradient.keyframes.len() > 1 {
            gradient.keyframes.remove(index);
        } else if let Some(first) = gradient.keyframes.first_mut() {
            *first = ParticleColorKeyframe {
                color: [1.0, 1.0, 1.0, 1.0],
                time: 0.0,
            };
        }
        changed = true;
    }

    if ui.button("Add keyframe").clicked() {
        let (color, time) = gradient
            .keyframes
            .last()
            .map(|key| (key.color, key.time + 0.1))
            .unwrap_or(([1.0, 1.0, 1.0, 1.0], 0.0));
        gradient
            .keyframes
            .push(ParticleColorKeyframe { color, time });
        changed = true;
    }

    if changed {
        gradient
            .keyframes
            .sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(Ordering::Equal));
    }

    changed
}

fn size_curve_editor(ui: &mut egui::Ui, curve: &mut ParticleSizeCurve) -> bool {
    let mut changed = false;
    let mut remove_index = None;

    for (index, keyframe) in curve.keyframes.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Keyframe {}", index + 1));
                    if ui.small_button("Remove").clicked() {
                        remove_index = Some(index);
                    }
                });
                Grid::new(format!("size_keyframe_grid_{index}"))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        if float_drag_value(
                            ui,
                            "Size",
                            &mut keyframe.size,
                            0.01,
                            Some(0.0..=1_000.0),
                        ) {
                            if keyframe.size < 0.0 {
                                keyframe.size = 0.0;
                            }
                            changed = true;
                        }

                        ui.label("Time");
                        let mut time = keyframe.time;
                        if ui.add(DragValue::new(&mut time).speed(0.01)).changed() {
                            keyframe.time = time.max(0.0);
                            changed = true;
                        }
                        ui.end_row();
                    });
            });
        });
    }

    if let Some(index) = remove_index {
        if curve.keyframes.len() > 1 {
            curve.keyframes.remove(index);
        } else if let Some(first) = curve.keyframes.first_mut() {
            *first = ParticleSizeKeyframe {
                size: 1.0,
                time: 0.0,
            };
        }
        changed = true;
    }

    if ui.button("Add keyframe").clicked() {
        let (size, time) = curve
            .keyframes
            .last()
            .map(|key| (key.size, key.time + 0.1))
            .unwrap_or((1.0, 0.0));
        curve.keyframes.push(ParticleSizeKeyframe { size, time });
        changed = true;
    }

    if changed {
        curve
            .keyframes
            .sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(Ordering::Equal));
    }

    changed
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

        ui.separator();
        ui.label("Select existing script:");

        let available_scripts = list_available_scripts();
        if available_scripts.is_empty() {
            ui.label("No script files found in scripts/");
        } else {
            let current_path = match script.source() {
                RuneScriptSource::File { path } => Some(path.clone()),
                _ => None,
            };

            for script_path in available_scripts {
                let file_name = script_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                let is_current = current_path.as_ref() == Some(&script_path);
                let button_text = if is_current {
                    format!("● {}", file_name)
                } else {
                    file_name.to_string()
                };

                if ui.button(button_text).clicked() && !is_current {
                    action = Some(InspectorAction::ChangeScriptSource {
                        entity,
                        script_path,
                    });
                }
            }
        }
    });
    action
}

fn show_add_script_section(ui: &mut egui::Ui, entity: Entity) -> Option<InspectorAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label("Script");
        if ui.button("Add Script").clicked() {
            action = Some(InspectorAction::AddScript { entity });
        }
    });
    action
}

#[cfg(not(target_arch = "wasm32"))]
struct ScriptListCache {
    scripts: Vec<PathBuf>,
    last_update: Instant,
}

#[cfg(not(target_arch = "wasm32"))]
static SCRIPT_LIST_CACHE: Mutex<Option<ScriptListCache>> = Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
const CACHE_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[cfg(not(target_arch = "wasm32"))]
fn list_available_scripts() -> Vec<PathBuf> {
    // Check cache first
    if let Ok(mut cache) = SCRIPT_LIST_CACHE.lock() {
        if let Some(cached) = cache.as_ref() {
            if cached.last_update.elapsed() < CACHE_REFRESH_INTERVAL {
                return cached.scripts.clone();
            }
        }

        // Cache miss or expired, refresh from filesystem
        let scripts_dir = PathBuf::from("scripts");
        let mut scripts = Vec::new();

        if scripts_dir.exists() && scripts_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&scripts_dir) {
                for entry in entries.flatten() {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_file() {
                            let path = entry.path();
                            if path.extension().and_then(|s| s.to_str()) == Some("rn") {
                                scripts.push(path);
                            }
                        }
                    }
                }
            }
            scripts.sort();
        }

        // Update cache
        *cache = Some(ScriptListCache {
            scripts: scripts.clone(),
            last_update: Instant::now(),
        });

        scripts
    } else {
        // Fallback if mutex is poisoned
        Vec::new()
    }
}

#[cfg(target_arch = "wasm32")]
fn list_available_scripts() -> Vec<PathBuf> {
    // File system access is not available on wasm32
    Vec::new()
}

fn show_shader_controls(
    ui: &mut egui::Ui,
    entity: Entity,
    entity_name: &str,
    material: &InspectorMaterial,
    metadata: Option<&ShaderMaterialMetadata>,
    content_root: Option<&Path>,
    actions: &mut Vec<InspectorAction>,
) {
    ui.group(|ui| {
        ui.label("Shader source");
        let display_path = shader_path_display(metadata, content_root);
        ui.monospace(display_path);

        let has_root = content_root.is_some();
        ui.horizontal(|ui| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut dialog = FileDialog::new().add_filter("WGSL Shaders", &["wgsl"]);
                if let Some(root) = content_root {
                    let shaders_dir = root.join("shaders");
                    dialog = dialog.set_directory(shaders_dir);
                }
                let select_button = ui
                    .add_enabled(has_root, egui::Button::new("Select shader..."))
                    .on_hover_text(
                        "Assign an existing WGSL shader file from the project contents.",
                    );
                if select_button.clicked() {
                    if let Some(path) = dialog.pick_file() {
                        actions.push(InspectorAction::AssignShaderSource {
                            entity,
                            handle: material.handle,
                            shader_path: path,
                        });
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = has_root;
                ui.add_enabled(false, egui::Button::new("Select shader..."));
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                let create_button = ui
                    .add_enabled(has_root, egui::Button::new("Create new shader"))
                    .on_hover_text(
                        "Create a WGSL shader file in content/shaders using the default template.",
                    );
                if create_button.clicked() {
                    let suggested = suggested_shader_stem(material, entity_name);
                    actions.push(InspectorAction::CreateShaderSource {
                        entity,
                        handle: material.handle,
                        suggested_stem: suggested,
                    });
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                ui.add_enabled(false, egui::Button::new("Create new shader"));
            }
        });

        if let Some(metadata) = metadata {
            ui.separator();
            if ui.button("Edit Shader").clicked() {
                actions.push(InspectorAction::EditShader {
                    entity,
                    handle: material.handle,
                    metadata: metadata.clone(),
                });
            }
        } else {
            ui.label("Assign a shader file or create one to customize this material.");
        }
        if !has_root {
            ui.colored_label(
                egui::Color32::from_rgb(200, 80, 80),
                "Open or create a project to manage shader files.",
            );
        }
    });
}

fn shader_path_display(
    metadata: Option<&ShaderMaterialMetadata>,
    content_root: Option<&Path>,
) -> String {
    if let Some(metadata) = metadata {
        if let Some(path) = metadata.source_path() {
            if let Some(root) = content_root {
                if let Ok(relative) = path.strip_prefix(root) {
                    return relative.display().to_string();
                }
            }
            return path.display().to_string();
        }
    }
    "None".to_string()
}

fn suggested_shader_stem(material: &InspectorMaterial, entity_name: &str) -> String {
    if let Some(path) = material.canonical_path.as_ref() {
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            let sanitized = sanitize_shader_stem(stem);
            if !sanitized.is_empty() {
                return sanitized;
            }
        }
    }

    let sanitized_entity = sanitize_shader_stem(entity_name);
    if !sanitized_entity.is_empty() {
        sanitized_entity
    } else {
        format!("material_{}", material.handle.index())
    }
}

fn sanitize_shader_stem(name: &str) -> String {
    let mut stem = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            stem.push(ch.to_ascii_lowercase());
        } else if !stem.ends_with('_') {
            stem.push('_');
        }
    }

    let trimmed = stem.trim_matches('_');
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
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
