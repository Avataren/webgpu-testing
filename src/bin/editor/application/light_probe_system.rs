use std::time::Instant;

use super::core::{NotificationSeverity, ReloadNotification};
use super::system::{EditorContext, EditorSystem};
use crate::inspector::{ActiveProbeGrid, LightProbePanelStatus, LightProbeRequest};

/// Carries out the inspector's light probe requests (bake, debug view) on the renderer,
/// and refreshes the state the light probe section shows.
#[derive(Default)]
pub(crate) struct LightProbeSystem {
    /// The serial of the last bake outcome seen, so each finished bake (from the
    /// inspector or MCP) is announced and recorded once.
    last_outcome: Option<u32>,
}

impl EditorSystem for LightProbeSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let last_outcome = &mut self.last_outcome;
        let notifications = ctx.with_gpu_app(|app, gpu| {
            let mut notifications = Vec::new();
            for request in app.take_light_probe_requests() {
                match request {
                    LightProbeRequest::Bake { entity, quality } => {
                        let (warmup, sweeps, rays) = quality.settings();
                        let volume = [entity.to_bits().get()];
                        match gpu.renderer.start_light_probe_bake(
                            Some(&volume),
                            warmup,
                            sweeps,
                            rays,
                        ) {
                            Ok(frames) => log::info!(
                                "Light probes: {} bake started ({frames} frames)",
                                quality.label()
                            ),
                            Err(error) => notifications.push(notification(
                                format!("Light probes: cannot bake: {error}"),
                                NotificationSeverity::Error,
                            )),
                        }
                    }
                    LightProbeRequest::SetDebug(view) => gpu.renderer.set_light_probe_debug(view),
                }
            }

            // A bake the app saved: the volume points at the new file.
            let outcome = gpu.renderer.light_probe_bake_outcome().cloned();
            if let Some(outcome) = outcome {
                if *last_outcome != Some(outcome.serial) {
                    *last_outcome = Some(outcome.serial);
                    match outcome.result {
                        Ok(path) => {
                            app.record_scene_change(&mut gpu.scene);
                            notifications.push(notification(
                                format!(
                                    "Light probes baked into {path}; save the project to keep it"
                                ),
                                NotificationSeverity::Success,
                            ));
                        }
                        Err(error) => notifications.push(notification(
                            format!("Light probes: {error}"),
                            NotificationSeverity::Error,
                        )),
                    }
                }
            }

            let volumes = wgpu_cube::scene::light_probe_volumes(gpu.scene.main_world());
            let renderer = &*gpu.renderer;
            *app.light_probe_status_mut() = LightProbePanelStatus {
                volumes: volumes
                    .iter()
                    .map(|volume| ActiveProbeGrid {
                        entity: volume.entity,
                        counts: volume.grid.counts,
                        spacing: volume.grid.spacing,
                        showing_bake: renderer
                            .light_probe_baked_source(volume.grid.id)
                            .map(str::to_owned),
                    })
                    .collect(),
                can_trace: renderer.light_probes_can_trace(),
                bake: renderer.light_probe_bake_progress(),
                debug: renderer.light_probe_debug(),
                project_open: wgpu_cube::project::active_project_root().is_some(),
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

fn notification(message: String, severity: NotificationSeverity) -> ReloadNotification {
    ReloadNotification {
        message,
        severity,
        timestamp: Instant::now(),
    }
}
