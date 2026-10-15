use egui::{
    color_picker::color_edit_button_rgb, color_picker::color_edit_button_rgba, color_picker::Alpha,
    ComboBox, DragValue, Grid,
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

use std::collections::BTreeSet;
use wgpu_cube::asset::{
    Handle, MaterialAsset, MaterialKind, MaterialTextureReference, MaterialTextureSlot,
    ShaderMaterialMetadata,
};
use wgpu_cube::renderer::postprocess::{
    setting_overridden, AntiAliasing, ExposureMode, FocusMode, PostProcessSettings, Tonemapper,
    Upscaler, UpscalingSettings, TRACKBALL_NEUTRAL,
};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{Billboard, BillboardOrientation, ParticleRenderBlendMode};
use wgpu_cube::scene::{
    BoidsBehaviorConfig, CameraComponent, CameraProjection, CanCastShadow, DirectionalLight,
    EnvironmentComponent, MeshComponent, OptimizedBoidsBehaviorConfig, ParticleBehaviorConfig,
    ParticleBehaviorPreset, ParticleColorGradient, ParticleColorKeyframe, ParticleEmissionShape,
    ParticleEmitterComponent, ParticleFloatRange, ParticleSizeCurve, ParticleSizeKeyframe,
    ParticleSystemComponent, ParticleVec3Range, PhysicsBehaviorConfig, PointLight,
    PostProcessVolume, SpotLight, StarfieldBehaviorConfig, Transform, VolumeShape,
};
use wgpu_cube::scripting::{LuaScriptComponent, LuaScriptSource};
use wgpu_cube::{InspectorMaterial, SceneEntityComponentsSummary, SceneEntityInspectorData};

use crate::theme::{self, icons};

#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;

#[derive(Clone)]
pub enum InspectorAction {
    EditScript {
        entity: Entity,
        component: LuaScriptComponent,
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
    UpdatePostProcessVolume {
        entity: Entity,
        component: Box<PostProcessVolume>,
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
    RemoveComponent {
        entity: Entity,
        component: RemovableComponent,
    },
    AddCamera {
        entity: Entity,
    },
    AddMesh {
        entity: Entity,
    },
    AddPointLight {
        entity: Entity,
    },
    AddDirectionalLight {
        entity: Entity,
    },
    AddSpotLight {
        entity: Entity,
    },
    AddEnvironment {
        entity: Entity,
    },
    AddPostProcessVolume {
        entity: Entity,
    },
    AddParticleSystem {
        entity: Entity,
    },
    RenameEntity {
        entity: Entity,
        new_name: String,
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
    let entity = data.entity;

    // Editable name, committed on Enter or when focus leaves the field.
    let mut name = data.name.clone();
    let response = ui.add(
        egui::TextEdit::singleline(&mut name)
            .font(egui::FontId::proportional(16.0))
            .desired_width(f32::INFINITY)
            .hint_text("Entity name"),
    );
    if response.lost_focus() && name != data.name {
        actions.push(InspectorAction::RenameEntity {
            entity,
            new_name: name,
        });
    }
    ui.label(theme::hint(format!("Entity {entity:?}")).small());
    ui.add_space(6.0);

    let components = &data.components;

    if let Some(transform) = components.transform {
        show_transform_section(ui, entity, transform, &mut actions);
    }

    if let Some(camera) = components.camera {
        let header = show_camera_section(ui, entity, camera, &mut actions);
        removable(&header, entity, RemovableComponent::Camera, &mut actions);
    }

    if let Some(mesh) = components.mesh {
        let header = show_mesh_section(ui, mesh);
        removable(&header, entity, RemovableComponent::Mesh, &mut actions);
    }

    if let Some(material) = components.material.clone() {
        show_material_section(ui, entity, &data.name, material, content_root, &mut actions);
    }

    show_light_sections(ui, entity, components, &mut actions);

    if let Some(environment) = components.environment.clone() {
        let header = show_environment_section(ui, entity, environment, &mut actions);
        removable(
            &header,
            entity,
            RemovableComponent::Environment,
            &mut actions,
        );
    }

    if let Some(volume) = components.post_process_volume.clone() {
        let header = show_post_process_volume_section(ui, entity, volume, &mut actions);
        removable(
            &header,
            entity,
            RemovableComponent::PostProcessVolume,
            &mut actions,
        );
    }

    if let Some(component) = components.particle_system.clone() {
        let header = show_particle_system_section(
            ui,
            entity,
            component,
            components.particle_emitter.clone(),
            components.billboard,
            &mut actions,
        );
        removable(
            &header,
            entity,
            RemovableComponent::ParticleSystem,
            &mut actions,
        );
    }

    if let Some(script) = components.script.clone() {
        let (header, action) = show_script_section(ui, entity, &script, content_root);
        removable(&header, entity, RemovableComponent::Script, &mut actions);
        actions.extend(action);
    }

    ui.add_space(10.0);
    ui.vertical_centered_justified(|ui| {
        ui.menu_button(format!("{}  Add Component", icons::ADD), |ui| {
            add_component_menu(ui, entity, components, &mut actions);
        });
    });
    ui.add_space(6.0);

    actions
}

/// Components the inspector can remove from an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovableComponent {
    Camera,
    Mesh,
    PointLight,
    DirectionalLight,
    SpotLight,
    Environment,
    PostProcessVolume,
    ParticleSystem,
    Script,
}

impl RemovableComponent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::Mesh => "Mesh",
            Self::PointLight => "Point Light",
            Self::DirectionalLight => "Directional Light",
            Self::SpotLight => "Spot Light",
            Self::Environment => "Environment",
            Self::PostProcessVolume => "Post Process Volume",
            Self::ParticleSystem => "Particle System",
            Self::Script => "Script",
        }
    }
}

