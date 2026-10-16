//! Inspector sections for the physics components: rigid body, collider, character
//! controller and spring camera.

use egui::{DragValue, Grid};
use hecs::Entity;
use wgpu_cube::physics::{
    BodyKind, CharacterController, Collider, ColliderShape, RigidBody, SpringCamera,
};
use wgpu_cube::SceneEntityComponentsSummary;

use super::{float_drag_value, labeled_slider, InspectorAction};
use crate::theme::{self, icons};

fn set(
    actions: &mut Vec<InspectorAction>,
    entity: Entity,
    component: &'static str,
    value: impl serde::Serialize,
) {
    match serde_json::to_value(value) {
        Ok(value) => actions.push(InspectorAction::SetPhysics {
            entity,
            component,
            value,
        }),
        Err(err) => log::warn!("Cannot edit {component}: {err}"),
    }
}

/// A section header's context menu entry removing its component.
fn removable(
    header: &egui::Response,
    entity: Entity,
    component: &'static str,
    label: &str,
    actions: &mut Vec<InspectorAction>,
) {
    header.context_menu(|ui| {
        let text = egui::RichText::new(format!("{}  Remove {label}", icons::DELETE))
            .color(theme::colors::ERROR);
        if ui.button(text).clicked() {
            actions.push(InspectorAction::SetPhysics {
                entity,
                component,
                value: serde_json::Value::Null,
            });
            ui.close();
        }
    });
}

fn checkbox_row(ui: &mut egui::Ui, label: &str, value: &mut bool, hint: &str) -> bool {
    ui.label(label);
    let changed = ui.checkbox(value, "").on_hover_text(hint).changed();
    ui.end_row();
    changed
}

fn vec3_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut [f32; 3],
    speed: f32,
    suffix: &str,
) -> bool {
    ui.label(label);
    let mut changed = false;
    ui.horizontal(|ui| {
        for component in value.iter_mut() {
            changed |= ui
                .add(DragValue::new(component).speed(speed).suffix(suffix))
                .changed();
        }
    });
    ui.end_row();
    changed
}

pub(super) fn show_physics_sections(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    if let Some(body) = &components.rigid_body {
        let header = rigid_body_section(ui, entity, body, components.collider.is_none(), actions);
        removable(&header, entity, "rigid_body", "Rigid Body", actions);
    }
    if let Some(collider) = &components.collider {
        let header = collider_section(ui, entity, collider, actions);
        removable(&header, entity, "collider", "Collider", actions);
    }
    if let Some(controller) = &components.character_controller {
        let header = character_section(ui, entity, controller, actions);
        removable(
            &header,
            entity,
            "character_controller",
            "Character Controller",
            actions,
        );
    }
    if let Some(camera) = &components.spring_camera {
        let header =
            spring_camera_section(ui, entity, camera, components.camera.is_some(), actions);
        removable(&header, entity, "spring_camera", "Spring Camera", actions);
    }
}

pub(super) fn add_physics_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    ui.menu_button(format!("{}  Physics", icons::RIGID_BODY), |ui| {
        let mut add = |ui: &mut egui::Ui, enabled: bool, label: &str, component, value| {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                set(actions, entity, component, value);
                ui.close();
            }
        };
        add(
            ui,
            components.rigid_body.is_none(),
            "Rigid Body",
            "rigid_body",
            serde_json::to_value(RigidBody::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.collider.is_none(),
            "Collider",
            "collider",
            serde_json::to_value(Collider::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.character_controller.is_none(),
            "Character Controller",
            "character_controller",
            serde_json::to_value(CharacterController::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.spring_camera.is_none(),
            "Spring Camera",
            "spring_camera",
            serde_json::to_value(SpringCamera::default()).unwrap_or_default(),
        );
    });
}

fn rigid_body_section(
    ui: &mut egui::Ui,
    entity: Entity,
    body: &RigidBody,
    no_collider: bool,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::RIGID_BODY, "Rigid Body", |ui| {
        let mut edited = body.clone();
        let mut changed = false;
        Grid::new("rigid_body_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Kind");
                egui::ComboBox::from_id_salt("rigid_body_kind")
                    .selected_text(edited.kind.label())
                    .show_ui(ui, |ui| {
                        for kind in BodyKind::ALL {
                            changed |= ui
                                .selectable_value(&mut edited.kind, kind, kind.label())
                                .changed();
                        }
                    });
                ui.end_row();
                if edited.kind == BodyKind::Dynamic {
                    changed |=
                        float_drag_value(ui, "Mass (kg)", &mut edited.mass, 0.1, Some(0.0..=1.0e6));
                    labeled_slider(
                        ui,
                        "Gravity scale",
                        &mut edited.gravity_scale,
                        -2.0..=4.0,
                        0.05,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Linear damping",
                        &mut edited.linear_damping,
                        0.0..=10.0,
                        0.05,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Angular damping",
                        &mut edited.angular_damping,
                        0.0..=10.0,
                        0.05,
                        &mut changed,
                    );
                    changed |= checkbox_row(
                        ui,
                        "Lock rotation",
                        &mut edited.lock_rotation,
                        "Collisions never turn the body",
                    );
                    changed |= checkbox_row(
                        ui,
                        "Continuous (CCD)",
                        &mut edited.ccd,
                        "For small fast bodies that must not pass through walls",
                    );
                    changed |= checkbox_row(
                        ui,
                        "Can sleep",
                        &mut edited.can_sleep,
                        "Rest when still; wakes when touched",
                    );
                    changed |= vec3_row(
                        ui,
                        "Initial velocity",
                        &mut edited.initial_velocity,
                        0.05,
                        "",
                    );
                }
            });
        if no_collider {
            ui.label(theme::hint("No collider: it collides as its mesh (Auto)."));
        }
        if changed && edited != *body {
            set(actions, entity, "rigid_body", &edited);
        }
    })
}

