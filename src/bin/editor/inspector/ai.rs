//! The AI Agent section: the brain an entity runs and how it perceives, and in play
//! mode what it is thinking (its state, action, blackboard, tree or machine, senses).

use egui::{ComboBox, Grid};
use hecs::Entity;
use serde_json::Value;
use wgpu_cube::ai::{AiAgent, AiDebugDraw, LineOfSight};
use wgpu_cube::SceneEntityComponentsSummary;

use super::{float_drag_value, InspectorAction};
use crate::theme::{self, icons};

/// What the AI section asks of the scene (AiPanelSystem carries it out).
#[derive(Clone, Debug, PartialEq)]
pub enum AiRequest {
    SetDebug(AiDebugDraw),
    Restart(Entity),
    Pause(Entity, bool),
}

/// The AI state the inspector shows, refreshed every frame.
#[derive(Clone, Debug, Default)]
pub struct AiPanelStatus {
    pub playing: bool,
    pub debug: AiDebugDraw,
    /// The brains defined (in play: those the scripts defined).
    pub brains: Vec<(String, &'static str)>,
    /// The selected agent's mind (`ai.inspect`), while playing.
    pub selected: Option<Value>,
}

fn set(actions: &mut Vec<InspectorAction>, entity: Entity, value: Value) {
    actions.push(InspectorAction::SetPhysics {
        entity,
        component: "ai_agent",
        value,
    });
}

fn comma_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

const RUNNING: egui::Color32 = egui::Color32::from_rgb(240, 200, 90);
const SUCCEEDED: egui::Color32 = egui::Color32::from_rgb(120, 210, 130);
const FAILED: egui::Color32 = egui::Color32::from_rgb(230, 110, 100);

fn state_color(name: &str) -> egui::Color32 {
    let c = wgpu_cube::ai::state_color(name);
    egui::Color32::from_rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

pub(super) fn show_ai_section(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    status: &AiPanelStatus,
    actions: &mut Vec<InspectorAction>,
) {
    let Some(agent) = &components.ai_agent else {
        return;
    };
    let header = theme::section(ui, icons::AI, "AI Agent", |ui| {
        let mut edited = agent.clone();
        let mut changed = false;
        Grid::new(("ai_agent_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Brain");
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut edited.brain).desired_width(150.0))
                        .on_hover_text(
                            "A brain a script defines (ai_tree / ai_machine), or a JSON file in the project (content/ai/guard.json)",
                        )
                        .changed();
                    if !status.brains.is_empty() {
                        ComboBox::from_id_salt(("ai_brains", entity))
                            .selected_text("")
                            .width(20.0)
                            .show_ui(ui, |ui| {
                                for (name, kind) in &status.brains {
                                    if ui
                                        .selectable_label(edited.brain == *name, format!("{name} ({kind})"))
                                        .clicked()
                                    {
                                        edited.brain = name.clone();
                                        changed = true;
                                    }
                                }
                            });
                    }
                });
                ui.end_row();
                ui.label("Enabled");
                changed |= ui.checkbox(&mut edited.enabled, "").changed();
                ui.end_row();
                changed |= float_drag_value(
                    ui,
                    "Thinks per second",
                    &mut edited.tick_rate,
                    0.5,
                    Some(0.0..=120.0),
                );
                ui.label("Tags");
                let mut tags = edited.tags.join(", ");
                if ui
                    .text_edit_singleline(&mut tags)
                    .on_hover_text("What others perceive it as, comma separated (guard, child)")
                    .changed()
                {
                    edited.tags = comma_list(&tags);
                    changed = true;
                }
                ui.end_row();
                ui.label("Group");
                changed |= ui
                    .text_edit_singleline(&mut edited.group)
                    .on_hover_text("Agents of a group share the group. blackboard and group events")
                    .changed();
                ui.end_row();
                changed |= float_drag_value(
                    ui,
                    "Run speed (m/s)",
                    &mut edited.run_speed,
                    0.05,
                    Some(0.0..=30.0),
                );
            });
        egui::CollapsingHeader::new("Senses")
            .id_salt(("ai_senses_settings", entity))
            .show(ui, |ui| {
                Grid::new(("ai_perception_grid", entity))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        let p = &mut edited.perception;
                        changed |= float_drag_value(
                            ui,
                            "Sight range (m)",
                            &mut p.sight_range,
                            0.1,
                            Some(0.0..=200.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Field of view (°)",
                            &mut p.sight_angle,
                            1.0,
                            Some(0.0..=360.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Notices within (m)",
                            &mut p.near_range,
                            0.05,
                            Some(0.0..=20.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Hearing range (m)",
                            &mut p.hearing_range,
                            0.1,
                            Some(0.0..=200.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Memory (s)",
                            &mut p.memory,
                            0.1,
                            Some(0.0..=600.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Looks per second",
                            &mut p.rate,
                            0.1,
                            Some(0.1..=60.0),
                        );
                        changed |= float_drag_value(
                            ui,
                            "Eye height (m)",
                            &mut p.eye_height,
                            0.01,
                            Some(0.0..=5.0),
                        );
                        ui.label("Line of sight");
                        ComboBox::from_id_salt(("ai_los", entity))
                            .selected_text(format!("{:?}", p.line_of_sight))
                            .show_ui(ui, |ui| {
                                for option in [
                                    LineOfSight::Navmesh,
                                    LineOfSight::Physics,
                                    LineOfSight::None,
                                ] {
                                    changed |= ui
                                        .selectable_value(
                                            &mut p.line_of_sight,
                                            option,
                                            format!("{option:?}"),
                                        )
                                        .changed();
                                }
                            });
                        ui.end_row();
                        ui.label("Interests");
                        let mut interests = p.interests.join(", ");
                        if ui
                            .text_edit_singleline(&mut interests)
                            .on_hover_text("Tags it pays attention to (empty: anything with a tag)")
                            .changed()
                        {
                            p.interests = comma_list(&interests);
                            changed = true;
                        }
                        ui.end_row();
                    });
            });
        blackboard_editor(ui, entity, agent, actions);
        if changed {
            match serde_json::to_value(&edited) {
                Ok(mut value) => {
                    // The blackboard is edited on its own (it merges key by key).
                    if let Some(object) = value.as_object_mut() {
                        object.remove("blackboard");
                    }
                    set(actions, entity, value);
                }
                Err(err) => log::warn!("Cannot edit ai_agent: {err}"),
            }
        }

        debug_toggles(ui, status, actions);
        if status.playing {
            live_view(ui, entity, status, actions);
        } else {
            ui.label(theme::hint(
                "Brains defined by scripts exist in play; press Play to watch it think",
            ));
        }
    });
    header.context_menu(|ui| {
        let text = egui::RichText::new(format!("{}  Remove AI Agent", icons::DELETE))
            .color(theme::colors::ERROR);
        if ui.button(text).clicked() {
            set(actions, entity, Value::Null);
            ui.close();
        }
    });
}

