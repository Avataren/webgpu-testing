//! Inspector sections for skeletal animation: the Animator (the clip files a character
//! plays from and what plays now), the AnimationTree (states, blend spaces and
//! transitions, like Godot's AnimationTree) and "Make Character" for imported models.

use std::path::{Path, PathBuf};

use egui::{ComboBox, DragValue, Grid};
use hecs::Entity;
use wgpu_cube::scene::skeletal::{
    check_condition, AnimationTree, AnimationTreeState, AnimationTreeTransition, Animator,
    BlendPoint, BlendPoint2d, ClipInfo, Motion, RootMotion,
};
use wgpu_cube::SceneEntityComponentsSummary;

use super::InspectorAction;
use crate::theme::{self, icons};

#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;

/// What the animation sections show besides the components, worked out each frame for
/// the selected entity by the editor.
#[derive(Default, Clone, Debug)]
pub struct AnimationPanelStatus {
    /// The entity this describes.
    pub entity: Option<Entity>,
    /// The animator it drives: its own, or the model's below it.
    pub animator: Option<Entity>,
    /// The clips that animator can play.
    pub clips: Vec<ClipInfo>,
    /// Play mode: where the entity's AnimationTree is.
    pub live: Option<TreeLive>,
    /// Play mode: the clips layer 0 plays now, with their weights.
    pub playing: Vec<(String, f32)>,
    /// It is a model "Make Character" can turn into a character (meshes below, no
    /// CharacterController at or above it).
    pub can_make_character: bool,
    pub is_playing: bool,
}

/// An AnimationTree while playing.
#[derive(Default, Clone, Debug)]
pub struct TreeLive {
    pub state: Option<String>,
    pub previous: Option<String>,
    pub time: f32,
    pub progress: f32,
    pub parameters: Vec<(String, f32)>,
}

pub(super) fn show_animation_sections(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    status: &AnimationPanelStatus,
    content_root: Option<&Path>,
    actions: &mut Vec<InspectorAction>,
) {
    let empty = AnimationPanelStatus::default();
    let status = if status.entity == Some(entity) {
        status
    } else {
        &empty
    };
    if let Some(animator) = &components.animator {
        let header = animator_section(ui, entity, animator, status, content_root, actions);
        header.context_menu(|ui| {
            if remove_button(ui, "Animator") {
                actions.push(InspectorAction::SetAnimator {
                    entity,
                    animator: None,
                });
            }
        });
    }
    if let Some(tree) = &components.animation_tree {
        let header = tree_section(ui, entity, tree, status, actions);
        header.context_menu(|ui| {
            if remove_button(ui, "Animation Tree") {
                actions.push(InspectorAction::SetAnimationTree { entity, tree: None });
            }
        });
    }
    if status.can_make_character && !status.is_playing {
        make_character_box(ui, entity, actions);
    }
}

/// Entries for the "Add Component" menu.
pub(super) fn add_animation_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    ui.menu_button(format!("{}  Animation", icons::ANIMATOR), |ui| {
        if ui
            .add_enabled(components.animator.is_none(), egui::Button::new("Animator"))
            .on_hover_text("Plays clips on the bones below this entity")
            .clicked()
        {
            actions.push(InspectorAction::SetAnimator {
                entity,
                animator: Some(Box::default()),
            });
            ui.close();
        }
        if ui
            .add_enabled(
                components.animation_tree.is_none(),
                egui::Button::new("Animation Tree"),
            )
            .on_hover_text(
                "Picks the animation from parameters (speed, grounded, ...); generated from \
                 the clips of the animator below",
            )
            .clicked()
        {
            actions.push(InspectorAction::GenerateAnimationTree { entity });
            ui.close();
        }
    });
}

