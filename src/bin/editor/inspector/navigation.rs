//! Inspector sections for navigation: the navmesh surface (settings, bake, debug view,
//! path test), agents, obstacles and modifiers.

use egui::{ComboBox, DragValue, Grid};
use glam::Vec3;
use hecs::Entity;
use wgpu_cube::navigation::{
    AgentStatus, BakeProgress, NavAgent, NavAgentDrive, NavCollect, NavDebugDraw,
    NavGeometrySource, NavLink, NavMeshSurface, NavModifier, NavObstacle, NavObstacleShape,
    SurfaceStatus,
};
use wgpu_cube::SceneEntityComponentsSummary;

use super::{float_drag_value, labeled_slider, InspectorAction};
use crate::theme::{self, icons};

/// What the navigation sections ask of the scene (NavigationSystem carries them out).
#[derive(Clone, Debug, PartialEq)]
pub enum NavigationRequest {
    /// Bake this surface's navmesh (in the background) into content/navigation.
    Bake {
        entity: Entity,
    },
    SetDebug(NavDebugDraw),
    /// Clicks in the Scene view set a test path's ends (on), or select again (off).
    PathTool(bool),
    ClearPath,
}

/// A path test's ends and result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathTestStatus {
    pub from: Option<Vec3>,
    pub to: Option<Vec3>,
    pub complete: bool,
    pub length: f32,
    pub corners: usize,
}

/// The navigation state the inspector shows, refreshed every frame.
#[derive(Clone, Debug, Default)]
pub struct NavigationPanelStatus {
    pub surfaces: Vec<SurfaceStatus>,
    pub bake: Option<BakeProgress>,
    pub debug: NavDebugDraw,
    pub path_tool: bool,
    pub path: PathTestStatus,
    pub playing: bool,
    pub project_open: bool,
    /// The selected entity's agent, while playing.
    pub selected_agent: Option<AgentStatus>,
}

impl NavigationPanelStatus {
    fn surface(&self, entity: Entity) -> Option<&SurfaceStatus> {
        self.surfaces.iter().find(|s| s.entity == entity)
    }
}

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

fn vec3_row(ui: &mut egui::Ui, label: &str, value: &mut [f32; 3], speed: f32) -> bool {
    ui.label(label);
    let mut changed = false;
    ui.horizontal(|ui| {
        for component in value.iter_mut() {
            changed |= ui.add(DragValue::new(component).speed(speed)).changed();
        }
    });
    ui.end_row();
    changed
}

pub(super) fn show_navigation_sections(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    status: &NavigationPanelStatus,
    actions: &mut Vec<InspectorAction>,
) {
    if let Some(surface) = &components.nav_mesh_surface {
        let header = surface_section(ui, entity, surface, status, actions);
        removable(
            &header,
            entity,
            "nav_mesh_surface",
            "NavMesh Surface",
            actions,
        );
    }
    if let Some(agent) = &components.nav_agent {
        let header = agent_section(
            ui,
            entity,
            agent,
            components.character_controller.is_some(),
            status,
            actions,
        );
        removable(&header, entity, "nav_agent", "Nav Agent", actions);
    }
    if let Some(obstacle) = &components.nav_obstacle {
        let header = obstacle_section(ui, entity, obstacle, actions);
        removable(&header, entity, "nav_obstacle", "Nav Obstacle", actions);
    }
    if let Some(modifier) = &components.nav_modifier {
        let header = modifier_section(ui, entity, modifier, actions);
        removable(&header, entity, "nav_modifier", "Nav Modifier", actions);
    }
    if let Some(link) = &components.nav_link {
        let header = link_section(ui, entity, link, actions);
        removable(&header, entity, "nav_link", "Nav Link", actions);
    }
}