/// Starting blackboard values as JSON, committed when the field loses focus.
fn blackboard_editor(
    ui: &mut egui::Ui,
    entity: Entity,
    agent: &AiAgent,
    actions: &mut Vec<InspectorAction>,
) {
    egui::CollapsingHeader::new("Starting blackboard")
        .id_salt(("ai_blackboard_defaults", entity))
        .show(ui, |ui| {
            let id = egui::Id::new(("ai_blackboard_draft", entity));
            let current = serde_json::to_string_pretty(&agent.blackboard).unwrap_or_default();
            let mut draft = ui
                .data_mut(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| current.clone());
            let response = ui.add(
                egui::TextEdit::multiline(&mut draft)
                    .code_editor()
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            let parsed = serde_json::from_str::<Value>(&draft);
            match &parsed {
                Ok(Value::Object(_)) => {}
                Ok(_) => {
                    ui.label(theme::hint("An object of key: value"));
                }
                Err(err) => {
                    ui.colored_label(theme::colors::ERROR, err.to_string());
                }
            }
            if response.lost_focus() {
                if let Ok(Value::Object(new)) = parsed {
                    if Value::Object(new.clone()) != agent.blackboard {
                        // Keys that went away are removed (null).
                        let mut patch = new.clone();
                        if let Value::Object(old) = &agent.blackboard {
                            for key in old.keys() {
                                if !new.contains_key(key) {
                                    patch.insert(key.clone(), Value::Null);
                                }
                            }
                        }
                        set(actions, entity, serde_json::json!({ "blackboard": patch }));
                    }
                }
                ui.data_mut(|d| d.remove::<String>(id));
            } else if response.has_focus() {
                ui.data_mut(|d| d.insert_temp(id, draft));
            }
            ui.label(theme::hint(
                "[x, y, z] arrays are positions; entities are their ids",
            ));
        });
}

fn debug_toggles(ui: &mut egui::Ui, status: &AiPanelStatus, actions: &mut Vec<InspectorAction>) {
    let mut debug = status.debug;
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Draw");
        changed |= ui
            .checkbox(&mut debug.senses, "Senses")
            .on_hover_text("View cones and hearing ranges")
            .changed();
        changed |= ui
            .checkbox(&mut debug.goals, "Goals")
            .on_hover_text("Lines to where agents walk")
            .changed();
        changed |= ui
            .checkbox(&mut debug.percepts, "Percepts")
            .on_hover_text("Lines to what they see (green) and remember (grey)")
            .changed();
        changed |= ui
            .checkbox(&mut debug.states, "States")
            .on_hover_text("A ring at each agent's feet in its state's colour")
            .changed();
    });
    if changed {
        actions.push(InspectorAction::Ai(AiRequest::SetDebug(debug)));
    }
}

