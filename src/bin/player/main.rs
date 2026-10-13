#[cfg(feature = "egui")]
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "egui")]
use hecs::Entity;

use wgpu_cube::app::{StartupContext, UpdateContext};
use wgpu_cube::project::ProjectManifest;
use wgpu_cube::scene::SceneLibrary;

#[cfg(target_arch = "wasm32")]
use wgpu_cube::project::PROJECT_FILE_NAME;
use wgpu_cube::render_application::{run_application, RenderApplication};

#[cfg(feature = "egui")]
use wgpu_cube::scripting::lua::api::ui::{merge_ui_responses, UiCommand, UiResponse};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Widget responses for one script, keyed by widget id.
#[cfg(feature = "egui")]
type ScriptUiResponses = HashMap<String, UiResponse>;

/// `on_ui` output of the scene's scripts, collected in `update` and drawn in `ui`.
#[cfg(feature = "egui")]
#[derive(Default)]
struct ScriptUi {
    commands: HashMap<Entity, Vec<UiCommand>>,
    titles: HashMap<Entity, String>,
    responses: HashMap<Entity, ScriptUiResponses>,
    /// Logical size and scale of the window, from the last UI pass.
    viewport: Option<(f32, f32, f32)>,
}

struct ProjectPlayer {
    manifest: ProjectManifest,
    project_root: PathBuf,
    loaded: bool,
    library: SceneLibrary,
    #[cfg(feature = "egui")]
    script_ui: ScriptUi,
}

impl ProjectPlayer {
    fn new(manifest: ProjectManifest, project_root: PathBuf) -> Self {
        Self {
            manifest,
            project_root,
            loaded: false,
            library: SceneLibrary::new(),
            #[cfg(feature = "egui")]
            script_ui: ScriptUi::default(),
        }
    }
}

impl RenderApplication for ProjectPlayer {
    fn name(&self) -> &str {
        &self.manifest.metadata.name
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        if self.loaded {
            return;
        }

        match self.manifest.instantiate_workspace(
            ctx.renderer,
            &self.project_root,
            &mut self.library,
        ) {
            Ok((workspace, textures_changed)) => {
                ctx.request_workspace_swap(workspace, textures_changed);
                log::info!(
                    "Loaded project '{}' from {}",
                    self.manifest.metadata.name,
                    self.project_root.display()
                );
                self.loaded = true;
            }
            Err(err) => {
                log::error!("Failed to instantiate project: {err}");
            }
        }
    }

    #[cfg(not(feature = "egui"))]
    fn update(&mut self, _ctx: &mut UpdateContext) {}

    #[cfg(feature = "egui")]
    fn update(&mut self, ctx: &mut UpdateContext) {
        let ui = &mut self.script_ui;
        // Last frame's clicks and edits go back to the scripts before they build new UI.
        ctx.scene
            .set_ui_responses(std::mem::take(&mut ui.responses));
        ui.commands = ctx.scene.process_script_ui(false, ui.viewport);
        let world = ctx.scene.main_world();
        ui.titles = ui
            .commands
            .keys()
            .map(|&entity| {
                let title = world
                    .get::<&wgpu_cube::scene::Name>(entity)
                    .map(|name| name.0.clone())
                    .unwrap_or_else(|_| format!("Entity {}", entity.id()));
                (entity, title)
            })
            .collect();
    }

    /// A game shows only its own UI: script HUDs, not the engine's debug windows.
    #[cfg(feature = "egui")]
    fn show_default_ui(&self) -> bool {
        false
    }

    #[cfg(feature = "egui")]
    fn ui(&mut self, ui: &mut egui::Ui, _default_ui: &mut wgpu_cube::DefaultUI) {
        let ctx = ui.ctx().clone();
        let rect = ctx.viewport_rect();
        let script_ui = &mut self.script_ui;
        script_ui.viewport = Some((rect.width(), rect.height(), ctx.pixels_per_point()));

        let mut all_responses = HashMap::new();
        for (entity, commands) in &script_ui.commands {
            let mut responses = HashMap::new();
            let (overlays, widgets): (Vec<&UiCommand>, Vec<&UiCommand>) = commands
                .iter()
                .partition(|command| matches!(command, UiCommand::AnchoredPanel { .. }));

            if !overlays.is_empty() {
                egui::Area::new(egui::Id::new(("script_hud", *entity)))
                    .fixed_pos(rect.min)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        for command in &overlays {
                            command.render_and_collect(ui, &mut responses, Some(rect));
                        }
                    });
            }

            if !widgets.is_empty() {
                let title = script_ui
                    .titles
                    .get(entity)
                    .cloned()
                    .unwrap_or_else(|| format!("Entity {}", entity.id()));
                egui::Window::new(title)
                    .id(egui::Id::new(("script_ui", *entity)))
                    .resizable(true)
                    .default_size([300.0, 200.0])
                    .show(&ctx, |ui| {
                        for command in &widgets {
                            command.render_and_collect(ui, &mut responses, Some(rect));
                        }
                    });
            }

            if !responses.is_empty() {
                all_responses.insert(*entity, responses);
            }
        }

        // egui may run several passes per frame; only the first gets input, so later
        // passes must not overwrite its clicks.
        if ctx.current_pass_index() == 0 {
            script_ui.responses = all_responses;
        } else {
            merge_ui_responses(&mut script_ui.responses, all_responses);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    let manifest = ProjectManifest::load_from_dir(&project_dir)?;
    run_application(ProjectPlayer::new(manifest, project_dir))?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    if let Err(err) = start_app_internal() {
        log::error!("Failed to start project: {:?}", err);
    }
}

#[cfg(target_arch = "wasm32")]
fn start_app_internal() -> Result<(), JsValue> {
    let bytes = wgpu_cube::io::load_binary_from_str(PROJECT_FILE_NAME)
        .map_err(|err| JsValue::from_str(&format!("Failed to load manifest: {err}")))?;

    let mut manifest = ProjectManifest::from_json_bytes(&bytes)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    manifest
        .environment
        .resolve_paths(Path::new("."))
        .map_err(|err| JsValue::from_str(&err.to_string()))?;

    run_application(ProjectPlayer::new(manifest, PathBuf::from(".")))
}