fn remove_button(ui: &mut egui::Ui, label: &str) -> bool {
    let text = egui::RichText::new(format!("{}  Remove {label}", icons::DELETE))
        .color(theme::colors::ERROR);
    let clicked = ui.button(text).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn make_character_box(ui: &mut egui::Ui, entity: Entity, actions: &mut Vec<InspectorAction>) {
    ui.add_space(4.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(egui::RichText::new(format!("{}  Character", icons::CHARACTER)).strong());
        let id = ui.id().with(("make_character_script", entity));
        let mut script = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(true);
        if ui
            .checkbox(&mut script, "Third-person controls")
            .on_hover_text(
                "Also write content/scripts/<name>.lua with the engine's third-person \
                 controls (keyboard and mouse, World of Warcraft style)",
            )
            .changed()
        {
            ui.data_mut(|data| data.insert_temp(id, script));
        }
        if ui
            .add(theme::primary_button(format!(
                "{}  Make Character",
                icons::CHARACTER
            )))
            .on_hover_text(
                "Make this model playable: it goes under a new entity with a Character \
                 Controller sized to it, an Animation Tree built from its clips, and the \
                 scene's camera follows it",
            )
            .clicked()
        {
            actions.push(InspectorAction::MakeCharacter { entity, script });
        }
    });
}

// ------------------------------------------------------------------------------------
// Animator
// ------------------------------------------------------------------------------------

fn animator_section(
    ui: &mut egui::Ui,
    entity: Entity,
    animator: &Animator,
    status: &AnimationPanelStatus,
    content_root: Option<&Path>,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::ANIMATOR, "Animator", |ui| {
        let mut edited = animator.clone();
        let mut changed = false;

        ui.label(theme::hint("Clip files").small());
        let mut remove = None;
        for (index, path) in animator.libraries.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui
                    .small_button(icons::CLOSE)
                    .on_hover_text("Stop using this file's clips")
                    .clicked()
                {
                    remove = Some(index);
                }
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                ui.label(name).on_hover_text(path.display().to_string());
            });
        }
        if let Some(index) = remove {
            edited.libraries.remove(index);
            changed = true;
        }
        add_clips_button(ui, entity, content_root, actions);
        ui.add_space(4.0);

        let clip_names: Vec<&str> = status.clips.iter().map(|c| c.name.as_str()).collect();
        Grid::new(("animator_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Default clip").on_hover_text(
                    "Plays when play starts; an Animation Tree, when there is one, decides \
                     instead",
                );
                let shown = edited
                    .default_clip
                    .clone()
                    .unwrap_or_else(|| "(none)".into());
                ComboBox::from_id_salt(("default_clip", entity))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut edited.default_clip, None, "(none)")
                            .changed();
                        for name in &clip_names {
                            changed |= ui
                                .selectable_value(
                                    &mut edited.default_clip,
                                    Some((*name).to_string()),
                                    *name,
                                )
                                .changed();
                        }
                    });
                ui.end_row();

                ui.label("Speed");
                changed |= ui
                    .add(
                        DragValue::new(&mut edited.speed)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    )
                    .changed();
                ui.end_row();

                ui.label("Root motion").on_hover_text(
                    "What happens to a clip's travel over the ground: keep it on the hips, \
                     remove it (walk on the spot) or move the entity by it",
                );
                ComboBox::from_id_salt(("root_motion", entity))
                    .selected_text(root_motion_label(edited.root_motion))
                    .show_ui(ui, |ui| {
                        for mode in [RootMotion::Keep, RootMotion::InPlace, RootMotion::Apply] {
                            changed |= ui
                                .selectable_value(
                                    &mut edited.root_motion,
                                    mode,
                                    root_motion_label(mode),
                                )
                                .changed();
                        }
                    });
                ui.end_row();
            });

        if status.is_playing && !status.playing.is_empty() {
            let playing: Vec<String> = status
                .playing
                .iter()
                .map(|(clip, weight)| format!("{clip} {:.0}%", weight * 100.0))
                .collect();
            ui.label(format!("Playing: {}", playing.join(", ")));
        }

        let title = format!("Clips ({})", status.clips.len());
        egui::CollapsingHeader::new(title)
            .id_salt(("animator_clips", entity))
            .default_open(status.clips.len() <= 12)
            .show(ui, |ui| {
                if status.clips.is_empty() {
                    ui.label(theme::hint("No clips yet: use Add clips..."));
                    return;
                }
                clips_table(ui, entity, &status.clips);
            });

        if changed {
            actions.push(InspectorAction::SetAnimator {
                entity,
                animator: Some(Box::new(edited)),
            });
        }
    })
}