pub(super) fn add_navigation_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    ui.menu_button(format!("{}  Navigation", icons::NAVMESH), |ui| {
        let mut add =
            |ui: &mut egui::Ui, enabled: bool, label: &str, component, value: serde_json::Value| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    actions.push(InspectorAction::SetPhysics {
                        entity,
                        component,
                        value,
                    });
                    ui.close();
                }
            };
        add(
            ui,
            components.nav_mesh_surface.is_none(),
            "NavMesh Surface",
            "nav_mesh_surface",
            serde_json::to_value(NavMeshSurface::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.nav_agent.is_none(),
            "Nav Agent",
            "nav_agent",
            serde_json::to_value(NavAgent::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.nav_obstacle.is_none(),
            "Nav Obstacle",
            "nav_obstacle",
            serde_json::to_value(NavObstacle::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.nav_modifier.is_none(),
            "Nav Modifier",
            "nav_modifier",
            serde_json::to_value(NavModifier::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.nav_link.is_none(),
            "Nav Link",
            "nav_link",
            serde_json::to_value(NavLink::default()).unwrap_or_default(),
        );
    });
}

fn surface_section(
    ui: &mut egui::Ui,
    entity: Entity,
    surface: &NavMeshSurface,
    status: &NavigationPanelStatus,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::NAVMESH, "NavMesh Surface", |ui| {
        let live = status.surface(entity);
        // What the navmesh is now.
        let summary = match live {
            Some(s) if s.tiles > 0 => format!(
                "{} polygons in {} tiles ({})",
                s.polys,
                s.tiles,
                match s.source {
                    "baked" => "from the bake",
                    "built" => "built at play start",
                    _ => "not built",
                }
            ),
            _ if surface.baked.is_some() => "The bake did not load".into(),
            _ if surface.build_on_start => "Not baked: it is built when play starts".into(),
            _ => "Not baked, and not built at play start".into(),
        };
        ui.label(theme::hint(summary));
        if let Some(error) = live.and_then(|s| s.error.as_ref()) {
            ui.label(
                egui::RichText::new(format!("{}  {error}", icons::WARNING))
                    .color(theme::colors::ERROR),
            );
        }
        if live.is_some_and(|s| s.stale) {
            ui.label(
                egui::RichText::new(format!(
                    "{}  The settings changed since the bake: bake again",
                    icons::WARNING
                ))
                .color(theme::colors::WARNING),
            );
        }
        ui.horizontal(|ui| {
            ui.label("Baked");
            match &surface.baked {
                Some(path) => {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    ui.monospace(name).on_hover_text(path);
                }
                None => {
                    ui.label(theme::hint("never"));
                }
            }
        });

        // Bake.
        let baking = status.bake.as_ref();
        if let Some(progress) = baking.filter(|p| p.surface == entity) {
            let share = progress.tiles_done as f32 / progress.tiles_total.max(1) as f32;
            ui.add(
                egui::ProgressBar::new(share)
                    .text(format!(
                        "Baking tile {}/{} ({:.1} s)",
                        progress.tiles_done, progress.tiles_total, progress.seconds
                    ))
                    .animate(true),
            );
        } else {
            let blocker = if baking.is_some() {
                Some("Another navmesh is baking")
            } else if status.playing {
                Some("Stop play to bake")
            } else {
                None
            };
            ui.horizontal(|ui| {
                let button = ui
                    .add_enabled(blocker.is_none(), theme::primary_button(format!("{}  Bake", icons::NAVMESH)))
                    .on_hover_text(
                        "Build the navmesh from the scene's static geometry (in the background) and save it to \
                         content/navigation/<entity name>.navmesh; save the project afterwards to keep the link",
                    );
                let button = match blocker {
                    Some(reason) => button.on_disabled_hover_text(reason),
                    None => button,
                };
                if button.clicked() {
                    actions.push(InspectorAction::Navigation(NavigationRequest::Bake { entity }));
                }
                if surface.baked.is_some()
                    && ui
                        .small_button("Clear")
                        .on_hover_text("Forget the bake (the file stays); the navmesh is then built at play start")
                        .clicked()
                {
                    set(actions, entity, "nav_mesh_surface", serde_json::json!({ "baked": null }));
                }
            });
            if !status.project_open {
                ui.label(theme::hint(
                    "No project is open: the bake is kept in memory only",
                ));
            }
        }

        ui.add_space(4.0);
        ui.separator();
        debug_controls(ui, status, actions);

        ui.add_space(4.0);
        ui.separator();
        let mut edited = surface.clone();
        let mut changed = false;
        ui.label(egui::RichText::new("Agents").strong());
        Grid::new(("nav_surface_agent", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |= float_drag_value(
                    ui,
                    "Radius (m)",
                    &mut edited.agent_radius,
                    0.01,
                    Some(0.05..=5.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Height (m)",
                    &mut edited.agent_height,
                    0.01,
                    Some(0.1..=10.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Max climb (m)",
                    &mut edited.agent_max_climb,
                    0.01,
                    Some(0.0..=5.0),
                );
                labeled_slider(
                    ui,
                    "Max slope (°)",
                    &mut edited.agent_max_slope,
                    0.0..=85.0,
                    1.0,
                    &mut changed,
                );
                ui.label("Agent type");
                changed |= ui
                    .text_edit_singleline(&mut edited.agent_type)
                    .on_hover_text(
                        "Agents with the same agent type walk this navmesh (empty: the default)",
                    )
                    .changed();
                ui.end_row();
            });
        egui::CollapsingHeader::new("Voxels and detail")
            .id_salt(("nav_surface_voxels", entity))
            .show(ui, |ui| {
                Grid::new(("nav_surface_voxel_grid", entity))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        changed |= float_drag_value(
                            ui,
                            "Cell size (m)",
                            &mut edited.cell_size,
                            0.005,
                            Some(0.02..=2.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Cell height (m)",
                            &mut edited.cell_height,
                            0.005,
                            Some(0.02..=2.0),
                        );
                        ui.label("Tile size (cells)");
                        changed |= ui
                            .add(DragValue::new(&mut edited.tile_size).range(16..=256))
                            .changed();
                        ui.end_row();
                        changed |= float_drag_value(
                            ui,
                            "Min region (m²)",
                            &mut edited.min_region_area,
                            0.1,
                            Some(0.0..=1000.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Merge region (m²)",
                            &mut edited.merge_region_area,
                            0.1,
                            Some(0.0..=10000.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Max edge length (m)",
                            &mut edited.max_edge_length,
                            0.1,
                            Some(0.0..=100.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Max edge error (cells)",
                            &mut edited.max_edge_error,
                            0.05,
                            Some(0.1..=10.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Detail spacing (m)",
                            &mut edited.detail_sample_distance,
                            0.05,
                            Some(0.0..=20.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Detail error (m)",
                            &mut edited.detail_sample_max_error,
                            0.01,
                            Some(0.01..=2.0),
                        );
                    });
            });
        egui::CollapsingHeader::new("Geometry")
            .id_salt(("nav_surface_geometry", entity))
            .show(ui, |ui| {
                Grid::new(("nav_surface_geometry_grid", entity)).num_columns(2).striped(true).show(ui, |ui| {
                    ui.label("Built from");
                    ComboBox::from_id_salt(("nav_geometry_source", entity))
                        .selected_text(edited.geometry.label())
                        .show_ui(ui, |ui| {
                            for option in NavGeometrySource::ALL {
                                changed |= ui.selectable_value(&mut edited.geometry, option, option.label()).changed();
                            }
                        });
                    ui.end_row();
                    ui.label("Collect");
                    ComboBox::from_id_salt(("nav_collect", entity))
                        .selected_text(edited.collect.label())
                        .show_ui(ui, |ui| {
                            for option in NavCollect::ALL {
                                changed |= ui.selectable_value(&mut edited.collect, option, option.label()).changed();
                            }
                        });
                    ui.end_row();
                    changed |= vec3_row(ui, "Bounds (m)", &mut edited.bounds, 0.1);
                    changed |= checkbox_row(
                        ui,
                        "Build at play start",
                        &mut edited.build_on_start,
                        "Without a bake, build the navmesh when play starts",
                    );
                });
                ui.label(theme::hint(
                    "Bounds limit the build to a box around this entity (zero: everything). Static visible \
                     meshes count; skinned meshes, dynamic bodies, agents and obstacles never do. A Nav \
                     Modifier leaves geometry out or makes it unwalkable.",
                ));
            });
        if changed {
            set(actions, entity, "nav_mesh_surface", &edited);
        }
    })
}

fn debug_controls(
    ui: &mut egui::Ui,
    status: &NavigationPanelStatus,
    actions: &mut Vec<InspectorAction>,
) {
    let mut debug = status.debug;
    let before = debug;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut debug.mesh, "Navmesh")
            .on_hover_text("Draw every navmesh (a selected surface always shows its own)");
        ui.checkbox(&mut debug.agents, "Agents");
        ui.checkbox(&mut debug.paths, "Paths");
        ui.checkbox(&mut debug.obstacles, "Obstacles");
        ui.checkbox(&mut debug.x_ray, "X-ray")
            .on_hover_text("Show the parts scenery hides, faintly");
    });
    if debug != before {
        actions.push(InspectorAction::Navigation(NavigationRequest::SetDebug(
            debug,
        )));
    }
    ui.horizontal(|ui| {
        let mut tool = status.path_tool;
        if ui
            .toggle_value(&mut tool, format!("{}  Test path", icons::NAV_AGENT))
            .on_hover_text("Click two points in the Scene view: a path between them is drawn")
            .changed()
        {
            actions.push(InspectorAction::Navigation(NavigationRequest::PathTool(
                tool,
            )));
        }
        if (status.path.from.is_some() || status.path.to.is_some())
            && ui.small_button("Clear").clicked()
        {
            actions.push(InspectorAction::Navigation(NavigationRequest::ClearPath));
        }
    });
    let path = &status.path;
    let text = match (path.from, path.to) {
        (None, _) if status.path_tool => Some("Click the start in the Scene view".to_string()),
        (Some(_), None) if status.path_tool => Some("Click the end".to_string()),
        (Some(_), Some(_)) if path.corners == 0 => {
            Some("No path: an end is off the navmesh".to_string())
        }
        (Some(_), Some(_)) if path.complete => {
            Some(format!("{:.1} m, {} corners", path.length, path.corners))
        }
        (Some(_), Some(_)) => Some(format!(
            "Blocked: walks {:.1} m as near as it can get",
            path.length
        )),
        _ => None,
    };
    if let Some(text) = text {
        ui.label(theme::hint(text));
    }
}

fn agent_section(
    ui: &mut egui::Ui,
    entity: Entity,
    agent: &NavAgent,
    has_controller: bool,
    status: &NavigationPanelStatus,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::NAV_AGENT, "Nav Agent", |ui| {
        if let Some(live) = &status.selected_agent {
            let speed = Vec3::new(live.velocity.x, 0.0, live.velocity.z).length();
            let mut text = format!("{} at {speed:.2} m/s", live.state.name());
            if let Some(remaining) = live.remaining_distance {
                text.push_str(&format!(", {remaining:.1} m to go"));
            }
            if live.partial {
                text.push_str(" (blocked: as near as it can)");
            }
            ui.label(theme::hint(text));
        } else if status.playing {
            ui.label(theme::hint("Not walking: no navmesh for its agent type"));
        }
        let mut edited = agent.clone();
        let mut changed = false;
        Grid::new(("nav_agent_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |=
                    float_drag_value(ui, "Speed (m/s)", &mut edited.speed, 0.05, Some(0.0..=50.0));
                changed |= float_drag_value(
                    ui,
                    "Acceleration (m/s²)",
                    &mut edited.acceleration,
                    0.1,
                    Some(0.1..=200.0),
                );
                changed |= float_drag_value(
                    ui,
                    "Turn speed (°/s)",
                    &mut edited.turn_speed,
                    5.0,
                    Some(0.0..=3600.0),
                );
                changed |=
                    float_drag_value(ui, "Radius (m)", &mut edited.radius, 0.01, Some(0.05..=5.0));
                changed |=
                    float_drag_value(ui, "Height (m)", &mut edited.height, 0.01, Some(0.1..=10.0));
                changed |= float_drag_value(
                    ui,
                    "Stopping distance (m)",
                    &mut edited.stopping_distance,
                    0.01,
                    Some(0.0..=10.0),
                );
                changed |= checkbox_row(
                    ui,
                    "Avoidance",
                    &mut edited.avoidance,
                    "Steer around other agents and moving obstacles",
                );
                labeled_slider(
                    ui,
                    "Priority",
                    &mut edited.priority,
                    0.0..=1.0,
                    0.05,
                    &mut changed,
                );
                labeled_slider(
                    ui,
                    "Separation",
                    &mut edited.separation,
                    0.0..=4.0,
                    0.05,
                    &mut changed,
                );
                changed |= checkbox_row(
                    ui,
                    "Face movement",
                    &mut edited.face_movement,
                    "Turn toward where it walks",
                );
                changed |= float_drag_value(
                    ui,
                    "Yaw offset (°)",
                    &mut edited.yaw_offset,
                    1.0,
                    Some(-360.0..=360.0),
                );
                ui.label("Moves by");
                ComboBox::from_id_salt(("nav_agent_drive", entity))
                    .selected_text(edited.drive.label())
                    .show_ui(ui, |ui| {
                        for option in NavAgentDrive::ALL {
                            changed |= ui
                                .selectable_value(&mut edited.drive, option, option.label())
                                .changed();
                        }
                    });
                ui.end_row();
                ui.label("Agent type");
                changed |= ui.text_edit_singleline(&mut edited.agent_type).changed();
                ui.end_row();
                ui.label("Destination");
                ui.horizontal(|ui| {
                    let mut has = edited.destination.is_some();
                    if ui
                        .checkbox(&mut has, "")
                        .on_hover_text("Head here when play starts")
                        .changed()
                    {
                        edited.destination = has.then_some([0.0; 3]);
                        changed = true;
                    }
                    if let Some(destination) = edited.destination.as_mut() {
                        for component in destination.iter_mut() {
                            changed |= ui.add(DragValue::new(component).speed(0.1)).changed();
                        }
                    }
                });
                ui.end_row();
            });
        let drives_character = match edited.drive {
            NavAgentDrive::Auto | NavAgentDrive::Character => has_controller,
            NavAgentDrive::Transform => false,
        };
        ui.label(theme::hint(if drives_character {
            "Steers its Character Controller (physics moves it)"
        } else {
            "Moves the entity along the navmesh; an Animation Tree reading from the character gets its speed"
        }));
        if !edited.area_costs.is_empty() {
            ui.label(theme::hint(format!(
                "{} area costs (set them from Lua or MCP)",
                edited.area_costs.len()
            )));
        }
        if changed {
            set(actions, entity, "nav_agent", &edited);
        }
    })
}

fn obstacle_section(
    ui: &mut egui::Ui,
    entity: Entity,
    obstacle: &NavObstacle,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::NAV_OBSTACLE, "Nav Obstacle", |ui| {
        let mut edited = obstacle.clone();
        let mut changed = false;
        Grid::new(("nav_obstacle_grid", entity)).num_columns(2).striped(true).show(ui, |ui| {
            ui.label("Shape");
            let is_box = matches!(edited.shape, NavObstacleShape::Box { .. });
            ComboBox::from_id_salt(("nav_obstacle_shape", entity))
                .selected_text(if is_box { "Box" } else { "Cylinder" })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(is_box, "Box").clicked() && !is_box {
                        edited.shape = NavObstacleShape::Box { size: [1.0; 3] };
                        changed = true;
                    }
                    if ui.selectable_label(!is_box, "Cylinder").clicked() && is_box {
                        edited.shape = NavObstacleShape::Cylinder { radius: 0.5, height: 1.0 };
                        changed = true;
                    }
                });
            ui.end_row();
            match &mut edited.shape {
                NavObstacleShape::Box { size } => changed |= vec3_row(ui, "Size", size, 0.01),
                NavObstacleShape::Cylinder { radius, height } => {
                    changed |= float_drag_value(ui, "Radius", radius, 0.01, Some(0.01..=100.0));
                    changed |= float_drag_value(ui, "Height", height, 0.01, Some(0.01..=100.0));
                }
            }
            changed |= vec3_row(ui, "Offset", &mut edited.offset, 0.01);
            changed |= checkbox_row(ui, "Carve", &mut edited.carve, "Cut a hole into the navmesh so paths go around it");
            changed |= checkbox_row(
                ui,
                "Only when still",
                &mut edited.carve_only_stationary,
                "Carve only after standing still for a moment (while moving, agents steer around it)",
            );
            changed |= float_drag_value(ui, "Move threshold (m)", &mut edited.move_threshold, 0.01, Some(0.001..=10.0));
            changed |= float_drag_value(ui, "Still for (s)", &mut edited.stationary_time, 0.05, Some(0.0..=30.0));
        });
        if changed {
            set(actions, entity, "nav_obstacle", &edited);
        }
    })
}

