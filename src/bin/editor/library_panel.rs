//! The Library pane: browse the asset library (characters, animations, models shared by
//! every project; see `wgpu_cube::library`) and take assets into the project.

use std::collections::HashMap;

use egui::{Color32, RichText, Sense, Vec2};
use hecs::Entity;
use wgpu_cube::library::{Library, LibraryAsset, LibraryQuery};

use crate::theme::{self, icons};

/// What the pane asks the editor to do.
#[derive(Debug, Clone)]
pub enum LibraryAction {
    /// Take the asset into the project: an animation into `content/animations` (and onto
    /// `attach`'s animator), a character or model imported into the scene.
    Add { id: String, attach: Option<Entity> },
}

const CARD: f32 = 90.0;
const THUMB: f32 = 82.0;
/// Thumbnails decoded per frame, so scrolling a large library does not stall.
const THUMBS_PER_FRAME: usize = 6;

#[derive(Default)]
pub struct LibraryPanelState {
    library: Option<Library>,
    error: Option<String>,
    query: String,
    kind: Option<&'static str>,
    skeleton: Option<String>,
    selected: Option<String>,
    thumbnails: HashMap<String, Option<egui::TextureHandle>>,
}

impl LibraryPanelState {
    /// The library as last read (read on first use).
    pub fn library(&mut self) -> Option<&Library> {
        if self.library.is_none() && self.error.is_none() {
            self.reload();
        }
        self.library.as_ref()
    }

    pub fn reload(&mut self) {
        match Library::open_default() {
            Ok(library) => {
                self.library = Some(library);
                self.error = None;
            }
            Err(err) => {
                self.library = None;
                self.error = Some(err);
            }
        }
        self.thumbnails.clear();
    }

    fn thumbnail(
        &mut self,
        ctx: &egui::Context,
        asset: &LibraryAsset,
        budget: &mut usize,
    ) -> Option<egui::TextureHandle> {
        if let Some(cached) = self.thumbnails.get(&asset.id) {
            return cached.clone();
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let path = self.library.as_ref()?.thumbnail_path(asset);
        let texture = path.and_then(|path| image::open(path).ok()).map(|image| {
            let image = image.to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            ctx.load_texture(
                format!("library:{}", asset.id),
                pixels,
                egui::TextureOptions::LINEAR,
            )
        });
        self.thumbnails.insert(asset.id.clone(), texture.clone());
        texture
    }

    /// Show the pane. `target` is the selected entity (and its name), which animations
    /// can be added to.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: Option<(Entity, String)>,
        has_project: bool,
    ) -> Vec<LibraryAction> {
        let mut actions = Vec::new();
        if self.library.is_none() && self.error.is_none() {
            self.reload();
        }
        self.toolbar(ui);
        if let Some(error) = &self.error {
            theme::empty_state(ui, "No library", error);
            return actions;
        }
        let Some(library) = self.library.clone() else {
            return actions;
        };
        if library.assets.is_empty() {
            theme::empty_state(
                ui,
                "The library is empty",
                &format!(
                    "Fill {} with tools/library/library.py (fetch-pack, fetch-mixamo, ingest).",
                    library.root.display()
                ),
            );
            return actions;
        }
        let query = LibraryQuery {
            text: self.query.clone(),
            kind: self.kind.map(str::to_owned),
            tags: Vec::new(),
            skeleton: self.skeleton.clone(),
        };
        let found: Vec<LibraryAsset> = library.search(&query).into_iter().cloned().collect();

        let details_width = 230.0;
        ui.horizontal_top(|ui| {
            let grid_width = (ui.available_width() - details_width - 8.0).max(CARD);
            ui.vertical(|ui| {
                ui.set_width(grid_width);
                ui.label(
                    theme::hint(format!("{} of {}", found.len(), library.assets.len())).small(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("library_grid")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut budget = THUMBS_PER_FRAME;
                            for asset in &found {
                                let response = self.card(ui, asset, &mut budget);
                                if response.clicked() {
                                    self.selected = Some(asset.id.clone());
                                }
                                if response.double_clicked() && has_project {
                                    actions.push(LibraryAction::Add {
                                        id: asset.id.clone(),
                                        attach: (asset.kind == "animation")
                                            .then(|| target.as_ref().map(|(e, _)| *e))
                                            .flatten(),
                                    });
                                }
                            }
                        });
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(details_width - 12.0);
                let selected = self
                    .selected
                    .as_ref()
                    .and_then(|id| library.get(id))
                    .cloned();
                match selected {
                    Some(asset) => details(ui, &asset, target.as_ref(), has_project, &mut actions),
                    None => {
                        ui.label(theme::hint(
                            "Pick an asset. Double-click adds it: an animation to the \
                             selected character, a character or model to the scene.",
                        ));
                    }
                }
            });
        });
        actions
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(format!("{}  Search the library", icons::SEARCH))
                    .desired_width(200.0),
            );
            for (label, kind) in [
                ("All", None),
                ("Characters", Some("character")),
                ("Animations", Some("animation")),
                ("Models", Some("model")),
            ] {
                if ui.selectable_label(self.kind == kind, label).clicked() {
                    self.kind = kind;
                }
            }
            let skeletons: Vec<String> = self
                .library
                .as_ref()
                .map(|library| {
                    let mut names: Vec<String> = library
                        .assets
                        .iter()
                        .filter_map(|asset| asset.skeleton.clone())
                        .collect();
                    names.sort();
                    names.dedup();
                    names
                })
                .unwrap_or_default();
            let shown = self.skeleton.clone().unwrap_or_else(|| "any rig".into());
            egui::ComboBox::from_id_salt("library_skeleton")
                .selected_text(shown)
                .width(90.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.skeleton, None, "any rig");
                    for name in skeletons {
                        ui.selectable_value(&mut self.skeleton, Some(name.clone()), name);
                    }
                })
                .response
                .on_hover_text(
                    "The bone naming. Clips work on any humanoid rig (they bind by role); \
                     this narrows the list",
                );
            if ui
                .button(icons::REVERT)
                .on_hover_text("Read the library again (after fetching or ingesting)")
                .clicked()
            {
                self.reload();
            }
        });
    }

    fn card(
        &mut self,
        ui: &mut egui::Ui,
        asset: &LibraryAsset,
        budget: &mut usize,
    ) -> egui::Response {
        let selected = self.selected.as_deref() == Some(asset.id.as_str());
        let texture = self.thumbnail(ui.ctx(), asset, budget);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(CARD, CARD + 30.0), Sense::click());
        let painter = ui.painter_at(rect);
        let fill = if selected {
            theme::colors::ACCENT_DIM
        } else if response.hovered() {
            theme::colors::WIDGET_HOVER
        } else {
            theme::colors::WIDGET
        };
        painter.rect_filled(rect, 4.0, fill);
        let image_rect = egui::Rect::from_min_size(
            rect.min + Vec2::splat((CARD - THUMB) / 2.0),
            Vec2::splat(THUMB),
        );
        match texture {
            Some(texture) => {
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                let icon = match asset.kind.as_str() {
                    "character" => icons::CHARACTER,
                    "animation" => icons::ANIMATOR,
                    _ => icons::MESH,
                };
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon,
                    egui::FontId::proportional(28.0),
                    theme::colors::TEXT_WEAK,
                );
            }
        }
        let name = elide(&asset.name, 15);
        painter.text(
            egui::pos2(rect.center().x, image_rect.bottom() + 9.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(12.0),
            theme::colors::TEXT,
        );
        painter.text(
            egui::pos2(rect.center().x, image_rect.bottom() + 23.0),
            egui::Align2::CENTER_CENTER,
            subtitle(asset),
            egui::FontId::proportional(10.0),
            theme::colors::TEXT_WEAK,
        );
        response.on_hover_text(format!("{}\n{}", asset.name, asset.id))
    }
}

