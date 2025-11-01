use crate::renderer::texture::{
    DEFAULT_CHECKER_TEXTURE_INDEX, DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
    DEFAULT_NORMAL_TEXTURE_INDEX, DEFAULT_WHITE_TEXTURE_INDEX,
};
use crate::renderer::{Renderer, Texture};
use crate::scene::workspace::SceneDocumentId;
use crate::scene::{Scene, SceneHandle, SceneWorkspace, SceneWorkspaceSceneMut};
use crate::time::Instant;

use super::runtime::RuntimeMode;

struct WorkspaceSwapRequest {
    workspace: SceneWorkspace,
    textures_changed: bool,
}

pub struct StartupContext<'a> {
    pub scene_handle: SceneHandle,
    pub scene: SceneWorkspaceSceneMut<'a>,
    pub renderer: &'a mut Renderer,
    workspace_swap: Option<WorkspaceSwapRequest>,
    active_scene_request: Option<SceneDocumentId>,
}

impl<'a> StartupContext<'a> {
    pub fn request_workspace_swap(&mut self, workspace: SceneWorkspace, textures_changed: bool) {
        self.workspace_swap = Some(WorkspaceSwapRequest {
            workspace,
            textures_changed,
        });
    }

    pub fn request_active_scene(&mut self, document_id: SceneDocumentId) {
        self.active_scene_request = Some(document_id);
    }

    fn take_workspace_swap(&mut self) -> Option<WorkspaceSwapRequest> {
        self.workspace_swap.take()
    }

    fn take_active_scene_request(&mut self) -> Option<SceneDocumentId> {
        self.active_scene_request.take()
    }
}

pub struct UpdateContext<'a> {
    pub scene_handle: SceneHandle,
    pub scene: SceneWorkspaceSceneMut<'a>,
    pub dt: f64,
    pub runtime: RuntimeMode,
    active_scene_request: Option<SceneDocumentId>,
}

impl<'a> UpdateContext<'a> {
    pub fn request_active_scene(&mut self, document_id: SceneDocumentId) {
        self.active_scene_request = Some(document_id);
    }

    fn take_active_scene_request(&mut self) -> Option<SceneDocumentId> {
        self.active_scene_request.take()
    }
}

pub struct GpuUpdateContext<'a> {
    pub scene_handle: SceneHandle,
    pub scene: SceneWorkspaceSceneMut<'a>,
    pub renderer: &'a mut Renderer,
    pub dt: f64,
    workspace_swap: Option<WorkspaceSwapRequest>,
    active_scene_request: Option<SceneDocumentId>,
}

impl<'a> GpuUpdateContext<'a> {
    pub fn request_workspace_swap(&mut self, workspace: SceneWorkspace, textures_changed: bool) {
        self.workspace_swap = Some(WorkspaceSwapRequest {
            workspace,
            textures_changed,
        });
    }

    pub fn request_active_scene(&mut self, document_id: SceneDocumentId) {
        self.active_scene_request = Some(document_id);
    }

    fn take_workspace_swap(&mut self) -> Option<WorkspaceSwapRequest> {
        self.workspace_swap.take()
    }

    fn take_active_scene_request(&mut self) -> Option<SceneDocumentId> {
        self.active_scene_request.take()
    }
}

pub type StartupSystem = Box<dyn for<'a> FnMut(&mut StartupContext<'a>) + 'static>;
pub type UpdateSystem = Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static>;
pub type GpuUpdateSystem = Box<dyn for<'a> FnMut(&mut GpuUpdateContext<'a>) + 'static>;

pub struct FrameStep {
    dt: f64,
    skip_rendering: bool,
    scene_handle: SceneHandle,
}

impl FrameStep {
    pub fn dt(&self) -> f64 {
        self.dt
    }

    pub fn should_render(&self) -> bool {
        !self.skip_rendering
    }

    pub fn scene_handle(&self) -> SceneHandle {
        self.scene_handle
    }
}

