use std::time::Instant;

use glam::{Vec2, Vec3};
use wgpu_cube::app::RuntimeMode;
use wgpu_cube::navigation::QueryFilter;

use super::core::{EditorApplication, NotificationSeverity, ReloadNotification};
use super::system::{EditorContext, EditorSystem};
use crate::inspector::{NavigationPanelStatus, PathTestStatus};

/// The path test: while active, clicks in the Scene view set its ends instead of
/// selecting (start, then end, then a new start).
#[derive(Clone, Debug, Default)]
pub(crate) struct NavPathTool {
    pub active: bool,
    pub from: Option<Vec3>,
    pub to: Option<Vec3>,
    /// A click to place an end at, as a fraction of the Scene view.
    pub pending_click: Option<Vec2>,
}

/// Announces finished navmesh bakes (and records them for undo), places the path
/// test's ends where clicks meet the navmesh, and refreshes what the navigation sections
/// of the inspector show.
#[derive(Default)]
pub(crate) struct NavigationSystem {
    last_bake: Option<u32>,
}

fn notification(message: String, severity: NotificationSeverity) -> ReloadNotification {
    ReloadNotification {
        message,
        severity,
        timestamp: Instant::now(),
    }
}

impl EditorSystem for NavigationSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let last_bake = &mut self.last_bake;
        let notifications = ctx.with_gpu_app(|app, gpu| {
            let mut notifications = Vec::new();
            let playing = app.runtime_state().active_mode() == RuntimeMode::Playing;

            // A finished bake set the surface's baked path: an undoable change.
            let outcome = gpu.scene.navigation().last_bake().cloned();
            if let Some(outcome) = outcome {
                if *last_bake != Some(outcome.serial) {
                    *last_bake = Some(outcome.serial);
                    match &outcome.result {
                        Ok(path) if !path.is_empty() => {
                            app.record_scene_change(&mut gpu.scene);
                            let stats = outcome.stats.as_ref();
                            notifications.push(notification(
                                format!(
                                    "Navmesh baked into {path} ({} polygons, {:.1} s); save the project to keep it",
                                    stats.map_or(0, |s| s.polys),
                                    stats.map_or(0.0, |s| s.millis / 1000.0)
                                ),
                                NotificationSeverity::Success,
                            ));
                        }
                        Ok(_) => notifications.push(notification(
                            "Navmesh baked (no project is open, so it is not saved)".into(),
                            NotificationSeverity::Warning,
                        )),
                        Err(error) => notifications.push(notification(
                            format!("Navmesh bake failed: {error}"),
                            NotificationSeverity::Error,
                        )),
                    }
                }
            }

            // A click for the path test: where the ray meets the navmesh.
            let tool = app.nav_path_tool_mut();
            if let Some(uv) = tool.pending_click.take() {
                let region = app.scene_viewport().region();
                if let Some(region) = region {
                    let aspect = region.width().max(1) as f32 / region.height().max(1) as f32;
                    let (origin, direction) = EditorApplication::ray_from_uv(gpu.scene.camera(), uv, aspect);
                    match gpu.scene.navigation().pick(origin, direction) {
                        Some(point) => {
                            let tool = app.nav_path_tool_mut();
                            if tool.from.is_none() || tool.to.is_some() {
                                tool.from = Some(point);
                                tool.to = None;
                            } else {
                                tool.to = Some(point);
                            }
                        }
                        None => notifications.push(notification(
                            "That point is not on a navmesh (bake one, or click a walkable floor)".into(),
                            NotificationSeverity::Warning,
                        )),
                    }
                }
            }
            let tool = app.nav_path_tool_mut().clone();
            let navigation = gpu.scene.navigation_mut();
            match (tool.from, tool.to) {
                (Some(from), Some(to)) => navigation.set_test_path(Some((from, to))),
                (Some(from), None) => navigation.set_test_path(Some((from, from))),
                _ => {
                    if navigation.test_path().is_some() && !tool.active {
                        // Leave paths set by tools (MCP) alone unless the tool was used.
                    }
                }
            }

            let navigation = gpu.scene.navigation();
            let world = gpu.scene.main_world();
            let path = match (tool.from, tool.to) {
                (Some(from), Some(to)) => {
                    let found = navigation.find_path(from, to, "", &QueryFilter::default());
                    PathTestStatus {
                        from: Some(from),
                        to: Some(to),
                        complete: found.as_ref().is_some_and(|f| f.1),
                        length: found.as_ref().map_or(0.0, |f| f.2),
                        corners: found.as_ref().map_or(0, |f| f.0.len()),
                    }
                }
                (from, to) => PathTestStatus {
                    from,
                    to,
                    ..Default::default()
                },
            };
            let selected_agent = app
                .selection_system()
                .selected()
                .filter(|_| playing)
                .and_then(|entity| navigation.agent_status(entity));
            *app.navigation_status_mut() = NavigationPanelStatus {
                surfaces: navigation.surface_statuses(world),
                bake: navigation.bake_progress(),
                debug: navigation.debug_draw(),
                path_tool: tool.active,
                path,
                playing,
                project_open: wgpu_cube::project::active_project_root().is_some(),
                selected_agent,
            };
            notifications
        });
        for message in notifications.unwrap_or_default() {
            ctx.notify(message);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