fn root_motion_label(mode: RootMotion) -> &'static str {
    match mode {
        RootMotion::Keep => "Keep",
        RootMotion::InPlace => "In place",
        RootMotion::Apply => "Apply to entity",
    }
}

fn clips_table(ui: &mut egui::Ui, entity: Entity, clips: &[ClipInfo]) {
    Grid::new(("animator_clip_table", entity))
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.label(theme::hint("Clip").small());
            ui.label(theme::hint("Length").small());
            ui.label(theme::hint("Moves").small())
                .on_hover_text("How fast it travels over the ground");
            ui.label(theme::hint("Bones").small());
            ui.end_row();
            for clip in clips {
                ui.label(&clip.name)
                    .on_hover_text(format!("from {}", clip.library.display()));
                ui.label(format!("{:.2} s", clip.duration));
                if clip.root_speed > 0.05 {
                    ui.label(format!("{:.2} m/s", clip.root_speed));
                } else {
                    ui.label(theme::hint("in place"));
                }
                let bones = format!("{}/{}", clip.bound, clip.tracks);
                if clip.missing.is_empty() {
                    ui.label(bones);
                } else {
                    let shown: Vec<&str> =
                        clip.missing.iter().take(8).map(String::as_str).collect();
                    ui.label(egui::RichText::new(bones).color(theme::colors::WARNING))
                        .on_hover_text(format!(
                            "The skeleton lacks {} of its bones: {}",
                            clip.missing.len(),
                            shown.join(", ")
                        ));
                }
                ui.end_row();
            }
        });
}