pub struct Scheduler {
    startup_systems: Vec<StartupSystem>,
    update_systems: Vec<UpdateSystem>,
    gpu_systems: Vec<GpuUpdateSystem>,
    auto_init_default_textures: bool,
    auto_add_default_lighting: bool,
    startup_ran: bool,
    frame_counter: u32,
    skip_rendering_until_frame: Option<u32>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            startup_systems: Vec::new(),
            update_systems: Vec::new(),
            gpu_systems: Vec::new(),
            auto_init_default_textures: true,
            auto_add_default_lighting: true,
            startup_ran: false,
            frame_counter: 0,
            skip_rendering_until_frame: None,
        }
    }
}

impl Scheduler {
    pub fn add_startup_system(&mut self, system: StartupSystem) {
        self.startup_systems.push(system);
    }

    pub fn add_update_system(&mut self, system: UpdateSystem) {
        self.update_systems.push(system);
    }

    pub fn add_gpu_system(&mut self, system: GpuUpdateSystem) {
        self.gpu_systems.push(system);
    }

    pub fn disable_default_textures(&mut self) {
        self.auto_init_default_textures = false;
    }

    pub fn disable_default_lighting(&mut self) {
        self.auto_add_default_lighting = false;
    }

    pub fn skip_initial_frames(&mut self, frames: u32) {
        self.skip_rendering_until_frame = Some(frames);
    }

    pub fn begin_frame(&mut self, handle: SceneHandle, scene: &mut Scene) -> FrameStep {
        self.frame_counter = self.frame_counter.saturating_add(1);

        let skip_rendering = if let Some(skip_until) = self.skip_rendering_until_frame {
            if self.frame_counter < skip_until {
                true
            } else {
                self.skip_rendering_until_frame = None;
                false
            }
        } else {
            false
        };

        let now = Instant::now();
        let last_frame = match scene.last_frame_instant() {
            Some(last_frame) => last_frame,
            None => {
                scene.set_last_frame(now);
                now
            }
        };
        let dt = (now - last_frame).as_secs_f64();
        scene.set_last_frame(now);

        FrameStep {
            dt,
            skip_rendering,
            scene_handle: handle,
        }
    }

    pub fn run_startup_systems(
        &mut self,
        workspace: &mut SceneWorkspace,
        handle: SceneHandle,
        renderer: &mut Renderer,
    ) -> Option<SceneHandle> {
        if self.startup_ran {
            return Some(handle);
        }

        let mut active_handle = handle;
        let mut textures_changed = false;
        let mut workspace_swapped = false;

        if self.auto_init_default_textures {
            if let Some(mut scene) = workspace.scene_mut_by_handle(active_handle) {
                if scene.assets.textures.is_empty() {
                    self.init_default_textures(&mut scene, renderer);
                    textures_changed = true;
                }
            } else {
                log::warn!(
                    "Startup initialization skipped: active scene handle {:?} is unavailable",
                    active_handle
                );
                return None;
            }
        }

        for system in &mut self.startup_systems {
            let Some(scene) = workspace.scene_mut_by_handle(active_handle) else {
                log::warn!(
                    "Startup system skipped: active scene handle {:?} is unavailable",
                    active_handle
                );
                return None;
            };

            let mut ctx = StartupContext {
                scene_handle: active_handle,
                scene,
                renderer,
                workspace_swap: None,
                active_scene_request: None,
            };
            (system)(&mut ctx);

            let swap = ctx.take_workspace_swap();
            let active_request = ctx.take_active_scene_request();
            active_handle = ctx.scene_handle;
            drop(ctx);

            let (new_handle, swap_changed, swapped) =
                Self::apply_workspace_requests(workspace, active_handle, swap, active_request);
            active_handle = new_handle;
            textures_changed |= swap_changed;
            workspace_swapped |= swapped;
        }

        if let Some(mut scene) = workspace.scene_mut_by_handle(active_handle) {
            if self.auto_init_default_textures
                && (workspace_swapped || scene.assets.textures.is_empty())
            {
                if scene.assets.textures.is_empty() {
                    self.init_default_textures(&mut scene, renderer);
                    textures_changed = true;
                }
            }

            if self.auto_add_default_lighting {
                let added_lights = scene.add_default_lighting();
                if added_lights > 0 {
                    log::info!("Added {} default lights to scene", added_lights);
                }
            }

            log::info!("Running initial transform propagation...");
            scene.set_animation_playback(false);
            scene.update(0.0);
            log::info!("Initial propagation complete");
        } else {
            log::warn!(
                "Startup post-processing skipped: active scene handle {:?} is unavailable",
                active_handle
            );
            return None;
        }

        if textures_changed {
            renderer.update_texture_bind_group(workspace.assets());
        }

        self.startup_ran = true;
        Some(active_handle)
    }