fn live_view(
    ui: &mut egui::Ui,
    entity: Entity,
    status: &AiPanelStatus,
    actions: &mut Vec<InspectorAction>,
) {
    let Some(mind) = &status.selected else {
        ui.label(theme::hint("Not thinking yet (its brain is not defined?)"));
        return;
    };
    let text = |v: &Value| v.as_str().map(str::to_owned).unwrap_or_default();
    ui.separator();
    let state = text(&mind["state"]);
    if !state.is_empty() {
        ui.horizontal(|ui| {
            ui.label("State");
            let last = state.rsplit('/').next().unwrap_or(&state);
            ui.label(
                egui::RichText::new(&state)
                    .strong()
                    .color(state_color(last)),
            );
        });
    }
    let action = text(&mind["action"]);
    if !action.is_empty() {
        ui.horizontal(|ui| {
            ui.label("Doing");
            ui.label(egui::RichText::new(action).color(RUNNING));
        });
    }
    if let Some(error) = mind["error"].as_str() {
        ui.colored_label(theme::colors::ERROR, error);
    }
    ui.horizontal(|ui| {
        if ui.button("Restart").clicked() {
            actions.push(InspectorAction::Ai(AiRequest::Restart(entity)));
        }
        let paused = mind["paused"].as_bool().unwrap_or(false);
        if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
            actions.push(InspectorAction::Ai(AiRequest::Pause(entity, !paused)));
        }
    });

    egui::CollapsingHeader::new("Blackboard")
        .id_salt(("ai_live_blackboard", entity))
        .default_open(true)
        .show(ui, |ui| {
            for (title, key) in [
                ("", "blackboard"),
                ("group.", "group_blackboard"),
                ("global.", "global_blackboard"),
            ] {
                let Some(map) = mind[key].as_object() else {
                    continue;
                };
                if map.is_empty() {
                    continue;
                }
                Grid::new(("ai_bb_grid", entity, key))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (k, v) in map {
                            ui.label(format!("{title}{k}"));
                            let shown = match v {
                                Value::String(s) => s.clone(),
                                Value::Number(n) => match n.as_f64() {
                                    Some(f) if f.fract() != 0.0 => format!("{f:.3}"),
                                    _ => n.to_string(),
                                },
                                other => other.to_string(),
                            };
                            ui.label(egui::RichText::new(shown).monospace());
                            ui.end_row();
                        }
                    });
            }
        });

    let runtime = &mind["runtime"];
    if let Some(machine) = runtime.get("machine") {
        show_machine(ui, entity, machine, 0);
    }
    if let Some(tree) = runtime.get("tree") {
        show_tree(ui, entity, tree, "Tree");
    }

    egui::CollapsingHeader::new("Senses")
        .id_salt(("ai_live_senses", entity))
        .show(ui, |ui| {
            let percepts = mind["senses"]["percepts"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if percepts.is_empty() {
                ui.label(theme::hint("Sees nothing it cares about"));
            }
            for p in percepts {
                let tags: Vec<String> = p["tags"]
                    .as_array()
                    .map(|t| {
                        t.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let visible = p["visible"].as_bool().unwrap_or(false);
                let line = format!(
                    "{} {} {}",
                    p["entity"],
                    tags.join(","),
                    if visible {
                        "(in view)".to_string()
                    } else {
                        format!("(seen {:.1} s ago)", p["seen_ago"].as_f64().unwrap_or(0.0))
                    }
                );
                ui.label(egui::RichText::new(line).color(if visible {
                    SUCCEEDED
                } else {
                    theme::colors::TEXT_WEAK
                }));
            }
            for n in mind["senses"]["noises"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                ui.label(format!(
                    "heard {} {:.1} s ago",
                    n["tag"].as_str().unwrap_or("?"),
                    n["ago"].as_f64().unwrap_or(0.0)
                ));
            }
        });
}

fn show_machine(ui: &mut egui::Ui, entity: Entity, machine: &Value, depth: usize) {
    let name = machine["machine"].as_str().unwrap_or("machine");
    egui::CollapsingHeader::new(format!("Machine {name}"))
        .id_salt(("ai_machine", entity, depth))
        .default_open(depth == 0)
        .show(ui, |ui| {
            let current = machine["state"].as_str().unwrap_or("");
            ui.horizontal_wrapped(|ui| {
                for state in machine["states"].as_array().cloned().unwrap_or_default() {
                    let state = state.as_str().unwrap_or("").to_owned();
                    let text = egui::RichText::new(&state);
                    if state == current {
                        ui.label(text.strong().color(state_color(&state)));
                    } else {
                        ui.label(text.color(theme::colors::TEXT_WEAK));
                    }
                }
            });
            ui.label(theme::hint(format!(
                "in {current} for {:.1} s",
                machine["time_in_state"].as_f64().unwrap_or(0.0)
            )));
            for t in machine["transitions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                ui.label(
                    egui::RichText::new(t.as_str().unwrap_or(""))
                        .monospace()
                        .small(),
                );
            }
            let history = machine["history"].as_array().cloned().unwrap_or_default();
            if !history.is_empty() {
                ui.label(theme::hint("Lately:"));
                for h in history.iter().take(6) {
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.1} s ago {} -> {}: {}",
                            h["ago"].as_f64().unwrap_or(0.0),
                            h["from"].as_str().unwrap_or(""),
                            h["to"].as_str().unwrap_or(""),
                            h["why"].as_str().unwrap_or("")
                        ))
                        .small(),
                    );
                }
            }
            if let Some(tree) = machine.get("tree") {
                show_tree(ui, entity, tree, &format!("Tree in {current}"));
            }
            if let Some(inner) = machine.get("machine_in_state") {
                show_machine(ui, entity, inner, depth + 1);
            }
        });
}