/// "Add clips...": a glTF/GLB inside the project is used where it is; anything else
/// (an FBX, a file elsewhere) is imported into content/animations first.
fn add_clips_button(
    ui: &mut egui::Ui,
    entity: Entity,
    content_root: Option<&Path>,
    actions: &mut Vec<InspectorAction>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let project = content_root.and_then(Path::parent);
        let response = ui
            .add_enabled(
                project.is_some(),
                egui::Button::new(format!("{}  Add clips...", icons::ADD)),
            )
            .on_hover_text(
                "Animations from a glTF/GLB or FBX file (Mixamo: \"Without Skin\"); files \
                 from outside the project are imported into content/animations",
            );
        if response.clicked() {
            let Some(project) = project else {
                return;
            };
            let mut dialog = FileDialog::new()
                .add_filter("Animations", &["glb", "gltf", "fbx", "blend"])
                .set_directory(content_root.map_or(project.to_path_buf(), |content| {
                    let animations = content.join("animations");
                    if animations.is_dir() {
                        animations
                    } else {
                        content.to_path_buf()
                    }
                }));
            dialog = dialog.set_title("Add animation clips");
            for path in dialog.pick_files().unwrap_or_default() {
                actions.push(library_action(entity, project, path));
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (entity, content_root, actions);
        ui.add_enabled(false, egui::Button::new("Add clips..."));
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn library_action(entity: Entity, project: &Path, path: PathBuf) -> InspectorAction {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let inside = canonical(&path)
        .strip_prefix(canonical(project))
        .ok()
        .map(Path::to_path_buf);
    match inside {
        Some(relative) if extension == "glb" || extension == "gltf" => {
            InspectorAction::AddAnimationLibrary {
                entity,
                path: relative,
            }
        }
        _ => InspectorAction::ImportAnimations { entity, path },
    }
}

// ------------------------------------------------------------------------------------
// AnimationTree
// ------------------------------------------------------------------------------------

fn tree_section(
    ui: &mut egui::Ui,
    entity: Entity,
    tree: &AnimationTree,
    status: &AnimationPanelStatus,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::ANIMATION_TREE, "Animation Tree", |ui| {
        let mut edited = tree.clone();
        let mut changed = false;

        ui.horizontal(|ui| {
            if ui
                .button("Generate from clips")
                .on_hover_text(
                    "Replace the states and transitions with locomotion built from the \
                     animator's clips by name: idle, walk, run, strafes, back, jump, fall, \
                     land",
                )
                .clicked()
            {
                actions.push(InspectorAction::GenerateAnimationTree { entity });
            }
            if status.animator.is_none() {
                ui.label(
                    egui::RichText::new(format!("{} no Animator", icons::WARNING))
                        .color(theme::colors::WARNING),
                )
                .on_hover_text("No Animator on this entity, below it or above it");
            }
        });

        if let Some(live) = &status.live {
            live_view(ui, entity, live);
        }

        let state_names: Vec<String> = tree.states.iter().map(|s| s.name.clone()).collect();
        let clip_names: Vec<String> = status.clips.iter().map(|c| c.name.clone()).collect();

        Grid::new(("tree_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Start state");
                changed |= name_combo(
                    ui,
                    ("tree_start", entity),
                    &mut edited.start,
                    &state_names,
                    None,
                    None,
                );
                ui.end_row();
                ui.label("From character").on_hover_text(
                    "Read speed, forward_speed, side_speed, vertical_speed, grounded, \
                     air_time, landing_speed and the jumped/landed triggers from the \
                     Character Controller",
                );
                changed |= ui.checkbox(&mut edited.from_character, "").changed();
                ui.end_row();
            });

        let title = format!("States ({})", tree.states.len());
        egui::CollapsingHeader::new(title)
            .id_salt(("tree_states", entity))
            .default_open(true)
            .show(ui, |ui| {
                changed |= states_ui(ui, entity, &mut edited, &clip_names);
            });

        let title = format!("Transitions ({})", tree.transitions.len());
        egui::CollapsingHeader::new(title)
            .id_salt(("tree_transitions", entity))
            .default_open(false)
            .show(ui, |ui| {
                changed |= transitions_ui(ui, entity, &mut edited, &state_names);
            })
            .header_response
            .on_hover_text(
                "Checked in order; the first from the current state whose condition holds \
                 is taken. Conditions use parameters, numbers, < <= > >= == !=, && || ! \
                 (or and, or, not); empty is always.",
            );

        let title = format!("Parameters ({})", tree.parameters.len());
        egui::CollapsingHeader::new(title)
            .id_salt(("tree_parameters", entity))
            .default_open(false)
            .show(ui, |ui| {
                changed |= parameters_ui(ui, entity, &mut edited);
            })
            .header_response
            .on_hover_text(
                "Starting values of the parameters scripts set (animation_set); the \
                 character's own come from its controller.",
            );

        if changed {
            actions.push(InspectorAction::SetAnimationTree {
                entity,
                tree: Some(Box::new(edited)),
            });
        }
    })
}

fn live_view(ui: &mut egui::Ui, entity: Entity, live: &TreeLive) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let state = live.state.as_deref().unwrap_or("(not started)");
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(state)
                    .strong()
                    .color(theme::colors::SUCCESS),
            );
            if let Some(previous) = &live.previous {
                ui.label(theme::hint(format!("from {previous}")));
            }
            ui.label(theme::hint(format!(
                "{:.1} s · {:.0}%",
                live.time,
                live.progress * 100.0
            )));
        });
        if !live.parameters.is_empty() {
            Grid::new(("tree_live_parameters", entity))
                .num_columns(2)
                .show(ui, |ui| {
                    for (name, value) in &live.parameters {
                        ui.label(theme::hint(name).small());
                        ui.label(
                            egui::RichText::new(format!("{value:.2}"))
                                .small()
                                .monospace(),
                        );
                        ui.end_row();
                    }
                });
        }
    });
}