    pub fn run_update_stage(
        &mut self,
        workspace: &mut SceneWorkspace,
        handle: SceneHandle,
        dt: f64,
        runtime: RuntimeMode,
    ) -> Option<SceneHandle> {
        let mut active_handle = handle;

        if runtime == RuntimeMode::Playing {
            if let Some(mut scene) = workspace.scene_mut_by_handle(active_handle) {
                scene.update(dt);
            } else {
                log::warn!(
                    "Update skipped: active scene handle {:?} is unavailable",
                    active_handle
                );
                return None;
            }
        }

        for system in &mut self.update_systems {
            let Some(scene) = workspace.scene_mut_by_handle(active_handle) else {
                log::warn!(
                    "Update system skipped: active scene handle {:?} is unavailable",
                    active_handle
                );
                return None;
            };

            let mut ctx = UpdateContext {
                scene_handle: active_handle,
                scene,
                dt,
                runtime,
                active_scene_request: None,
            };
            (system)(&mut ctx);

            let active_request = ctx.take_active_scene_request();
            active_handle = ctx.scene_handle;
            drop(ctx);

            let (new_handle, _, _) =
                Self::apply_workspace_requests(workspace, active_handle, None, active_request);
            active_handle = new_handle;
        }

        if let Some(mut scene) = workspace.scene_mut_by_handle(active_handle) {
            scene.propagate_transforms();
            Some(active_handle)
        } else {
            log::warn!(
                "Transform propagation skipped: active scene handle {:?} is unavailable",
                active_handle
            );
            None
        }
    }

    pub fn run_gpu_systems(
        &mut self,
        workspace: &mut SceneWorkspace,
        handle: SceneHandle,
        renderer: &mut Renderer,
        dt: f64,
    ) -> Option<SceneHandle> {
        let mut active_handle = handle;
        let mut textures_changed = false;
        let mut workspace_swapped = false;

        for system in &mut self.gpu_systems {
            let Some(scene) = workspace.scene_mut_by_handle(active_handle) else {
                log::warn!(
                    "GPU system skipped: active scene handle {:?} is unavailable",
                    active_handle
                );
                return None;
            };

            let mut ctx = GpuUpdateContext {
                scene_handle: active_handle,
                scene,
                renderer,
                dt,
                workspace_swap: None,
                active_scene_request: None,
            };
            (system)(&mut ctx);

            let swap = ctx.take_workspace_swap();
            let active_request = ctx.take_active_scene_request();
            active_handle = ctx.scene_handle;
            drop(ctx);

            let (new_handle, swap_changed, swapped) =
                Self::apply_workspace_requests(workspace, active_handle, swap, active_request);
            active_handle = new_handle;
            textures_changed |= swap_changed;
            workspace_swapped |= swapped;
        }

        if let Some(mut scene) = workspace.scene_mut_by_handle(active_handle) {
            scene.propagate_transforms();
        } else {
            log::warn!(
                "GPU propagation skipped: active scene handle {:?} is unavailable",
                active_handle
            );
            return None;
        }

        if textures_changed {
            renderer.update_texture_bind_group(workspace.assets());
        }

        if workspace_swapped {
            log::info!("Workspace swap applied during GPU systems");
        }

        Some(active_handle)
    }