/// Give a section header a context menu that removes its component.
fn removable(
    header: &egui::Response,
    entity: Entity,
    component: RemovableComponent,
    actions: &mut Vec<InspectorAction>,
) {
    header.context_menu(|ui| {
        let text = egui::RichText::new(format!("{}  Remove {}", icons::DELETE, component.label()))
            .color(theme::colors::ERROR);
        if ui.button(text).clicked() {
            actions.push(InspectorAction::RemoveComponent { entity, component });
            ui.close();
        }
    });
}

fn add_component_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    let mut add = |ui: &mut egui::Ui, enabled: bool, label: &str, action: InspectorAction| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            actions.push(action);
            ui.close();
        }
    };

    let has_light = components.point_light.is_some()
        || components.directional_light.is_some()
        || components.spot_light.is_some();

    add(
        ui,
        components.mesh.is_none(),
        &format!("{}  Mesh", icons::MESH),
        InspectorAction::AddMesh { entity },
    );
    add(
        ui,
        components.camera.is_none(),
        &format!("{}  Camera", icons::CAMERA),
        InspectorAction::AddCamera { entity },
    );
    ui.add_enabled_ui(!has_light, |ui| {
        ui.menu_button(format!("{}  Light", icons::LIGHT), |ui| {
            add(
                ui,
                true,
                "Point Light",
                InspectorAction::AddPointLight { entity },
            );
            add(
                ui,
                true,
                "Directional Light",
                InspectorAction::AddDirectionalLight { entity },
            );
            add(
                ui,
                true,
                "Spot Light",
                InspectorAction::AddSpotLight { entity },
            );
        });
    });
    add(
        ui,
        components.environment.is_none(),
        &format!("{}  Environment", icons::ENVIRONMENT),
        InspectorAction::AddEnvironment { entity },
    );
    add(
        ui,
        components.post_process_volume.is_none(),
        &format!("{}  Post Process Volume", icons::POST_PROCESS),
        InspectorAction::AddPostProcessVolume { entity },
    );
    add(
        ui,
        components.particle_system.is_none(),
        &format!("{}  Particle System", icons::PARTICLES),
        InspectorAction::AddParticleSystem { entity },
    );
    add(
        ui,
        components.script.is_none(),
        &format!("{}  Script", icons::SCRIPT),
        InspectorAction::AddScript { entity },
    );
}

fn show_transform_section(
    ui: &mut egui::Ui,
    entity: Entity,
    transform: Transform,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::TRANSFORM, "Transform", |ui| {
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
    })
}

fn show_camera_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: CameraComponent,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ProjectionMode {
        Perspective,
        Orthographic,
    }

    theme::section(ui, icons::CAMERA, "Camera", |ui| {
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
    })
}

fn show_mesh_section(ui: &mut egui::Ui, mesh: MeshComponent) -> egui::Response {
    theme::section(ui, icons::MESH, "Mesh", |ui| {
        ui.label(theme::hint(format!("Mesh asset #{}", mesh.0.index())));
    })
}

fn show_material_section(
    ui: &mut egui::Ui,
    entity: Entity,
    entity_name: &str,
    material_data: InspectorMaterial,
    content_root: Option<&Path>,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::MATERIAL, "Material", |ui| {
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
                changed |= material_emissive_editor(ui, &mut material.emissive_strength);
                changed |= material_float_editor(
                    ui,
                    "Transmission",
                    material.transmission_factor,
                    |value| material.transmission_factor = value,
                );
                changed |= material_ior_editor(ui, &mut material.ior);
                changed |= material_dispersion_editor(ui, &mut material.dispersion);

                let slots = [
                    (
                        "Base color map",
                        MaterialTextureSlot::BaseColor,
                        material.base_color_texture,
                    ),
                    (
                        "Metal/rough map",
                        MaterialTextureSlot::MetallicRoughness,
                        material.metallic_roughness_texture,
                    ),
                    (
                        "Normal map",
                        MaterialTextureSlot::Normal,
                        material.normal_texture,
                    ),
                    (
                        "Emissive map",
                        MaterialTextureSlot::Emissive,
                        material.emissive_texture,
                    ),
                    (
                        "Occlusion map",
                        MaterialTextureSlot::Occlusion,
                        material.occlusion_texture,
                    ),
                ];
                for (label, slot, index) in slots {
                    ui.label(label);
                    let reference = reference_for(slot);
                    let name = reference
                        .and_then(|reference| {
                            reference.display_name().map(str::to_string).or_else(|| {
                                reference.canonical_path().and_then(|path| {
                                    path.file_name()
                                        .map(|name| name.to_string_lossy().into_owned())
                                })
                            })
                        })
                        .unwrap_or_else(|| default_texture_name(index));
                    let response = ui.label(theme::hint(name));
                    if let Some(path) = reference.and_then(|reference| reference.canonical_path()) {
                        response.on_hover_text(path.display().to_string());
                    }
                    ui.end_row();
                }
            });

        if changed {
            actions.push(InspectorAction::UpdateMaterial {
                entity,
                handle: material_data.handle,
                material,
            });
        }
    })
}

fn show_particle_system_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: ParticleSystemComponent,
    emitter: Option<ParticleEmitterComponent>,
    billboard: Option<Billboard>,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::PARTICLES, "Particle System", |ui| {
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
    })
}