/// A combo box choosing one of `names`, with an optional first entry (`"*"`: any).
fn name_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    names: &[String],
    any: Option<&str>,
    width: Option<f32>,
) -> bool {
    let mut changed = false;
    let shown = if value.is_empty() {
        "(none)".to_string()
    } else if any == Some(value.as_str()) {
        "any state".to_string()
    } else {
        value.clone()
    };
    let missing = !value.is_empty() && any != Some(value.as_str()) && !names.contains(value);
    let text = if missing {
        egui::RichText::new(shown).color(theme::colors::ERROR)
    } else {
        egui::RichText::new(shown)
    };
    let mut combo = ComboBox::from_id_salt(id).selected_text(text);
    if let Some(width) = width {
        combo = combo.width(width);
    }
    combo.show_ui(ui, |ui| {
        if let Some(any) = any {
            changed |= ui
                .selectable_value(value, any.to_string(), "any state")
                .changed();
        }
        for name in names {
            changed |= ui.selectable_value(value, name.clone(), name).changed();
        }
    });
    changed
}

fn states_ui(
    ui: &mut egui::Ui,
    entity: Entity,
    tree: &mut AnimationTree,
    clips: &[String],
) -> bool {
    let mut changed = false;
    let mut remove = None;
    let old_names: Vec<String> = tree.states.iter().map(|s| s.name.clone()).collect();
    for (index, state) in tree.states.iter_mut().enumerate() {
        let title = format!("{}  ({})", state.name, motion_label(&state.motion));
        egui::CollapsingHeader::new(title)
            .id_salt(("tree_state", entity, index))
            .default_open(false)
            .show(ui, |ui| {
                changed |= state_ui(ui, entity, index, state, clips, &mut remove);
            });
    }
    // A renamed state keeps its transitions and start.
    for (old, state) in old_names.iter().zip(&tree.states) {
        if *old != state.name {
            for transition in &mut tree.transitions {
                if transition.from == *old {
                    transition.from = state.name.clone();
                }
                if transition.to == *old {
                    transition.to = state.name.clone();
                }
            }
            if tree.start == *old {
                tree.start = state.name.clone();
            }
        }
    }
    if let Some(index) = remove {
        let name = tree.states.remove(index).name;
        tree.transitions.retain(|t| t.from != name && t.to != name);
        if tree.start == name {
            tree.start = tree
                .states
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_default();
        }
        changed = true;
    }
    if ui
        .button(format!("{}  State", icons::ADD))
        .on_hover_text("A state playing one clip")
        .clicked()
    {
        let mut name = "State".to_string();
        let mut number = 2;
        while tree.states.iter().any(|s| s.name == name) {
            name = format!("State {number}");
            number += 1;
        }
        if tree.start.is_empty() {
            tree.start = name.clone();
        }
        tree.states.push(AnimationTreeState {
            name,
            motion: Motion::Clip {
                clip: clips.first().cloned().unwrap_or_default(),
                speed: 1.0,
            },
            looping: true,
            speed: 1.0,
        });
        changed = true;
    }
    changed
}

fn motion_label(motion: &Motion) -> &'static str {
    match motion {
        Motion::Clip { .. } => "clip",
        Motion::Blend1d { .. } => "blend 1D",
        Motion::Blend2d { .. } => "blend 2D",
    }
}