fn modifier_section(
    ui: &mut egui::Ui,
    entity: Entity,
    modifier: &NavModifier,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::NAV_MODIFIER, "Nav Modifier", |ui| {
        let mut edited = modifier.clone();
        let mut changed = false;
        Grid::new(("nav_modifier_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |= checkbox_row(
                    ui,
                    "Ignore",
                    &mut edited.ignore,
                    "Leave this geometry out of navmeshes",
                );
                changed |= checkbox_row(
                    ui,
                    "Walkable",
                    &mut edited.walkable,
                    "Its tops can be walked on (off: solid but never walked on, like rooftops)",
                );
                ui.label("Area");
                changed |= ui
                    .add(DragValue::new(&mut edited.area).range(1..=63))
                    .on_hover_text(
                        "1..62 are custom areas agents can give costs; 63 is plain ground",
                    )
                    .changed();
                ui.end_row();
                changed |= checkbox_row(
                    ui,
                    "Children too",
                    &mut edited.include_children,
                    "Apply to the entity's descendants",
                );
            });
        if changed {
            set(actions, entity, "nav_modifier", &edited);
        }
    })
}

fn link_section(
    ui: &mut egui::Ui,
    entity: Entity,
    link: &NavLink,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::NAV_LINK, "Nav Link", |ui| {
        let mut edited = link.clone();
        let mut changed = false;
        Grid::new(("nav_link_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                changed |= vec3_row(ui, "End (local)", &mut edited.end, 0.05);
                changed |= checkbox_row(
                ui,
                "Both ways",
                &mut edited.bidirectional,
                "Crossable both ways (a ladder, a gap); off: only from here to the end (a drop)",
            );
                changed |=
                    float_drag_value(ui, "Radius (m)", &mut edited.radius, 0.01, Some(0.05..=5.0));
                ui.label("Area");
                changed |= ui
                    .add(DragValue::new(&mut edited.area).range(1..=63))
                    .changed();
                ui.end_row();
            });
        ui.label(theme::hint(
            "Starts at this entity and lands at End; both ends must be near a navmesh. Agents cross it in an \
             arc and scripts hear on_nav_link_start / on_nav_link_end. Shown in yellow with the navmesh.",
        ));
        if changed {
            set(actions, entity, "nav_link", &edited);
        }
    })
}