fn show_light_sections(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    if let Some(light) = components.point_light {
        let header =
            show_point_light_section(ui, entity, light, components.can_cast_shadow, actions);
        removable(&header, entity, RemovableComponent::PointLight, actions);
    }

    if let Some(light) = components.directional_light {
        let header =
            show_directional_light_section(ui, entity, light, components.can_cast_shadow, actions);
        removable(
            &header,
            entity,
            RemovableComponent::DirectionalLight,
            actions,
        );
    }

    if let Some(light) = components.spot_light {
        let header =
            show_spot_light_section(ui, entity, light, components.can_cast_shadow, actions);
        removable(&header, entity, RemovableComponent::SpotLight, actions);
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
) -> egui::Response {
    theme::section(ui, icons::ENVIRONMENT, "Environment", |ui| {
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
        Grid::new("environment_component_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                labeled_slider(
                    ui,
                    "Ambient intensity",
                    &mut edited.ambient_intensity,
                    0.0..=2.0,
                    0.005,
                    &mut changed,
                );

                ui.label("HDR enabled");
                changed |= ui.checkbox(&mut hdr_enabled, "").changed();
                ui.end_row();

                ui.label("HDR intensity");
                let intensity_response = ui.add_enabled(
                    hdr_settings.path.is_some(),
                    wgpu_cube::ui::FineSlider::new(&mut hdr_settings.intensity, 0.0..=5.0)
                        .step(0.01),
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
            });

        hdr_settings.enabled = hdr_enabled;
        if hdr_settings.path.is_some() || hdr_enabled {
            edited.hdr = Some(hdr_settings);
        } else {
            edited.hdr = None;
        }

        if changed && edited != component {
            actions.push(InspectorAction::UpdateEnvironment {
                entity,
                component: edited,
            });
        }
    })
}

fn show_point_light_section(
    ui: &mut egui::Ui,
    entity: Entity,
    mut light: PointLight,
    casts_shadow: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::LIGHT, "Point Light", |ui| {
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
    })
}

fn show_directional_light_section(
    ui: &mut egui::Ui,
    entity: Entity,
    mut light: DirectionalLight,
    casts_shadow: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::SUN, "Directional Light", |ui| {
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
    })
}

fn show_spot_light_section(
    ui: &mut egui::Ui,
    entity: Entity,
    mut light: SpotLight,
    casts_shadow: Option<CanCastShadow>,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::SPOT, "Spot Light", |ui| {
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
    })
}

fn show_script_section(
    ui: &mut egui::Ui,
    entity: Entity,
    script: &LuaScriptComponent,
    content_root: Option<&Path>,
) -> (egui::Response, Option<InspectorAction>) {
    let mut action = None;
    let header = theme::section(ui, icons::SCRIPT, "Script", |ui| {
        let current_path = match script.source() {
            LuaScriptSource::File { path } => Some(path.clone()),
            LuaScriptSource::Inline { .. } => None,
        };

        ui.horizontal(|ui| {
            let source_label = match script.source() {
                LuaScriptSource::Inline { name, .. } => format!("{name} (inline)"),
                LuaScriptSource::File { path } => display_relative(path, content_root),
            };
            ui.add(egui::Label::new(egui::RichText::new(source_label).monospace()).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(theme::primary_button(format!("{}  Edit", icons::RENAME)))
                    .on_hover_text("Open in the code editor")
                    .clicked()
                {
                    action = Some(InspectorAction::EditScript {
                        entity,
                        component: script.clone(),
                    });
                }
            });
        });

        ui.add_space(2.0);
        let available_scripts = list_available_scripts(content_root);
        let selected_text = current_path
            .as_deref()
            .map(|path| display_relative(path, content_root))
            .unwrap_or_else(|| "Inline script".to_string());
        ui.horizontal(|ui| {
            ui.label("Source");
            ComboBox::from_id_salt(("script_source", entity))
                .selected_text(selected_text)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if available_scripts.is_empty() {
                        ui.label(theme::hint("No .lua files in the project yet."));
                    }
                    for script_path in available_scripts {
                        let is_current = current_path.as_ref() == Some(&script_path);
                        let label = display_relative(&script_path, content_root);
                        if ui.selectable_label(is_current, label).clicked() && !is_current {
                            action = Some(InspectorAction::ChangeScriptSource {
                                entity,
                                script_path,
                            });
                        }
                    }
                });
        });
    });
    (header, action)
}

/// Label for a texture slot that has no named texture assigned.
fn default_texture_name(index: u32) -> String {
    use wgpu_cube::renderer::texture::{
        DEFAULT_CHECKER_TEXTURE_INDEX, DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
        DEFAULT_NORMAL_TEXTURE_INDEX, DEFAULT_WHITE_TEXTURE_INDEX,
    };
    match index {
        DEFAULT_WHITE_TEXTURE_INDEX
        | DEFAULT_NORMAL_TEXTURE_INDEX
        | DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX => "None".to_string(),
        DEFAULT_CHECKER_TEXTURE_INDEX => "Checker (default)".to_string(),
        other => format!("Texture #{other}"),
    }
}

/// `path` relative to the project content folder when it is inside it.
fn display_relative(path: &Path, content_root: Option<&Path>) -> String {
    content_root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(not(target_arch = "wasm32"))]
struct ScriptListCache {
    root: Option<PathBuf>,
    scripts: Vec<PathBuf>,
    last_update: Instant,
}

#[cfg(not(target_arch = "wasm32"))]
static SCRIPT_LIST_CACHE: Mutex<Option<ScriptListCache>> = Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
const CACHE_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Lua files under the project content folder, refreshed every couple of seconds.
#[cfg(not(target_arch = "wasm32"))]
fn list_available_scripts(content_root: Option<&Path>) -> Vec<PathBuf> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 8 {
                    collect(&path, out, depth + 1);
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
                out.push(path);
            }
        }
    }

    let Ok(mut cache) = SCRIPT_LIST_CACHE.lock() else {
        return Vec::new();
    };
    if let Some(cached) = cache.as_ref() {
        if cached.root.as_deref() == content_root
            && cached.last_update.elapsed() < CACHE_REFRESH_INTERVAL
        {
            return cached.scripts.clone();
        }
    }

    let mut scripts = Vec::new();
    if let Some(root) = content_root {
        collect(root, &mut scripts, 0);
    }
    scripts.sort();

    *cache = Some(ScriptListCache {
        root: content_root.map(Path::to_path_buf),
        scripts: scripts.clone(),
        last_update: Instant::now(),
    });
    scripts
}