fn state_ui(
    ui: &mut egui::Ui,
    entity: Entity,
    index: usize,
    state: &mut AnimationTreeState,
    clips: &[String],
    remove: &mut Option<usize>,
) -> bool {
    let mut changed = false;
    Grid::new(("tree_state_grid", entity, index))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.horizontal(|ui| {
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut state.name).desired_width(110.0))
                    .changed();
                if ui
                    .small_button(icons::DELETE)
                    .on_hover_text("Remove this state and its transitions")
                    .clicked()
                {
                    *remove = Some(index);
                }
            });
            ui.end_row();

            ui.label("Motion");
            let mut kind = motion_label(&state.motion);
            let before = kind;
            ComboBox::from_id_salt(("tree_motion_kind", entity, index))
                .selected_text(kind)
                .show_ui(ui, |ui| {
                    for option in ["clip", "blend 1D", "blend 2D"] {
                        ui.selectable_value(&mut kind, option, option);
                    }
                });
            if kind != before {
                state.motion = convert_motion(&state.motion, kind);
                changed = true;
            }
            ui.end_row();

            ui.label("Loop");
            changed |= ui.checkbox(&mut state.looping, "").changed();
            ui.end_row();

            ui.label("Speed");
            changed |= ui
                .add(
                    DragValue::new(&mut state.speed)
                        .speed(0.01)
                        .range(0.0..=10.0),
                )
                .changed();
            ui.end_row();
        });
    changed |= motion_ui(ui, entity, index, &mut state.motion, clips);
    changed
}

/// The same clips in another kind of motion.
fn convert_motion(motion: &Motion, kind: &str) -> Motion {
    let clips: Vec<String> = motion.clips().into_iter().map(str::to_owned).collect();
    let first = clips.first().cloned().unwrap_or_default();
    match kind {
        "blend 1D" => Motion::Blend1d {
            parameter: "speed".into(),
            points: clips
                .iter()
                .enumerate()
                .map(|(i, clip)| BlendPoint {
                    at: i as f32 * 2.0,
                    clip: clip.clone(),
                    speed: 1.0,
                })
                .collect(),
            scale_speed: false,
        },
        "blend 2D" => Motion::Blend2d {
            x: "side_speed".into(),
            y: "forward_speed".into(),
            points: clips
                .iter()
                .enumerate()
                .map(|(i, clip)| BlendPoint2d {
                    at: [0.0, i as f32 * 2.0],
                    clip: clip.clone(),
                    speed: 1.0,
                })
                .collect(),
            scale_speed: false,
        },
        _ => Motion::Clip {
            clip: first,
            speed: 1.0,
        },
    }
}

