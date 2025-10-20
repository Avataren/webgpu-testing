use egui::{Color32, Stroke, StrokeKind};
use egui_tiles::{Behavior, Container, Tile, TileId, Tree, UiResponse};

use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::{LogWindow, SceneHierarchyWindow};

use crate::inspector::{show_entity_inspector, InspectorAction};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditorPane {
    SceneHierarchy,
    SceneViewport,
    GameViewport,
    Inspector,
    AssetBrowser,
    Log,
}

#[derive(Default)]
pub struct ViewportState {
    region: Option<RenderRegion>,
    rect: Option<egui::Rect>,
}

impl ViewportState {
    pub fn clear(&mut self) {
        self.region = None;
        self.rect = None;
    }

    pub fn set(&mut self, rect: egui::Rect, region: RenderRegion) {
        self.rect = Some(rect);
        self.region = Some(region);
    }

    pub fn region(&self) -> Option<RenderRegion> {
        self.region
    }

    pub fn rect(&self) -> Option<egui::Rect> {
        self.rect
    }
}

pub struct EditorBehavior<'a> {
    pub scene_viewport: &'a mut ViewportState,
    pub game_viewport: &'a mut ViewportState,
    pub scene_hierarchy: &'a mut SceneHierarchyWindow,
    pub log_window: &'a mut LogWindow,
    pub is_playing: bool,
    pub inspector_actions: &'a mut Vec<InspectorAction>,
}

impl Behavior<EditorPane> for EditorBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut EditorPane,
    ) -> UiResponse {
        match pane {
            EditorPane::SceneHierarchy => {
                ui.set_min_width(220.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        self.scene_hierarchy.ui(ui);
                    });
            }
            EditorPane::SceneViewport => {
                show_viewport(ui, self.scene_viewport, "Scene View", true, None);
            }
            EditorPane::GameViewport => {
                let placeholder = if self.is_playing {
                    None
                } else {
                    Some("Press Play to run the scene")
                };
                show_viewport(
                    ui,
                    self.game_viewport,
                    "Game View",
                    self.is_playing,
                    placeholder,
                );
            }
            EditorPane::Inspector => {
                ui.set_min_width(240.0);
                ui.heading("Inspector");
                ui.separator();

                if let Some(selection) = self.scene_hierarchy.selected_entity_data() {
                    let actions = show_entity_inspector(ui, &selection);
                    self.inspector_actions.extend(actions);
                } else {
                    ui.label("Select an entity to view its components.");
                }
            }
            EditorPane::AssetBrowser => {
                ui.heading("Asset Browser");
                ui.label("Browse project assets (coming soon).");
            }
            EditorPane::Log => {
                ui.heading("Log");
                ui.add_space(4.0);
                self.log_window.ui(ui);
            }
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &EditorPane) -> egui::WidgetText {
        match pane {
            EditorPane::SceneHierarchy => "Hierarchy".into(),
            EditorPane::SceneViewport => "Scene".into(),
            EditorPane::GameViewport => {
                if self.is_playing {
                    "Game ▶".into()
                } else {
                    "Game".into()
                }
            }
            EditorPane::Inspector => "Inspector".into(),
            EditorPane::AssetBrowser => "Assets".into(),
            EditorPane::Log => "Log".into(),
        }
    }
}

pub fn create_editor_layout() -> Tree<EditorPane> {
    let mut tiles = egui_tiles::Tiles::default();

    let hierarchy = tiles.insert_pane(EditorPane::SceneHierarchy);
    let scene_view = tiles.insert_pane(EditorPane::SceneViewport);
    let game_view = tiles.insert_pane(EditorPane::GameViewport);
    let inspector = tiles.insert_pane(EditorPane::Inspector);
    let asset_browser = tiles.insert_pane(EditorPane::AssetBrowser);
    let log_view = tiles.insert_pane(EditorPane::Log);

    let hierarchy_tab = tiles.insert_tab_tile(vec![hierarchy]);
    let viewport_tab = tiles.insert_tab_tile(vec![scene_view, game_view]);
    let inspector_tab = tiles.insert_tab_tile(vec![inspector]);
    let asset_tab = tiles.insert_tab_tile(vec![asset_browser]);
    let log_tab = tiles.insert_tab_tile(vec![log_view]);

    let horizontal = tiles.insert_horizontal_tile(vec![hierarchy_tab, viewport_tab, inspector_tab]);
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(horizontal) {
        linear.shares.set_share(hierarchy_tab, 0.22);
        linear.shares.set_share(viewport_tab, 0.58);
        linear.shares.set_share(inspector_tab, 0.2);
    }

    let bottom_split = tiles.insert_horizontal_tile(vec![asset_tab, log_tab]);
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(bottom_split) {
        linear.shares.set_share(asset_tab, 0.5);
        linear.shares.set_share(log_tab, 0.5);
    }

    let root = tiles.insert_vertical_tile(vec![horizontal, bottom_split]);
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(root) {
        linear.shares.set_share(horizontal, 0.78);
        linear.shares.set_share(bottom_split, 0.22);
    }

    Tree::new("editor_dock", root, tiles)
}

fn show_viewport(
    ui: &mut egui::Ui,
    viewport: &mut ViewportState,
    label: &str,
    active: bool,
    placeholder: Option<&str>,
) {
    let desired = ui.available_size();
    let desired = egui::vec2(desired.x.max(1.0), desired.y.max(1.0));
    let (rect, _response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());

    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        viewport.clear();
        return;
    }

    let painter = ui.painter_at(rect);

    let Some(region) = compute_viewport_region(ui.ctx(), rect) else {
        viewport.clear();
        painter.rect_filled(rect, 0.0, ui.visuals().panel_fill);
        return;
    };

    viewport.set(rect, region);

    if !active {
        painter.rect_filled(rect, 0.0, ui.visuals().panel_fill);

        if let Some(text) = placeholder {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::TextStyle::Body.resolve(ui.style()),
                Color32::from_gray(140),
            );
        }

        return;
    }

    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Outside,
    );
    painter.text(
        rect.center_top() + egui::vec2(0.0, 8.0),
        egui::Align2::CENTER_TOP,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        Color32::from_gray(180),
    );
}

pub fn compute_viewport_region(ctx: &egui::Context, rect: egui::Rect) -> Option<RenderRegion> {
    let pixels_per_point = ctx.pixels_per_point();
    let screen = ctx.viewport_rect();
    let max_width = (screen.width() * pixels_per_point).round().max(0.0) as u32;
    let max_height = (screen.height() * pixels_per_point).round().max(0.0) as u32;

    let min_x = (rect.min.x * pixels_per_point).floor().max(0.0);
    let min_y = (rect.min.y * pixels_per_point).floor().max(0.0);
    let width = (rect.width() * pixels_per_point).round().max(0.0);
    let height = (rect.height() * pixels_per_point).round().max(0.0);

    let region = RenderRegion::new(min_x as u32, min_y as u32, width as u32, height as u32)?;
    region.clamp(max_width, max_height)
}

pub fn show_fullscreen_viewport(ui: &mut egui::Ui, viewport: &mut ViewportState) {
    let desired = ui.available_size();
    let desired = egui::vec2(desired.x.max(1.0), desired.y.max(1.0));
    let (rect, _response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());

    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        viewport.clear();
        return;
    }

    if let Some(region) = compute_viewport_region(ui.ctx(), rect) {
        viewport.set(rect, region);
    } else {
        viewport.clear();
    }
}