#[cfg(target_arch = "wasm32")]
fn list_available_scripts(_content_root: Option<&Path>) -> Vec<PathBuf> {
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

/// Dispersion of transmissive materials (20 / Abbe number: 0.36 diamond).
fn material_dispersion_editor(ui: &mut egui::Ui, encoded_value: &mut u8) -> bool {
    let mut value = f32::from(*encoded_value) / 100.0;
    ui.label("Dispersion");
    let changed = ui
        .add(
            DragValue::new(&mut value)
                .speed(0.01)
                .range(0.0..=2.55)
                .fixed_decimals(2),
        )
        .changed();
    ui.end_row();
    if changed {
        *encoded_value = Material::encode_dispersion(value);
    }
    changed
}

/// Index of refraction of transmissive materials (1.5 glass, 2.42 diamond).
fn material_ior_editor(ui: &mut egui::Ui, encoded_value: &mut u8) -> bool {
    let mut value = 1.0 + f32::from(*encoded_value) / 100.0;
    ui.label("Index of refraction");
    let changed = ui
        .add(
            DragValue::new(&mut value)
                .speed(0.01)
                .range(1.0..=3.55)
                .fixed_decimals(2),
        )
        .changed();
    ui.end_row();
    if changed {
        *encoded_value = Material::encode_ior(value);
    }
    changed
}

/// Emissive strength; values above 1 are HDR and make bloom pick the surface up.
fn material_emissive_editor(ui: &mut egui::Ui, encoded_value: &mut u16) -> bool {
    let mut value = f32::from(*encoded_value) / 255.0;
    ui.label("Emissive strength");
    let changed = ui
        .add(
            DragValue::new(&mut value)
                .speed(0.02)
                .range(0.0..=Material::MAX_EMISSIVE)
                .fixed_decimals(3),
        )
        .changed();
    ui.end_row();
    if changed {
        *encoded_value = Material::encode_emissive(value);
    }
    changed
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

/// A labelled fine-step slider row in a two-column grid.
fn labeled_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    changed: &mut bool,
) {
    ui.label(label);
    *changed |= ui
        .add(wgpu_cube::ui::FineSlider::new(value, range).step(step))
        .changed();
    ui.end_row();
}

/// Rows of post-process settings with their override checkboxes. While a volume
/// overrides everything (`overrides` is `None`) the checkboxes are hidden; otherwise
/// each row has one, and editing a value ticks it.
struct PostRows<'a> {
    changed: bool,
    overrides: &'a mut Option<BTreeSet<String>>,
}

impl PostRows<'_> {
    /// The label cell, with the override checkbox when the volume overrides selectively.
    fn label(&mut self, ui: &mut egui::Ui, path: &str, label: &str) {
        let Some(set) = self.overrides.as_mut() else {
            ui.label(label);
            return;
        };
        let mut on = setting_overridden(path, set);
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut on, "")
                .on_hover_text("Override: this volume sets this value; unticked, it comes from the volumes below")
                .changed()
            {
                set_override(set, path, on);
                self.changed = true;
            }
            if on {
                ui.label(label);
            } else {
                ui.label(egui::RichText::new(label).color(theme::colors::TEXT_WEAK));
            }
        });
    }

    /// Record an edit of the setting at `path`, ticking its override.
    fn touched(&mut self, path: &str) {
        self.changed = true;
        if let Some(set) = self.overrides.as_mut() {
            if !setting_overridden(path, set) {
                set.insert(path.to_string());
            }
        }
    }

    fn slider(
        &mut self,
        ui: &mut egui::Ui,
        path: &str,
        label: &str,
        value: &mut f32,
        range: RangeInclusive<f32>,
        step: f32,
    ) {
        self.slider_with(ui, path, label, value, range, step, "");
    }

    #[allow(clippy::too_many_arguments)]
    fn slider_with(
        &mut self,
        ui: &mut egui::Ui,
        path: &str,
        label: &str,
        value: &mut f32,
        range: RangeInclusive<f32>,
        step: f32,
        suffix: &str,
    ) {
        self.label(ui, path, label);
        let mut slider = wgpu_cube::ui::FineSlider::new(value, range).step(step);
        if !suffix.is_empty() {
            slider = slider.suffix(suffix);
        }
        if ui.add(slider).changed() {
            self.touched(path);
        }
        ui.end_row();
    }

    fn color(&mut self, ui: &mut egui::Ui, path: &str, label: &str, color: &mut [f32; 3]) {
        self.label(ui, path, label);
        let mut clamped = color.map(|c| c.clamp(0.0, 1.0));
        if color_edit_button_rgb(ui, &mut clamped).changed() {
            *color = clamped;
            self.touched(path);
        }
        ui.end_row();
    }

    fn checkbox(&mut self, ui: &mut egui::Ui, path: &str, label: &str, value: &mut bool) {
        self.label(ui, path, label);
        if ui.checkbox(value, "").changed() {
            self.touched(path);
        }
        ui.end_row();
    }

    /// A combo box row choosing one of `options`.
    fn choice<T: Copy + PartialEq>(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        path: &str,
        label: &str,
        value: &mut T,
        options: &[(T, &str)],
    ) {
        self.label(ui, path, label);
        let current = options
            .iter()
            .find(|(option, _)| option == value)
            .map_or("", |(_, name)| *name);
        ComboBox::from_id_salt(id)
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (option, name) in options {
                    if ui.selectable_value(value, *option, *name).changed() {
                        self.touched(path);
                    }
                }
            });
        ui.end_row();
    }
}

/// Tick or untick `path`. Unticking a setting covered by a whole-group entry replaces
/// the group with its other settings.
fn set_override(set: &mut BTreeSet<String>, path: &str, on: bool) {
    if on {
        set.insert(path.to_string());
        return;
    }
    set.remove(path);
    let covering: Vec<String> = set
        .iter()
        .filter(|entry| path.starts_with(&format!("{entry}.")))
        .cloned()
        .collect();
    for group in covering {
        set.remove(&group);
        let prefix = format!("{group}.");
        set.extend(
            PostProcessSettings::setting_paths()
                .into_iter()
                .filter(|leaf| leaf.starts_with(&prefix) && leaf != path),
        );
    }
}