fn collider_section(
    ui: &mut egui::Ui,
    entity: Entity,
    collider: &Collider,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::COLLIDER, "Collider", |ui| {
        let mut edited = collider.clone();
        let mut changed = false;
        Grid::new("collider_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Shape");
                let current = edited.shape.kind_name();
                egui::ComboBox::from_id_salt("collider_shape")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for name in ColliderShape::KIND_NAMES {
                            if ui.selectable_label(name == current, name).clicked()
                                && name != current
                            {
                                if let Some(shape) = ColliderShape::from_kind_name(name) {
                                    edited.shape = shape;
                                    changed = true;
                                }
                            }
                        }
                    });
                ui.end_row();
                match &mut edited.shape {
                    ColliderShape::Box { size } => {
                        changed |= vec3_row(ui, "Size", size, 0.02, " m");
                    }
                    ColliderShape::Sphere { radius } => {
                        changed |=
                            float_drag_value(ui, "Radius (m)", radius, 0.01, Some(0.001..=1000.0));
                    }
                    ColliderShape::Capsule { radius, height }
                    | ColliderShape::Cylinder { radius, height } => {
                        changed |=
                            float_drag_value(ui, "Radius (m)", radius, 0.01, Some(0.001..=1000.0));
                        changed |=
                            float_drag_value(ui, "Height (m)", height, 0.01, Some(0.001..=1000.0));
                    }
                    ColliderShape::Auto | ColliderShape::Mesh | ColliderShape::ConvexHull => {}
                }
                changed |= vec3_row(ui, "Offset", &mut edited.offset, 0.01, "");
                labeled_slider(
                    ui,
                    "Friction",
                    &mut edited.friction,
                    0.0..=2.0,
                    0.01,
                    &mut changed,
                );
                labeled_slider(
                    ui,
                    "Restitution",
                    &mut edited.restitution,
                    0.0..=1.0,
                    0.01,
                    &mut changed,
                );
                changed |=
                    float_drag_value(ui, "Density", &mut edited.density, 0.01, Some(0.0..=1.0e5));
                changed |= checkbox_row(
                    ui,
                    "Sensor",
                    &mut edited.sensor,
                    "Detects overlaps (on_trigger_enter / on_trigger_exit) without blocking",
                );
                changed |= checkbox_row(
                    ui,
                    "Include children",
                    &mut edited.include_children,
                    "Also the meshes of descendants (imported models and levels)",
                );
            });
        ui.label(theme::hint(match edited.shape {
            ColliderShape::Auto => "Fits the mesh: exact for primitives, triangles for scenery, the hull for dynamic bodies.",
            ColliderShape::Mesh => "The mesh's triangles (dynamic bodies get its hull).",
            _ => "Sizes are stretched by the entity's scale.",
        }));
        if changed && edited != *collider {
            set(actions, entity, "collider", &edited);
        }
    })
}