fn motion_ui(
    ui: &mut egui::Ui,
    entity: Entity,
    index: usize,
    motion: &mut Motion,
    clips: &[String],
) -> bool {
    let mut changed = false;
    let id = |part: &str| (part.to_string(), entity, index);
    match motion {
        Motion::Clip { clip, speed } => {
            ui.horizontal(|ui| {
                ui.label("Clip");
                changed |= clip_and_speed(ui, id("clip"), clip, speed, clips);
            });
        }
        Motion::Blend1d {
            parameter,
            points,
            scale_speed,
        } => {
            ui.horizontal(|ui| {
                ui.label("Blend by");
                changed |= ui
                    .add(egui::TextEdit::singleline(parameter).desired_width(f32::INFINITY))
                    .on_hover_text("The parameter placing the blend between its points")
                    .changed();
            });
            ui.label(theme::hint("At · clip (its menu sets the speed)").small());
            let mut remove = None;
            for (i, point) in points.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(DragValue::new(&mut point.at).speed(0.02).max_decimals(2))
                        .changed();
                    changed |= clip_and_speed(
                        ui,
                        (id("blend1d"), i),
                        &mut point.clip,
                        &mut point.speed,
                        clips,
                    );
                    if ui.small_button(icons::CLOSE).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                points.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Point", icons::ADD)).clicked() {
                    let at = points.last().map_or(0.0, |p| p.at + 1.0);
                    points.push(BlendPoint {
                        at,
                        clip: clips.first().cloned().unwrap_or_default(),
                        speed: 1.0,
                    });
                    changed = true;
                }
                changed |= ui
                    .checkbox(scale_speed, "Match speed")
                    .on_hover_text(
                        "Past the last point, play faster (and slower before the first) so \
                         the feet keep up with the parameter",
                    )
                    .changed();
            });
        }
        Motion::Blend2d {
            x,
            y,
            points,
            scale_speed,
        } => {
            Grid::new(id("blend2d_axes")).num_columns(2).show(ui, |ui| {
                ui.label("X");
                changed |= ui
                    .add(egui::TextEdit::singleline(x).desired_width(f32::INFINITY))
                    .on_hover_text("The parameter along X (side_speed: + is right)")
                    .changed();
                ui.end_row();
                ui.label("Y");
                changed |= ui
                    .add(egui::TextEdit::singleline(y).desired_width(f32::INFINITY))
                    .on_hover_text("The parameter along Y (forward_speed: + is ahead)")
                    .changed();
                ui.end_row();
            });
            ui.label(theme::hint("X · Y · clip (its menu sets the speed)").small());
            let mut remove = None;
            for (i, point) in points.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    for value in &mut point.at {
                        changed |= ui
                            .add(DragValue::new(value).speed(0.02).max_decimals(2))
                            .changed();
                    }
                    changed |= clip_and_speed(
                        ui,
                        (id("blend2d"), i),
                        &mut point.clip,
                        &mut point.speed,
                        clips,
                    );
                    if ui.small_button(icons::CLOSE).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                points.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Point", icons::ADD)).clicked() {
                    points.push(BlendPoint2d {
                        at: [0.0, 0.0],
                        clip: clips.first().cloned().unwrap_or_default(),
                        speed: 1.0,
                    });
                    changed = true;
                }
                changed |= ui
                    .checkbox(scale_speed, "Match speed")
                    .on_hover_text("Beyond the outermost points, play faster so the feet keep up")
                    .changed();
            });
        }
    }
    changed
}

/// A clip box filling the row (leaving room for a remove button); its menu also sets the
/// clip's speed, which the box shows when it is not 1.
fn clip_and_speed(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    clip: &mut String,
    speed: &mut f32,
    clips: &[String],
) -> bool {
    let mut changed = false;
    // The box adds its arrow and padding to `width`; leave room for a remove button.
    let reserved = ui.spacing().interact_size.y + 2.0 * ui.spacing().item_spacing.x + 40.0;
    let width = (ui.available_width() - reserved).clamp(60.0, 220.0);
    let missing = !clips.is_empty() && !clips.contains(clip);
    let mut label = clip.clone();
    if (*speed - 1.0).abs() > 1e-4 {
        let speed = format!("{speed:.2}");
        label.push_str(&format!(
            " ×{}",
            speed.trim_end_matches('0').trim_end_matches('.')
        ));
    }
    let text = if missing {
        egui::RichText::new(label).color(theme::colors::ERROR)
    } else {
        egui::RichText::new(label)
    };
    let response = ComboBox::from_id_salt(id)
        .selected_text(text)
        .width(width)
        .show_ui(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Speed");
                changed |= ui
                    .add(
                        DragValue::new(speed)
                            .speed(0.01)
                            .max_decimals(2)
                            .range(-10.0..=10.0),
                    )
                    .on_hover_text("Negative plays it backwards (a walk as a backpedal)")
                    .changed();
            });
            ui.separator();
            for name in clips {
                changed |= ui.selectable_value(clip, name.clone(), name).changed();
            }
        });
    let hover = if missing {
        "The animator has no clip by this name"
    } else {
        "The clip, and the speed it plays at"
    };
    response.response.on_hover_text(hover);
    changed
}