    fn apply_workspace_requests(
        workspace: &mut SceneWorkspace,
        mut handle: SceneHandle,
        swap: Option<WorkspaceSwapRequest>,
        active_request: Option<SceneDocumentId>,
    ) -> (SceneHandle, bool, bool) {
        let mut textures_changed = false;
        let mut workspace_swapped = false;

        if let Some(request) = swap {
            *workspace = request.workspace;
            textures_changed |= request.textures_changed;
            workspace_swapped = true;

            if let Some(new_handle) = workspace.active_scene_handle() {
                handle = new_handle;
            } else {
                log::warn!(
                    "Workspace swap resulted in no active scene; retaining previous handle {:?}",
                    handle
                );
            }
        }

        if let Some(document_id) = active_request {
            match workspace.set_active_scene(&document_id) {
                Ok(new_handle) => {
                    handle = new_handle;
                }
                Err(err) => {
                    log::warn!("Failed to activate scene '{}': {}", document_id, err);
                }
            }
        }

        (handle, textures_changed, workspace_swapped)
    }

    fn init_default_textures(&mut self, scene: &mut Scene, renderer: &mut Renderer) {
        let device = renderer.get_device();
        let queue = renderer.get_queue();

        let white = Texture::white(device, queue);
        let white_handle = scene.assets.textures.insert(white);
        debug_assert_eq!(
            white_handle.index() as u32,
            DEFAULT_WHITE_TEXTURE_INDEX,
            "Default white texture index changed; update the constants in renderer::texture",
        );

        let normal = Texture::default_normal(device, queue);
        let normal_handle = scene.assets.textures.insert(normal);
        debug_assert_eq!(
            normal_handle.index() as u32,
            DEFAULT_NORMAL_TEXTURE_INDEX,
            "Default normal texture index changed; update the constants in renderer::texture",
        );

        let mr = Texture::default_metallic_roughness(device, queue);
        let mr_handle = scene.assets.textures.insert(mr);
        debug_assert_eq!(
            mr_handle.index() as u32,
            DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
            "Default metallic-roughness texture index changed; update the constants in renderer::texture",
        );

        let checker = Texture::checkerboard(
            device,
            queue,
            128,
            16,
            [255, 255, 255, 255],
            [24, 24, 24, 255],
            Some("DefaultCheckerboard"),
        );
        let checker_handle = scene.assets.textures.insert(checker);
        debug_assert_eq!(
            checker_handle.index() as u32,
            DEFAULT_CHECKER_TEXTURE_INDEX,
            "Default checker texture index changed; update the constants in renderer::texture",
        );

        log::info!(
            "Initialized default textures (white, normal, metallic-roughness, checkerboard)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_skip_counts_down() {
        let mut scheduler = Scheduler::default();
        scheduler.skip_initial_frames(2);
        let mut workspace = SceneWorkspace::new();
        let handle = workspace.open_scene("test".into(), Scene::new());

        let first = {
            let mut scene = workspace.scene_mut_by_handle(handle).unwrap();
            scheduler.begin_frame(handle, &mut scene)
        };
        assert!(!first.should_render());

        let second = {
            let mut scene = workspace.scene_mut_by_handle(handle).unwrap();
            scheduler.begin_frame(handle, &mut scene)
        };
        assert!(second.should_render());
    }

    #[test]
    fn update_stage_switches_active_scene() {
        let mut scheduler = Scheduler::default();
        let target = "second".to_string();
        scheduler.add_update_system(Box::new(move |ctx| {
            ctx.request_active_scene(target.clone());
        }));

        let mut workspace = SceneWorkspace::new();
        let first_id = "first".to_string();
        let second_id = "second".to_string();
        let first_handle = workspace.open_scene(first_id.clone(), Scene::new());
        workspace.open_scene(second_id.clone(), Scene::new());

        let result = scheduler
            .run_update_stage(&mut workspace, first_handle, 0.0, RuntimeMode::Editor)
            .expect("update stage should succeed");

        assert_eq!(workspace.active_scene_handle(), Some(result));
        assert_eq!(workspace.active_document_id(), Some(&second_id));
    }
}
