// Inspector module - handles entity component inspection and editing UI
//
// This module has been refactored from a monolithic 2,372-line file into
// a modular structure for better maintainability.
//
// Structure:
// - actions.rs: InspectorAction enum (all possible actions from inspector UI)
// - helpers.rs: Common UI helper functions
// - sections/: Component-specific UI modules
//   - transform.rs: Transform component editor
//   - camera.rs: Camera component editor
//   - mesh.rs: Mesh component display
//   - (More sections to be extracted...)
//
// Migration status: Partial - core sections extracted, remaining sections to follow

pub mod actions;
pub mod helpers;
pub mod sections;

// Re-export main types
pub use actions::InspectorAction;

// Temporarily keep the show_entity_inspector and remaining sections here
// until full migration is complete
use std::path::Path;
use wgpu_cube::{SceneEntityInspectorData};

use helpers::begin_section;
use sections::{show_camera_section, show_mesh_section, show_transform_section};

/// Main entry point for the entity inspector UI.
/// Displays all components of the selected entity and returns actions to be processed.
pub fn show_entity_inspector(
    ui: &mut egui::Ui,
    data: &SceneEntityInspectorData,
    content_root: Option<&Path>,
) -> Vec<InspectorAction> {
    let mut actions = Vec::new();

    // Editable name field
    ui.horizontal(|ui| {
        ui.label("Name:");
        let mut name = data.name.clone();
        let response = ui.text_edit_singleline(&mut name);
        if response.lost_focus() && name != data.name {
            actions.push(InspectorAction::RenameEntity {
                entity: data.entity,
                new_name: name,
            });
        }
    });

    ui.label(format!("Entity: {:?}", data.entity));
    ui.add_space(8.0);

    let mut first_section = true;

    // Use extracted section modules
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

    // TODO: Extract remaining sections to separate modules:
    // - Materials (show_material_section)
    // - Lights (show_light_sections)
    // - Environment (show_environment_section)
    // - Particles (show_particle_system_section)
    // - Scripts (show_script_section)
    //
    // For now, these remain in the original inspector.rs and will be
    // migrated in follow-up commits.

    // Add component context menu
    begin_section(ui, &mut first_section);
    ui.add_space(4.0);
    let response = ui.allocate_response(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y * 3.0),
        egui::Sense::click(),
    );

    response.context_menu(|ui| {
        ui.label("Add Component");
        ui.separator();

        if data.components.camera.is_none() {
            if ui.button("Camera").clicked() {
                actions.push(InspectorAction::AddCamera { entity: data.entity });
                ui.close_menu();
            }
        }

        if data.components.mesh.is_none() {
            if ui.button("Mesh").clicked() {
                actions.push(InspectorAction::AddMesh { entity: data.entity });
                ui.close_menu();
            }
        }

        let has_light = data.components.point_light.is_some()
            || data.components.directional_light.is_some()
            || data.components.spot_light.is_some();
        if !has_light {
            ui.menu_button("Light", |ui| {
                if ui.button("Point Light").clicked() {
                    actions.push(InspectorAction::AddPointLight { entity: data.entity });
                    ui.close_menu();
                }
                if ui.button("Directional Light").clicked() {
                    actions.push(InspectorAction::AddDirectionalLight { entity: data.entity });
                    ui.close_menu();
                }
                if ui.button("Spot Light").clicked() {
                    actions.push(InspectorAction::AddSpotLight { entity: data.entity });
                    ui.close_menu();
                }
            });
        }

        if data.components.environment.is_none() {
            if ui.button("Environment").clicked() {
                actions.push(InspectorAction::AddEnvironment { entity: data.entity });
                ui.close_menu();
            }
        }

        if data.components.particle_system.is_none() {
            if ui.button("Particle System").clicked() {
                actions.push(InspectorAction::AddParticleSystem { entity: data.entity });
                ui.close_menu();
            }
        }

        if data.components.script.is_none() {
            if ui.button("Script").clicked() {
                actions.push(InspectorAction::AddScript { entity: data.entity });
                ui.close_menu();
            }
        }
    });

    actions
}