fn transitions_ui(
    ui: &mut egui::Ui,
    entity: Entity,
    tree: &mut AnimationTree,
    states: &[String],
) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (index, transition) in tree.transitions.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // Two boxes (each adding its arrow and padding), "to" and a remove
                // button share the row.
                let spacing = ui.spacing().item_spacing.x;
                let width =
                    ((ui.available_width() - 50.0 - 3.0 * spacing) / 2.0 - 30.0).clamp(50.0, 160.0);
                changed |= name_combo(
                    ui,
                    ("transition_from", entity, index),
                    &mut transition.from,
                    states,
                    Some("*"),
                    Some(width),
                );
                ui.label("to");
                changed |= name_combo(
                    ui,
                    ("transition_to", entity, index),
                    &mut transition.to,
                    states,
                    None,
                    Some(width),
                );
                if ui
                    .small_button(icons::DELETE)
                    .on_hover_text("Remove this transition")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
            ui.horizontal(|ui| {
                ui.label("when");
                let error = check_condition(&transition.when).err();
                let mut edit = egui::TextEdit::singleline(&mut transition.when)
                    .hint_text("always")
                    .desired_width(ui.available_width() - 4.0);
                if error.is_some() {
                    edit = edit.text_color(theme::colors::ERROR);
                }
                let response = ui.add(edit);
                changed |= response.changed();
                if let Some(error) = error {
                    response.on_hover_text(error);
                }
            });
            ui.horizontal(|ui| {
                ui.label("fade");
                changed |= ui
                    .add(
                        DragValue::new(&mut transition.fade)
                            .speed(0.01)
                            .range(0.0..=5.0)
                            .suffix(" s"),
                    )
                    .changed();
                let mut has_after = transition.after.is_some();
                if ui
                    .checkbox(&mut has_after, "after")
                    .on_hover_text(
                        "Only once the state's motion has played this far (0..1; above 1 \
                         for later loops)",
                    )
                    .changed()
                {
                    transition.after = has_after.then_some(0.9);
                    changed = true;
                }
                if let Some(after) = &mut transition.after {
                    changed |= ui
                        .add(DragValue::new(after).speed(0.01).range(0.0..=10.0))
                        .changed();
                }
            });
        });
    }
    if let Some(index) = remove {
        tree.transitions.remove(index);
        changed = true;
    }
    if ui.button(format!("{}  Transition", icons::ADD)).clicked() {
        tree.transitions.push(AnimationTreeTransition {
            from: states.first().cloned().unwrap_or_else(|| "*".into()),
            to: states
                .get(1)
                .or(states.first())
                .cloned()
                .unwrap_or_default(),
            when: String::new(),
            fade: 0.2,
            after: None,
        });
        changed = true;
    }
    changed
}

fn parameters_ui(ui: &mut egui::Ui, entity: Entity, tree: &mut AnimationTree) -> bool {
    let mut changed = false;
    let mut rename = None;
    let mut remove = None;
    Grid::new(("tree_parameters_grid", entity))
        .num_columns(3)
        .show(ui, |ui| {
            for (name, value) in tree.parameters.iter_mut() {
                let mut edited = name.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut edited).desired_width(110.0))
                    .changed()
                {
                    rename = Some((name.clone(), edited));
                }
                changed |= ui.add(DragValue::new(value).speed(0.01)).changed();
                if ui.small_button(icons::CLOSE).clicked() {
                    remove = Some(name.clone());
                }
                ui.end_row();
            }
        });
    if let Some((old, new)) = rename {
        if !new.is_empty() && !tree.parameters.contains_key(&new) {
            if let Some(value) = tree.parameters.remove(&old) {
                tree.parameters.insert(new, value);
                changed = true;
            }
        }
    }
    if let Some(name) = remove {
        tree.parameters.remove(&name);
        changed = true;
    }
    if ui.button(format!("{}  Parameter", icons::ADD)).clicked() {
        let mut name = "parameter".to_string();
        let mut number = 2;
        while tree.parameters.contains_key(&name) {
            name = format!("parameter_{number}");
            number += 1;
        }
        tree.parameters.insert(name, 0.0);
        changed = true;
    }
    changed
}