fn character_section(
    ui: &mut egui::Ui,
    entity: Entity,
    controller: &CharacterController,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::CHARACTER, "Character Controller", |ui| {
        let mut edited = controller.clone();
        let mut changed = false;
        Grid::new("character_controller_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |=
                    float_drag_value(ui, "Radius (m)", &mut edited.radius, 0.01, Some(0.05..=5.0));
                changed |=
                    float_drag_value(ui, "Height (m)", &mut edited.height, 0.01, Some(0.1..=10.0));
                changed |= float_drag_value(
                    ui,
                    "Walk speed (m/s)",
                    &mut edited.walk_speed,
                    0.05,
                    Some(0.0..=50.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Run speed (m/s)",
                    &mut edited.run_speed,
                    0.05,
                    Some(0.0..=50.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Acceleration",
                    &mut edited.acceleration,
                    0.5,
                    Some(0.0..=500.0),
                );
                labeled_slider(
                    ui,
                    "Air control",
                    &mut edited.air_control,
                    0.0..=1.0,
                    0.01,
                    &mut changed,
                );
                changed |= float_drag_value(
                    ui,
                    "Gravity (m/s²)",
                    &mut edited.gravity,
                    0.1,
                    Some(0.0..=200.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Jump height (m)",
                    &mut edited.jump_height,
                    0.01,
                    Some(0.0..=20.0),
                );
                ui.label("Air jumps");
                changed |= ui
                    .add(DragValue::new(&mut edited.air_jumps).range(0..=10))
                    .changed();
                ui.end_row();
                changed |= float_drag_value(
                    ui,
                    "Coyote time (s)",
                    &mut edited.coyote_time,
                    0.005,
                    Some(0.0..=1.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Jump buffer (s)",
                    &mut edited.jump_buffer,
                    0.005,
                    Some(0.0..=1.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Max slope (°)",
                    &mut edited.max_slope,
                    0.5,
                    Some(0.0..=89.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Step height (m)",
                    &mut edited.step_height,
                    0.01,
                    Some(0.0..=2.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Snap distance (m)",
                    &mut edited.snap_distance,
                    0.01,
                    Some(0.0..=2.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Max fall speed",
                    &mut edited.max_fall_speed,
                    0.5,
                    Some(0.0..=500.0),
                );
                changed |= checkbox_row(
                    ui,
                    "Face movement",
                    &mut edited.face_movement,
                    "Turn toward the direction it moves",
                );
                changed |= float_drag_value(
                    ui,
                    "Turn speed (°/s)",
                    &mut edited.turn_speed,
                    5.0,
                    Some(0.0..=5000.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Yaw offset (°)",
                    &mut edited.yaw_offset,
                    1.0,
                    Some(-180.0..=180.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Push mass (kg)",
                    &mut edited.push_mass,
                    1.0,
                    Some(0.0..=10000.0),
                );
                changed |= checkbox_row(
                    ui,
                    "Ride platforms",
                    &mut edited.ride_platforms,
                    "Move with the kinematic platform it stands on",
                );
            });
        ui.label(theme::hint(
            "The entity's origin is the feet. Steer it from Lua: character_move, character_jump.",
        ));
        if changed && edited != *controller {
            set(actions, entity, "character_controller", &edited);
        }
    })
}

fn spring_camera_section(
    ui: &mut egui::Ui,
    entity: Entity,
    camera: &SpringCamera,
    has_camera: bool,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::SPRING_CAMERA, "Spring Camera", |ui| {
        let mut edited = camera.clone();
        let mut changed = false;
        Grid::new("spring_camera_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Target");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut edited.target).hint_text("parent"))
                    .on_hover_text("Name of the entity to follow; empty follows the parent")
                    .changed();
                ui.end_row();
                changed |= vec3_row(ui, "Pivot offset", &mut edited.pivot_offset, 0.01, "");
                changed |= float_drag_value(
                    ui,
                    "Distance (m)",
                    &mut edited.distance,
                    0.05,
                    Some(0.0..=100.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Min distance",
                    &mut edited.min_distance,
                    0.05,
                    Some(0.0..=100.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Max distance",
                    &mut edited.max_distance,
                    0.05,
                    Some(0.0..=200.0),
                );
                changed |=
                    float_drag_value(ui, "Yaw (°)", &mut edited.yaw, 1.0, Some(-180.0..=180.0));
                changed |=
                    float_drag_value(ui, "Pitch (°)", &mut edited.pitch, 0.5, Some(-89.0..=89.0));
                changed |= float_drag_value(
                    ui,
                    "Min pitch (°)",
                    &mut edited.min_pitch,
                    0.5,
                    Some(-89.0..=89.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Max pitch (°)",
                    &mut edited.max_pitch,
                    0.5,
                    Some(-89.0..=89.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Shoulder offset",
                    &mut edited.shoulder_offset,
                    0.01,
                    Some(-5.0..=5.0),
                );
                changed |= checkbox_row(
                    ui,
                    "Mouse look",
                    &mut edited.mouse_look,
                    "Turn while the cursor is locked or the right button is held",
                );
                changed |= float_drag_value(
                    ui,
                    "Sensitivity (°/px)",
                    &mut edited.sensitivity,
                    0.005,
                    Some(0.0..=5.0),
                );
                changed |=
                    checkbox_row(ui, "Invert Y", &mut edited.invert_y, "Mouse up looks down");
                changed |= float_drag_value(
                    ui,
                    "Zoom step (m)",
                    &mut edited.zoom_step,
                    0.05,
                    Some(0.0..=10.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Follow lag (s)",
                    &mut edited.follow_lag,
                    0.005,
                    Some(0.0..=2.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Vertical lag (s)",
                    &mut edited.vertical_lag,
                    0.005,
                    Some(0.0..=2.0),
                );
                changed |= checkbox_row(
                    ui,
                    "Collision",
                    &mut edited.collision,
                    "Pull in when scenery comes between camera and target",
                );
                changed |= float_drag_value(
                    ui,
                    "Probe radius (m)",
                    &mut edited.probe_radius,
                    0.01,
                    Some(0.01..=2.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Auto follow",
                    &mut edited.auto_follow,
                    0.05,
                    Some(0.0..=20.0),
                );
            });
        if !has_camera {
            ui.label(theme::hint(
                "Add a Camera component too, and make it the active camera.",
            ));
        }
        if changed && edited != *camera {
            set(actions, entity, "spring_camera", &edited);
        }
    })
}