/// A collapsible group of settings, optionally led by the effect's "enabled" switch,
/// which greys out the other rows while off.
fn post_group(
    ui: &mut egui::Ui,
    entity: Entity,
    title: &str,
    group: &str,
    enabled: Option<&mut bool>,
    rows: &mut PostRows<'_>,
    add_rows: impl FnOnce(&mut egui::Ui, &mut PostRows<'_>),
) {
    let active = enabled.as_ref().is_none_or(|on| **on);
    let touched = rows.overrides.as_ref().is_none_or(|set| {
        set.iter()
            .any(|entry| entry == group || entry.starts_with(&format!("{group}.")))
    });
    let mut header_text = egui::RichText::new(title);
    header_text = if active && touched {
        header_text.strong()
    } else {
        header_text.color(theme::colors::TEXT_WEAK)
    };
    egui::CollapsingHeader::new(header_text)
        .id_salt(("post_process_group", entity, title))
        .default_open(false)
        .show(ui, |ui| {
            Grid::new(("post_process_grid", entity, title))
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    if let Some(enabled) = enabled {
                        rows.checkbox(ui, &format!("{group}.enabled"), "Enabled", enabled);
                    }
                    if active {
                        add_rows(ui, rows);
                    } else {
                        ui.add_enabled_ui(false, |ui| ui.label("(off)"));
                        ui.end_row();
                    }
                });
        });
}