fn show_tree(ui: &mut egui::Ui, entity: Entity, tree: &Value, title: &str) {
    egui::CollapsingHeader::new(title)
        .id_salt(("ai_tree", entity, title))
        .default_open(true)
        .show(ui, |ui| {
            for node in tree["nodes"].as_array().cloned().unwrap_or_default() {
                let depth = node["depth"].as_u64().unwrap_or(0) as usize;
                let active = node["active"].as_bool().unwrap_or(false);
                let this_tick = node["this_tick"].as_bool().unwrap_or(false);
                let color = match (active, node["last"].as_str()) {
                    (true, _) => RUNNING,
                    (false, Some("success")) if this_tick => SUCCEEDED,
                    (false, Some("failure")) if this_tick => FAILED,
                    _ => theme::colors::TEXT_WEAK,
                };
                let mut line = format!(
                    "{}{}",
                    "  ".repeat(depth),
                    node["label"].as_str().unwrap_or("?")
                );
                if let Some(left) = node["time_left"].as_f64() {
                    line.push_str(&format!("  ({left:.1} s)"));
                }
                ui.label(egui::RichText::new(line).monospace().color(color));
                if let Some(scores) = node["scores"].as_array() {
                    for s in scores {
                        ui.label(
                            egui::RichText::new(format!(
                                "{}  {:>6} {}",
                                "  ".repeat(depth + 1),
                                s["score"]
                                    .as_f64()
                                    .map_or("-".into(), |v| format!("{v:.2}")),
                                s["child"].as_str().unwrap_or("")
                            ))
                            .monospace()
                            .small()
                            .color(theme::colors::TEXT_WEAK),
                        );
                    }
                }
            }
        });
}

pub(super) fn add_ai_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    if ui
        .add_enabled(
            components.ai_agent.is_none(),
            egui::Button::new(format!("{}  AI Agent", icons::AI)),
        )
        .clicked()
    {
        set(
            actions,
            entity,
            serde_json::to_value(AiAgent::default()).unwrap_or_default(),
        );
        ui.close();
    }
}