fn subtitle(asset: &LibraryAsset) -> String {
    match asset.clips.first() {
        Some(clip) if clip.in_place => format!("{:.1} s", clip.duration),
        Some(clip) => format!("{:.1} s · {:.1} m/s", clip.duration, clip.root_speed),
        None => asset
            .height
            .map(|height| format!("{} · {height:.2} m", asset.kind))
            .unwrap_or_else(|| asset.kind.clone()),
    }
}

fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}

fn details(
    ui: &mut egui::Ui,
    asset: &LibraryAsset,
    target: Option<&(Entity, String)>,
    has_project: bool,
    actions: &mut Vec<LibraryAction>,
) {
    ui.label(
        RichText::new(&asset.name)
            .strong()
            .color(theme::colors::TEXT_STRONG),
    );
    ui.label(theme::hint(format!(
        "{}{}",
        asset.kind,
        asset
            .skeleton
            .as_ref()
            .map(|s| format!(" · {s} rig"))
            .unwrap_or_default()
    )));
    if let Some(clip) = asset.clips.first() {
        ui.label(format!(
            "{:.2} s{}",
            clip.duration,
            if clip.in_place {
                ", in place".to_string()
            } else {
                format!(", travels {:.2} m/s", clip.root_speed)
            }
        ));
    }
    if let Some(height) = asset.height {
        let mut line = format!("{height:.2} m tall");
        if let Some(triangles) = asset.triangles {
            line.push_str(&format!(", {triangles} triangles"));
        }
        ui.label(line);
    }
    if !asset.tags.is_empty() {
        ui.label(theme::hint(asset.tags.join(", ")).small());
    }
    ui.add_space(4.0);
    let mut source = asset.source.provider.clone();
    if let Some(pack) = &asset.source.pack {
        source.push_str(&format!(" · {pack}"));
    }
    ui.label(theme::hint(source).small());
    if !asset.source.license.is_empty() {
        ui.label(theme::hint(&asset.source.license).small())
            .on_hover_text("The asset's licence; the project copies the file, not the library");
    }
    ui.add_space(6.0);
    if !has_project {
        ui.label(theme::hint("Open a project to add assets."));
        return;
    }
    if asset.kind == "animation" {
        if let Some((entity, name)) = target {
            if ui
                .add(theme::primary_button(format!(
                    "{}  Add to {name}",
                    icons::ADD
                )))
                .on_hover_text(
                    "Copy into content/animations and add it to this character's Animator",
                )
                .clicked()
            {
                actions.push(LibraryAction::Add {
                    id: asset.id.clone(),
                    attach: Some(*entity),
                });
            }
        }
        if ui
            .button(format!("{}  Add to project", icons::ADD))
            .on_hover_text("Copy into content/animations")
            .clicked()
        {
            actions.push(LibraryAction::Add {
                id: asset.id.clone(),
                attach: None,
            });
        }
    } else if ui
        .add(theme::primary_button(format!(
            "{}  Add to scene",
            icons::ADD
        )))
        .on_hover_text("Import it into the project and the open scene")
        .clicked()
    {
        actions.push(LibraryAction::Add {
            id: asset.id.clone(),
            attach: None,
        });
    }
}