fn show_post_process_volume_section(
    ui: &mut egui::Ui,
    entity: Entity,
    component: PostProcessVolume,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::POST_PROCESS, "Post Process Volume", |ui| {
        let mut edited = component.clone();
        let mut changed = false;

        Grid::new(("post_process_volume_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Enabled");
                changed |= ui.checkbox(&mut edited.enabled, "").changed();
                ui.end_row();

                ui.label("Extent");
                let current = edited.shape;
                ComboBox::from_id_salt(("post_process_shape", entity))
                    .selected_text(current.label())
                    .show_ui(ui, |ui| {
                        let options = [
                            VolumeShape::Infinite,
                            VolumeShape::Box {
                                size: [10.0, 10.0, 10.0],
                            },
                            VolumeShape::Sphere { radius: 5.0 },
                        ];
                        for option in options {
                            let selected =
                                std::mem::discriminant(&current) == std::mem::discriminant(&option);
                            if ui.selectable_label(selected, option.label()).clicked() && !selected
                            {
                                edited.shape = option;
                                changed = true;
                            }
                        }
                    });
                ui.end_row();

                match &mut edited.shape {
                    VolumeShape::Infinite => {}
                    VolumeShape::Box { size } => {
                        ui.label("Size");
                        ui.horizontal(|ui| {
                            for value in size.iter_mut() {
                                changed |= ui
                                    .add(
                                        DragValue::new(value)
                                            .speed(0.05)
                                            .range(0.0..=100_000.0)
                                            .max_decimals(2),
                                    )
                                    .changed();
                            }
                        });
                        ui.end_row();
                    }
                    VolumeShape::Sphere { radius } => {
                        ui.label("Radius");
                        changed |= ui
                            .add(
                                DragValue::new(radius)
                                    .speed(0.05)
                                    .range(0.0..=100_000.0)
                                    .max_decimals(2),
                            )
                            .changed();
                        ui.end_row();
                    }
                }

                if !edited.shape.is_infinite() {
                    ui.label("Blend distance");
                    changed |= ui
                        .add(
                            DragValue::new(&mut edited.blend_distance)
                                .speed(0.05)
                                .range(0.0..=10_000.0)
                                .max_decimals(2)
                                .suffix(" m"),
                        )
                        .changed();
                    ui.end_row();
                }

                ui.label("Priority");
                changed |= ui
                    .add(DragValue::new(&mut edited.priority).speed(0.1))
                    .changed();
                ui.end_row();

                labeled_slider(
                    ui,
                    "Weight",
                    &mut edited.weight,
                    0.0..=1.0,
                    0.01,
                    &mut changed,
                );
            });
        ui.label(
            theme::hint(if edited.shape.is_infinite() {
                "Applies wherever the camera is. Higher priorities blend over lower ones."
            } else {
                "Applies inside the shape and fades out over the blend distance."
            })
            .small(),
        );
        ui.add_space(4.0);

        // Override mode: everything, or only the ticked settings.
        let mut selective = edited.overrides.is_some();
        if ui
            .checkbox(&mut selective, "Override only ticked settings")
            .on_hover_text(
                "Off: this volume sets every value. On: unticked values come from the \
                 volumes below it, so a local volume can change just a few things.",
            )
            .changed()
        {
            edited.overrides = if selective {
                // Start with nothing ticked except what differs from the defaults.
                Some(differing_settings(&edited.settings))
            } else {
                None
            };
            changed = true;
        }
        ui.add_space(2.0);

        let settings = &mut edited.settings;
        let mut rows = PostRows {
            changed: false,
            overrides: &mut edited.overrides,
        };

        post_group(
            ui,
            entity,
            "Exposure",
            "exposure",
            None,
            &mut rows,
            |ui, rows| {
                let exposure = &mut settings.exposure;
                rows.choice(
                    ui,
                    egui::Id::new(("post_exposure_mode", entity)),
                    "exposure.mode",
                    "Mode",
                    &mut exposure.mode,
                    &[
                        (ExposureMode::Manual, "Manual"),
                        (ExposureMode::Auto, "Auto (eye adaptation)"),
                    ],
                );
                rows.slider_with(
                    ui,
                    "exposure.compensation",
                    "Compensation",
                    &mut exposure.compensation,
                    -5.0..=5.0,
                    0.05,
                    " EV",
                );
                if exposure.mode == ExposureMode::Auto {
                    rows.slider(
                        ui,
                        "exposure.min_ev",
                        "Min EV",
                        &mut exposure.min_ev,
                        -12.0..=12.0,
                        0.1,
                    );
                    rows.slider(
                        ui,
                        "exposure.max_ev",
                        "Max EV",
                        &mut exposure.max_ev,
                        -12.0..=12.0,
                        0.1,
                    );
                    rows.slider(
                        ui,
                        "exposure.speed_up",
                        "Darken speed",
                        &mut exposure.speed_up,
                        0.1..=10.0,
                        0.1,
                    );
                    rows.slider(
                        ui,
                        "exposure.speed_down",
                        "Brighten speed",
                        &mut exposure.speed_down,
                        0.1..=10.0,
                        0.1,
                    );
                    rows.slider(
                        ui,
                        "exposure.low_percent",
                        "Ignore darkest",
                        &mut exposure.low_percent,
                        0.0..=0.95,
                        0.01,
                    );
                    rows.slider(
                        ui,
                        "exposure.high_percent",
                        "Ignore above",
                        &mut exposure.high_percent,
                        0.05..=1.0,
                        0.01,
                    );
                }
            },
        );

        post_group(
            ui,
            entity,
            "Tonemapping",
            "tonemapping",
            None,
            &mut rows,
            |ui, rows| {
                let options: Vec<(Tonemapper, &str)> = Tonemapper::ALL
                    .iter()
                    .map(|mapper| (*mapper, mapper.label()))
                    .collect();
                rows.choice(
                    ui,
                    egui::Id::new(("post_tonemapper", entity)),
                    "tonemapping",
                    "Curve",
                    &mut settings.tonemapping,
                    &options,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Color Grading",
            "color",
            None,
            &mut rows,
            |ui, rows| {
                let color = &mut settings.color;
                rows.slider(
                    ui,
                    "color.post_exposure",
                    "Post exposure",
                    &mut color.post_exposure,
                    -3.0..=3.0,
                    0.05,
                );
                rows.slider(
                    ui,
                    "color.temperature",
                    "Temperature",
                    &mut color.temperature,
                    -100.0..=100.0,
                    0.5,
                );
                rows.slider(
                    ui,
                    "color.tint",
                    "Tint",
                    &mut color.tint,
                    -100.0..=100.0,
                    0.5,
                );
                rows.color(
                    ui,
                    "color.color_filter",
                    "Color filter",
                    &mut color.color_filter,
                );
                rows.slider(
                    ui,
                    "color.hue_shift",
                    "Hue shift",
                    &mut color.hue_shift,
                    -180.0..=180.0,
                    0.5,
                );
                rows.slider(
                    ui,
                    "color.saturation",
                    "Saturation",
                    &mut color.saturation,
                    0.0..=2.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "color.vibrance",
                    "Vibrance",
                    &mut color.vibrance,
                    -1.0..=1.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "color.contrast",
                    "Contrast",
                    &mut color.contrast,
                    0.0..=2.0,
                    0.01,
                );
                for (path, label, control) in [
                    ("color.lift", "Shadows (lift)", &mut color.lift),
                    ("color.gamma", "Midtones (gamma)", &mut color.gamma),
                    ("color.gain", "Highlights (gain)", &mut color.gain),
                ] {
                    rows.label(ui, path, label);
                    ui.horizontal(|ui| {
                        let mut tint =
                            [control[0], control[1], control[2]].map(|c| c.clamp(0.0, 1.0));
                        let mut edited = false;
                        if color_edit_button_rgb(ui, &mut tint).changed() {
                            control[..3].copy_from_slice(&tint);
                            edited = true;
                        }
                        edited |= ui
                            .add(
                                wgpu_cube::ui::FineSlider::new(&mut control[3], -1.0..=1.0)
                                    .step(0.01),
                            )
                            .changed();
                        if ui
                            .small_button(icons::REVERT)
                            .on_hover_text("Reset")
                            .clicked()
                        {
                            *control = TRACKBALL_NEUTRAL;
                            edited = true;
                        }
                        if edited {
                            rows.touched(path);
                        }
                    });
                    ui.end_row();
                }
            },
        );

        post_group(
            ui,
            entity,
            "Color Lookup (LUT)",
            "lut",
            Some(&mut settings.lut.enabled),
            &mut rows,
            |ui, rows| {
                rows.label(ui, "lut.path", "File");
                ui.horizontal(|ui| {
                    let shown = if settings.lut.path.is_empty() {
                        "None".to_string()
                    } else {
                        Path::new(&settings.lut.path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| settings.lut.path.clone())
                    };
                    ui.label(shown).on_hover_text(&settings.lut.path);
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button("Select...").clicked() {
                        if let Some(picked) = FileDialog::new()
                            .add_filter("3D LUT", &["cube"])
                            .pick_file()
                        {
                            match import_lut(&picked) {
                                Ok(path) => {
                                    settings.lut.path = path;
                                    rows.touched("lut.path");
                                }
                                Err(err) => log::warn!("{err}"),
                            }
                        }
                    }
                    if !settings.lut.path.is_empty() && ui.button("Clear").clicked() {
                        settings.lut.path.clear();
                        rows.touched("lut.path");
                    }
                });
                ui.end_row();
                rows.slider(
                    ui,
                    "lut.intensity",
                    "Intensity",
                    &mut settings.lut.intensity,
                    0.0..=1.0,
                    0.01,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Bloom",
            "bloom",
            Some(&mut settings.bloom.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "bloom.intensity",
                    "Intensity",
                    &mut settings.bloom.intensity,
                    0.0..=8.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "bloom.threshold",
                    "Threshold",
                    &mut settings.bloom.threshold,
                    0.0..=5.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "bloom.knee",
                    "Knee",
                    &mut settings.bloom.knee,
                    0.0..=1.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "bloom.scatter",
                    "Scatter",
                    &mut settings.bloom.scatter,
                    0.0..=1.0,
                    0.01,
                );
                rows.color(ui, "bloom.tint", "Tint", &mut settings.bloom.tint);
            },
        );

        post_group(
            ui,
            entity,
            "Ambient Occlusion",
            "ssao",
            Some(&mut settings.ssao.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "ssao.intensity",
                    "Intensity",
                    &mut settings.ssao.intensity,
                    0.0..=1.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "ssao.radius",
                    "Radius",
                    &mut settings.ssao.radius,
                    0.01..=2.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "ssao.bias",
                    "Bias",
                    &mut settings.ssao.bias,
                    0.0..=0.2,
                    0.001,
                );
                rows.slider(
                    ui,
                    "ssao.power",
                    "Power",
                    &mut settings.ssao.power,
                    0.5..=4.0,
                    0.01,
                );
                rows.checkbox(
                    ui,
                    "ssao.ray_traced",
                    "Ray traced (hardware)",
                    &mut settings.ssao.ray_traced,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Depth of Field",
            "depth_of_field",
            Some(&mut settings.depth_of_field.enabled),
            &mut rows,
            |ui, rows| {
                rows.choice(
                    ui,
                    egui::Id::new(("post_dof_mode", entity)),
                    "depth_of_field.mode",
                    "Mode",
                    &mut settings.depth_of_field.mode,
                    &[
                        (FocusMode::Physical, "Physical lens"),
                        (FocusMode::Manual, "Manual ranges"),
                    ],
                );
                match settings.depth_of_field.mode {
                    FocusMode::Physical => {
                        rows.checkbox(
                            ui,
                            "depth_of_field.autofocus",
                            "Autofocus (centre)",
                            &mut settings.depth_of_field.autofocus,
                        );
                        if !settings.depth_of_field.autofocus {
                            rows.label(ui, "depth_of_field.focus_distance", "Focus distance");
                            if ui
                                .add(
                                    DragValue::new(&mut settings.depth_of_field.focus_distance)
                                        .speed(0.05)
                                        .range(0.01..=100_000.0)
                                        .max_decimals(2)
                                        .suffix(" m"),
                                )
                                .changed()
                            {
                                rows.touched("depth_of_field.focus_distance");
                            }
                            ui.end_row();
                        }
                        rows.label(ui, "depth_of_field.aperture", "Aperture");
                        if ui
                            .add(
                                wgpu_cube::ui::FineSlider::new(
                                    &mut settings.depth_of_field.aperture,
                                    0.7..=32.0,
                                )
                                .logarithmic(true)
                                .suffix(" f/"),
                            )
                            .changed()
                        {
                            rows.touched("depth_of_field.aperture");
                        }
                        ui.end_row();
                    }
                    FocusMode::Manual => {
                        for (path, label, value) in [
                            (
                                "depth_of_field.near_start",
                                "Near blur max",
                                &mut settings.depth_of_field.near_start,
                            ),
                            (
                                "depth_of_field.near_end",
                                "Near sharp from",
                                &mut settings.depth_of_field.near_end,
                            ),
                            (
                                "depth_of_field.far_start",
                                "Far sharp until",
                                &mut settings.depth_of_field.far_start,
                            ),
                            (
                                "depth_of_field.far_end",
                                "Far blur max",
                                &mut settings.depth_of_field.far_end,
                            ),
                        ] {
                            rows.label(ui, path, label);
                            if ui
                                .add(
                                    DragValue::new(value)
                                        .speed(0.05)
                                        .range(0.0..=100_000.0)
                                        .max_decimals(2)
                                        .suffix(" m"),
                                )
                                .changed()
                            {
                                rows.touched(path);
                            }
                            ui.end_row();
                        }
                    }
                }
                rows.slider_with(
                    ui,
                    "depth_of_field.max_blur",
                    "Max blur",
                    &mut settings.depth_of_field.max_blur,
                    0.0..=48.0,
                    0.1,
                    " px",
                );
            },
        );

        post_group(
            ui,
            entity,
            "Refraction",
            "refraction",
            None,
            &mut rows,
            |ui, rows| {
                rows.checkbox(
                    ui,
                    "refraction.ray_traced",
                    "Ray traced (hardware)",
                    &mut settings.refraction.ray_traced,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Ray-traced shadows",
            "rt_shadows",
            Some(&mut settings.rt_shadows.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider_with(
                    ui,
                    "rt_shadows.softness",
                    "Sun size",
                    &mut settings.rt_shadows.softness,
                    0.0..=5.0,
                    0.05,
                    " deg",
                );
                rows.slider_with(
                    ui,
                    "rt_shadows.light_radius",
                    "Lamp radius",
                    &mut settings.rt_shadows.light_radius,
                    0.0..=1.0,
                    0.01,
                    " m",
                );
            },
        );

        post_group(
            ui,
            entity,
            "Reflections",
            "ssr",
            Some(&mut settings.ssr.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "ssr.intensity",
                    "Intensity",
                    &mut settings.ssr.intensity,
                    0.0..=4.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "ssr.blur",
                    "Roughness blur",
                    &mut settings.ssr.blur,
                    0.0..=1.0,
                    0.01,
                );
                rows.slider_with(
                    ui,
                    "ssr.max_distance",
                    "Max distance",
                    &mut settings.ssr.max_distance,
                    0.5..=100.0,
                    0.1,
                    " m",
                );
                rows.checkbox(
                    ui,
                    "ssr.ray_traced",
                    "Ray traced (hardware)",
                    &mut settings.ssr.ray_traced,
                );
                rows.checkbox(
                    ui,
                    "ssr.full_resolution",
                    "Full resolution",
                    &mut settings.ssr.full_resolution,
                );
                rows.slider_with(
                    ui,
                    "ssr.thickness",
                    "Thickness",
                    &mut settings.ssr.thickness,
                    0.01..=5.0,
                    0.01,
                    " m",
                );
            },
        );

        post_group(
            ui,
            entity,
            "Motion Blur",
            "motion_blur",
            Some(&mut settings.motion_blur.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "motion_blur.intensity",
                    "Shutter",
                    &mut settings.motion_blur.intensity,
                    0.0..=1.0,
                    0.01,
                );
                rows.slider_with(
                    ui,
                    "motion_blur.max_blur",
                    "Max streak",
                    &mut settings.motion_blur.max_blur,
                    0.0..=128.0,
                    0.5,
                    " px",
                );
            },
        );

        post_group(
            ui,
            entity,
            "Vignette",
            "vignette",
            Some(&mut settings.vignette.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "vignette.intensity",
                    "Intensity",
                    &mut settings.vignette.intensity,
                    0.0..=1.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "vignette.smoothness",
                    "Smoothness",
                    &mut settings.vignette.smoothness,
                    0.01..=1.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "vignette.roundness",
                    "Roundness",
                    &mut settings.vignette.roundness,
                    0.0..=1.0,
                    0.01,
                );
                rows.checkbox(
                    ui,
                    "vignette.rounded",
                    "Circular",
                    &mut settings.vignette.rounded,
                );
                rows.color(ui, "vignette.color", "Color", &mut settings.vignette.color);
                rows.label(ui, "vignette.center", "Center");
                ui.horizontal(|ui| {
                    for value in settings.vignette.center.iter_mut() {
                        if ui
                            .add(
                                DragValue::new(value)
                                    .speed(0.005)
                                    .range(0.0..=1.0)
                                    .max_decimals(3),
                            )
                            .changed()
                        {
                            rows.touched("vignette.center");
                        }
                    }
                });
                ui.end_row();
            },
        );

        post_group(
            ui,
            entity,
            "Chromatic Aberration",
            "chromatic_aberration",
            Some(&mut settings.chromatic_aberration.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "chromatic_aberration.intensity",
                    "Intensity",
                    &mut settings.chromatic_aberration.intensity,
                    0.0..=1.0,
                    0.01,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Film Grain",
            "film_grain",
            Some(&mut settings.film_grain.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "film_grain.intensity",
                    "Intensity",
                    &mut settings.film_grain.intensity,
                    0.0..=1.0,
                    0.01,
                );
                rows.slider(
                    ui,
                    "film_grain.response",
                    "Response",
                    &mut settings.film_grain.response,
                    0.0..=1.0,
                    0.01,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Sharpen",
            "sharpen",
            Some(&mut settings.sharpen.enabled),
            &mut rows,
            |ui, rows| {
                rows.slider(
                    ui,
                    "sharpen.strength",
                    "Strength",
                    &mut settings.sharpen.strength,
                    0.0..=1.0,
                    0.01,
                );
            },
        );

        post_group(
            ui,
            entity,
            "Anti-aliasing",
            "anti_aliasing",
            None,
            &mut rows,
            |ui, rows| {
                rows.choice(
                    ui,
                    egui::Id::new(("post_anti_aliasing", entity)),
                    "anti_aliasing",
                    "Method",
                    &mut settings.anti_aliasing,
                    &[
                        (AntiAliasing::None, "None"),
                        (AntiAliasing::Fxaa, "FXAA"),
                        (AntiAliasing::Taa, "TAA (temporal)"),
                    ],
                );
            },
        );

        post_group(
            ui,
            entity,
            "Upscaling",
            "upscaling",
            None,
            &mut rows,
            |ui, rows| {
                rows.slider_with(
                    ui,
                    "upscaling.render_scale",
                    "Render scale",
                    &mut settings.upscaling.render_scale,
                    UpscalingSettings::MIN_RENDER_SCALE..=1.0,
                    0.01,
                    "x",
                );
                rows.choice(
                    ui,
                    egui::Id::new(("post_upscaler", entity)),
                    "upscaling.method",
                    "Upscaler",
                    &mut settings.upscaling.method,
                    &[(Upscaler::Fsr, "FSR 3.1"), (Upscaler::Taau, "TAAU")],
                );
                rows.slider(
                    ui,
                    "upscaling.sharpness",
                    "FSR sharpness",
                    &mut settings.upscaling.sharpness,
                    0.0..=1.0,
                    0.01,
                );
            },
        );

        changed |= rows.changed;

        ui.add_space(4.0);
        if ui
            .button(format!("{}  Reset effects to defaults", icons::REVERT))
            .clicked()
        {
            edited.settings = PostProcessSettings::default();
            if edited.overrides.is_some() {
                edited.overrides = Some(BTreeSet::new());
            }
            changed = true;
        }

        if changed && edited != component {
            actions.push(InspectorAction::UpdatePostProcessVolume {
                entity,
                component: Box::new(edited),
            });
        }
    })
}

/// The path to store for a picked `.cube` file: project-relative, copying files from
/// outside the project into `content/luts` so the project stays self-contained.
#[cfg(not(target_arch = "wasm32"))]
fn import_lut(picked: &Path) -> Result<String, String> {
    let Some(root) = wgpu_cube::project::active_project_root() else {
        return Ok(picked.to_string_lossy().into_owned());
    };
    let relative = |path: &Path| {
        path.strip_prefix(&root)
            .ok()
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
    };
    if let Some(rel) = relative(picked) {
        return Ok(rel);
    }
    let file_name = picked
        .file_name()
        .ok_or_else(|| format!("{} has no file name", picked.display()))?;
    let dir = root.join("content").join("luts");
    fs::create_dir_all(&dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
    let target = dir.join(file_name);
    fs::copy(picked, &target).map_err(|err| format!("copying LUT: {err}"))?;
    relative(&target).ok_or_else(|| "LUT copy is outside the project".to_string())
}

/// Settings whose values differ from the defaults, as override paths.
fn differing_settings(settings: &PostProcessSettings) -> BTreeSet<String> {
    let (Ok(current), Ok(defaults)) = (
        serde_json::to_value(settings),
        serde_json::to_value(PostProcessSettings::default()),
    ) else {
        return BTreeSet::new();
    };
    PostProcessSettings::setting_paths()
        .into_iter()
        .filter(|path| {
            let pointer = format!("/{}", path.replace('.', "/"));
            current.pointer(&pointer) != defaults.pointer(&pointer)
        })
        .collect()
}
