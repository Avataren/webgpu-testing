mod rules;
mod uci;

use egui::{Color32, RichText};
use glam::{Quat, Vec3};
use hecs::Entity;
use rules::{Game, MoveDelta};
use std::{collections::HashMap, path::PathBuf};
use wgpu_cube::{
    app::{StartupContext, UpdateContext},
    audio::{self, PlayOptions},
    project::ProjectManifest,
    remote::{RemoteContext, RemoteResult},
    render_application::{run_application, DefaultUI, RenderApplication},
    scene::{
        MeshBounds, MeshComponent, Name, Scene, SceneLibrary, Transform, TransformComponent,
        Visible,
    },
};

const GOLD: Color32 = Color32::from_rgb(205, 175, 116);
const INK: Color32 = Color32::from_rgb(17, 22, 25);
const PAPER: Color32 = Color32::from_rgb(238, 230, 212);

fn square_position(square: usize) -> Vec3 {
    Vec3::new(
        -4.2 + 1.2 * (square % 8) as f32,
        0.226,
        4.2 - 1.2 * (square / 8) as f32,
    )
}

#[derive(Clone, Copy)]
struct PieceVisual {
    entity: Entity,
    initial: Transform,
    mesh: MeshComponent,
    bounds: MeshBounds,
    square: usize,
    white: bool,
}
struct Motion {
    entity: Entity,
    from: Transform,
    to: Transform,
    lift: f32,
}
struct Animation {
    motions: Vec<Motion>,
    elapsed: f32,
    duration: f32,
    capture: bool,
}

struct Spectator {
    project: PathBuf,
    game: Game,
    engine: uci::Engine,
    generation: u64,
    game_number: u32,
    pieces: Vec<PieceVisual>,
    squares: [Option<Entity>; 64],
    meshes: HashMap<String, (MeshComponent, MeshBounds)>,
    markers: Vec<Entity>,
    camera: Option<Entity>,
    animation: Option<Animation>,
    pending_best: Option<String>,
    thinking: bool,
    paused: bool,
    sound: bool,
    orbit: bool,
    orbit_time: f32,
    speed: f32,
    search_millis: u64,
    delay: f32,
    ended: Option<String>,
    error: Option<String>,
    reset_requested: bool,
    info: String,
    evaluation: f32,
    wins: [u32; 3],
    captures: [usize; 2],
    last_move: Option<(usize, usize)>,
    close_requested: bool,
}

impl Spectator {
    fn new(root: PathBuf, engine_path: PathBuf) -> Self {
        Self {
            project: root.join("examples/projects/procedural-chess"),
            game: Game::new(),
            engine: uci::Engine::new(engine_path),
            generation: 1,
            game_number: 1,
            pieces: Vec::new(),
            squares: [None; 64],
            meshes: HashMap::new(),
            markers: Vec::new(),
            camera: None,
            animation: None,
            pending_best: None,
            thinking: false,
            paused: false,
            sound: true,
            orbit: true,
            orbit_time: 0.0,
            speed: 1.0,
            search_millis: 1000,
            delay: 1.4,
            ended: None,
            error: None,
            reset_requested: false,
            info: String::new(),
            evaluation: 0.0,
            wins: [0; 3],
            captures: [0; 2],
            last_move: None,
            close_requested: false,
        }
    }

    fn map_scene(&mut self, scene: &Scene) -> Result<(), String> {
        for (entity, (name, transform, mesh, bounds)) in scene
            .world()
            .query::<(&Name, &TransformComponent, &MeshComponent, &MeshBounds)>()
            .iter()
        {
            if name.0.starts_with("Chess.White.") || name.0.starts_with("Chess.Black.") {
                let white = name.0.starts_with("Chess.White.");
                let file = ((transform.0.translation.x + 4.2) / 1.2).round() as usize;
                let rank = ((4.2 - transform.0.translation.z) / 1.2).round() as usize;
                if file >= 8 || rank >= 8 {
                    return Err(format!("Invalid starting square: {}", name.0));
                }
                let square = rank * 8 + file;
                let piece = self
                    .game
                    .board
                    .get_piece_at_square(square as u16)
                    .ok_or_else(|| format!("No rules piece at {}", name.0))?;
                let kind = format!("{:?}", piece.piece_type()).to_lowercase();
                if piece.is_white() != white
                    || !name.0.split('.').any(|part| part == kind)
                    || self.squares[square].is_some()
                {
                    return Err(format!("Piece mapping mismatch: {}", name.0));
                }
                self.squares[square] = Some(entity);
                self.pieces.push(PieceVisual {
                    entity,
                    initial: transform.0,
                    mesh: *mesh,
                    bounds: *bounds,
                    square,
                    white,
                });
                self.meshes.insert(kind, (*mesh, *bounds));
            }
        }
        if self.pieces.len() != 32 {
            return Err(format!("Expected 32 pieces, found {}", self.pieces.len()));
        }
        self.camera = scene.active_camera_entity();
        let mut markers: Vec<_> = scene
            .world()
            .query::<&Name>()
            .iter()
            .filter(|(_, name)| name.0.starts_with("Move.Inlay."))
            .map(|(e, name)| (name.0.clone(), e))
            .collect();
        markers.sort_by(|a, b| a.0.cmp(&b.0));
        self.markers = markers.into_iter().map(|(_, entity)| entity).collect();
        Ok(())
    }

    fn reset(&mut self, scene: &mut Scene) {
        self.generation += 1;
        self.game_number += 1;
        self.game = Game::new();
        self.squares = [None; 64];
        for p in &self.pieces {
            self.squares[p.square] = Some(p.entity);
            let _ = scene.world_mut().insert(
                p.entity,
                (
                    TransformComponent(p.initial),
                    p.mesh,
                    p.bounds,
                    Visible(true),
                ),
            );
        }
        for marker in &self.markers {
            let _ = scene.world_mut().insert_one(*marker, Visible(false));
        }
        self.animation = None;
        self.pending_best = None;
        self.thinking = false;
        self.delay = 1.2;
        self.ended = None;
        self.error = None;
        self.info.clear();
        self.evaluation = 0.0;
        self.captures = [0; 2];
        self.last_move = None;
        self.reset_requested = false;
        log::info!("Starting exhibition {}", self.game_number);
    }

    fn motion(
        scene: &Scene,
        entity: Entity,
        target: Vec3,
        scale: Option<Vec3>,
        lift: f32,
    ) -> Result<Motion, String> {
        let from = scene
            .world()
            .get::<&TransformComponent>(entity)
            .map_err(|e| e.to_string())?
            .0;
        let mut to = from;
        to.translation = target;
        if let Some(scale) = scale {
            to.scale = scale;
        }
        Ok(Motion {
            entity,
            from,
            to,
            lift,
        })
    }

    fn animate(&mut self, scene: &mut Scene, delta: MoveDelta) -> Result<(), String> {
        let moving = self.squares[delta.from]
            .take()
            .ok_or("Missing moving piece")?;
        let mut motions = vec![Self::motion(
            scene,
            moving,
            square_position(delta.to),
            None,
            0.48,
        )?];
        if let Some(square) = delta.captured {
            let captured = self.squares[square]
                .take()
                .ok_or("Missing captured piece")?;
            let white = self
                .pieces
                .iter()
                .find(|p| p.entity == captured)
                .ok_or("Unknown captured piece")?
                .white;
            let side = usize::from(!white);
            let index = self.captures[side];
            self.captures[side] += 1;
            let target = Vec3::new(
                if white { -6.2 } else { 6.2 },
                -0.34,
                4.65 - (index % 12) as f32 * 0.82,
            );
            // The second rail holds exceptionally crowded capture lists.
            let target = target
                + Vec3::X
                    * if index >= 12 {
                        if white {
                            -0.8
                        } else {
                            0.8
                        }
                    } else {
                        0.0
                    };
            motions.push(Self::motion(
                scene,
                captured,
                target,
                Some(Vec3::splat(0.56)),
                1.0,
            )?);
        }
        self.squares[delta.to] = Some(moving);
        if let Some((from, to)) = delta.rook {
            let rook = self.squares[from].take().ok_or("Missing castling rook")?;
            self.squares[to] = Some(rook);
            motions.push(Self::motion(scene, rook, square_position(to), None, 0.32)?);
        }
        if let Some(kind) = delta.promotion {
            let (mesh, bounds) = self
                .meshes
                .get(&format!("{kind:?}").to_lowercase())
                .ok_or("Missing promotion mesh")?;
            scene
                .world_mut()
                .insert(moving, (*mesh, *bounds))
                .map_err(|e| e.to_string())?;
        }
        self.last_move = Some((delta.from, delta.to));
        for (index, marker) in self.markers.iter().enumerate() {
            let square = if index < 4 { delta.from } else { delta.to };
            let edge = index % 4;
            let offset = match edge {
                0 => Vec3::new(0.0, 0.0, -0.56),
                1 => Vec3::new(0.0, 0.0, 0.56),
                2 => Vec3::new(-0.56, 0.0, 0.0),
                _ => Vec3::new(0.56, 0.0, 0.0),
            };
            if let Ok(mut t) = scene.world().get::<&mut TransformComponent>(*marker) {
                t.0.translation = square_position(square) + offset + Vec3::Y * 0.01;
            }
            let _ = scene.world_mut().insert_one(*marker, Visible(true));
        }
        self.animation = Some(Animation {
            motions,
            elapsed: 0.0,
            duration: 0.9,
            capture: delta.captured.is_some(),
        });
        Ok(())
    }

    fn sound(&self, name: &str) {
        if !self.sound {
            return;
        }
        let path = self.project.join(format!("content/audio/{name}.wav"));
        if let Err(error) = audio::play(
            &path.to_string_lossy(),
            PlayOptions {
                volume: 0.28,
                ..Default::default()
            },
        ) {
            log::warn!("Chess audio: {error}");
        }
    }

    fn collect_engine(&mut self) {
        while let Ok(event) = self.engine.events.try_recv() {
            match event {
                uci::Event::Best { generation, uci } if generation == self.generation => {
                    self.thinking = false;
                    self.pending_best = Some(uci);
                }
                uci::Event::Error {
                    generation,
                    message,
                } if generation == self.generation => {
                    self.thinking = false;
                    self.error = Some(message);
                    self.delay = 3.0;
                }
                uci::Event::Info { generation, text } if generation == self.generation => {
                    let words: Vec<_> = text.split_whitespace().collect();
                    let value = |key: &str| {
                        words
                            .iter()
                            .position(|w| *w == key)
                            .and_then(|i| words.get(i + 1))
                            .copied()
                    };
                    if let Some(score) = value("cp").and_then(|s| s.parse::<f32>().ok()) {
                        self.evaluation = score / 100.0
                            * if self.game.board.is_white_active() {
                                1.0
                            } else {
                                -1.0
                            };
                    }
                    if let Some(mate) = value("mate").and_then(|s| s.parse::<i32>().ok()) {
                        self.evaluation = mate.signum() as f32
                            * 20.0
                            * if self.game.board.is_white_active() {
                                1.0
                            } else {
                                -1.0
                            };
                    }
                    self.info = format!(
                        "Depth {}  ·  {}k nodes",
                        value("depth").unwrap_or("–"),
                        value("nodes")
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(0)
                            / 1000
                    );
                }
                _ => {}
            }
        }
    }

    fn tick(&mut self, scene: &mut Scene, dt: f32) {
        self.collect_engine();
        if self.reset_requested {
            self.reset(scene);
        }
        if self.orbit && !self.paused {
            self.orbit_time += dt;
            let angle = 0.44 + (self.orbit_time * 0.022).sin() * 0.26;
            let eye = Vec3::new(
                angle.sin() * 19.8,
                15.6 + (self.orbit_time * 0.019).sin() * 0.6,
                angle.cos() * 19.8,
            );
            if let Some(camera) = self.camera {
                if let Ok(mut transform) = scene.world().get::<&mut TransformComponent>(camera) {
                    transform.0.translation = eye;
                    transform.0.rotation = Quat::from_mat4(
                        &glam::Mat4::look_at_rh(eye, Vec3::new(0.0, 0.35, 0.0), Vec3::Y).inverse(),
                    );
                }
            }
        }
        if self.paused {
            return;
        }
        let dt = dt * self.speed;
        if let Some(animation) = &mut self.animation {
            animation.elapsed += dt;
            let t = (animation.elapsed / animation.duration).min(1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            for motion in &animation.motions {
                if let Ok(mut transform) =
                    scene.world().get::<&mut TransformComponent>(motion.entity)
                {
                    transform.0.translation =
                        motion.from.translation.lerp(motion.to.translation, smooth)
                            + Vec3::Y * (std::f32::consts::PI * t).sin() * motion.lift;
                    transform.0.scale = motion.from.scale.lerp(motion.to.scale, smooth);
                }
            }
            if t >= 1.0 {
                let capture = animation.capture;
                self.animation = None;
                self.sound(if capture {
                    "chess_capture"
                } else {
                    "chess_move"
                });
                self.delay = 0.55;
                if let Some(result) = self.game.result() {
                    let score = if result.starts_with("Ivory") {
                        0
                    } else if result.starts_with("Ebony") {
                        1
                    } else {
                        2
                    };
                    self.wins[score] += 1;
                    log::info!("Game {} finished: {result}", self.game_number);
                    self.ended = Some(result);
                    self.delay = 7.0;
                    self.sound("chess_end");
                } else if self.game.in_check() {
                    self.sound("chess_check");
                }
            }
            return;
        }
        self.delay = (self.delay - dt).max(0.0);
        if self.delay > 0.0 {
            return;
        }
        if self.ended.is_some() {
            self.reset(scene);
            return;
        }
        if let Some(best) = self.pending_best.take() {
            match self
                .game
                .apply(&best)
                .and_then(|delta| self.animate(scene, delta))
            {
                Ok(()) => {
                    self.error = None;
                    log::info!(
                        "Game {} ply {}: {}",
                        self.game_number,
                        self.game.history.len(),
                        best
                    );
                }
                Err(message) => {
                    self.error = Some(message);
                    self.paused = true;
                }
            }
        } else if !self.thinking {
            match self
                .engine
                .search(self.generation, &self.game.history, self.search_millis)
            {
                Ok(()) => {
                    self.thinking = true;
                    self.error = None;
                }
                Err(message) => {
                    self.error = Some(message);
                    self.delay = 3.0;
                }
            }
        }
    }

    fn status(&self) -> serde_json::Value {
        serde_json::json!({"game": self.game_number, "generation": self.generation, "ply": self.game.history.len(), "history": self.game.history, "notation": self.game.notation, "white_to_move": self.game.board.is_white_active(), "thinking": self.thinking, "search_millis": self.search_millis, "animating": self.animation.is_some(), "paused": self.paused, "result": self.ended, "error": self.error, "mapped_pieces": self.pieces.len(), "on_board": self.squares.iter().flatten().count(), "captures": self.captures, "score": self.wins})
    }
}

impl RenderApplication for Spectator {
    fn name(&self) -> &str {
        "The Long Game · XavChess"
    }
    fn show_default_ui(&self) -> bool {
        false
    }
    fn setup(&mut self, ctx: &mut StartupContext) {
        let result = (|| {
            let manifest =
                ProjectManifest::load_from_dir(&self.project).map_err(|e| e.to_string())?;
            let (mut workspace, changed) = manifest
                .instantiate_workspace(ctx.renderer, &self.project, &mut SceneLibrary::new())
                .map_err(|e| e.to_string())?;
            workspace
                .set_active_scene(&"Procedural_Chess.scene".to_owned())
                .map_err(|e| e.to_string())?;
            {
                let scene = workspace.active_scene_mut().ok_or("No board scene")?;
                self.map_scene(&scene)?;
            }
            ctx.request_workspace_swap(workspace, changed);
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
            self.paused = true;
        }
    }
    fn update(&mut self, ctx: &mut UpdateContext) {
        let input = ctx.scene.input_state();
        if input.is_key_just_pressed("Space") {
            self.paused = !self.paused;
        }
        if input.is_key_just_pressed("R") {
            self.reset_requested = true;
        }
        if input.is_key_just_pressed("C") {
            self.orbit = !self.orbit;
        }
        if input.is_key_just_pressed("M") {
            self.sound = !self.sound;
        }
        if input.is_key_just_pressed("Escape") {
            self.close_requested = true;
        }
        if !self.pieces.is_empty() {
            self.tick(&mut ctx.scene, ctx.dt as f32);
        }
    }
    fn remote_methods(&self) -> Vec<&'static str> {
        vec!["chess.status", "chess.restart", "chess.configure"]
    }
    fn remote_command(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        _ctx: &mut RemoteContext<'_, '_>,
    ) -> Option<RemoteResult> {
        match method {
            "chess.status" => Some(Ok(self.status())),
            "chess.restart" => {
                self.reset_requested = true;
                Some(Ok(serde_json::json!({"queued": true})))
            }
            "chess.configure" => {
                if let Some(value) = params.get("paused").and_then(|v| v.as_bool()) {
                    self.paused = value;
                }
                if let Some(value) = params.get("orbit").and_then(|v| v.as_bool()) {
                    self.orbit = value;
                }
                if let Some(value) = params.get("sound").and_then(|v| v.as_bool()) {
                    self.sound = value;
                }
                if let Some(value) = params.get("speed").and_then(|v| v.as_f64()) {
                    self.speed = (value as f32).clamp(0.25, 16.0);
                }
                if let Some(value) = params.get("search_millis").and_then(|v| v.as_u64()) {
                    self.search_millis = value.clamp(10, 5000);
                }
                Some(Ok(self.status()))
            }
            _ => None,
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _default_ui: &mut DefaultUI) {
        let ctx = ui.ctx().clone();
        if self.close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let rect = ctx.content_rect();
        egui::Area::new("chess_title".into())
            .fixed_pos(egui::pos2(34.0, 30.0))
            .show(&ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(17, 22, 25, 205))
                    .corner_radius(10)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("X A V C H E S S   /   E X H I B I T I O N")
                                .size(11.0)
                                .color(GOLD),
                        );
                        ui.add_space(7.0);
                        ui.label(RichText::new("The Long Game").size(32.0).color(PAPER));
                        ui.label(
                            RichText::new("Ivory & ebony. An endless conversation.")
                                .size(12.0)
                                .color(Color32::from_rgb(174, 171, 157)),
                        );
                    });
            });
        let panel_width = 258.0;
        egui::Area::new("chess_session".into())
            .fixed_pos(egui::pos2(rect.right() - panel_width - 28.0, 30.0))
            .show(&ctx, |ui| {
                egui::Frame::new()
                    .fill(INK.gamma_multiply(0.94))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(67, 65, 52)))
                    .corner_radius(12)
                    .inner_margin(20.0)
                    .show(ui, |ui| {
                        ui.set_width(panel_width - 40.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("GAME {:03}", self.game_number))
                                    .size(11.0)
                                    .color(GOLD),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(if self.paused { "PAUSED" } else { "LIVE" })
                                            .size(10.0)
                                            .color(GOLD),
                                    );
                                },
                            );
                        });
                        ui.add_space(19.0);
                        for (index, label) in ["Ivory", "Ebony"].iter().enumerate() {
                            let active = self.game.board.is_white_active() == (index == 0)
                                && self.ended.is_none();
                            ui.horizontal(|ui| {
                                let (icon, _) = ui.allocate_exact_size(
                                    egui::vec2(26.0, 32.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    icon.center(),
                                    8.0,
                                    if index == 0 {
                                        PAPER
                                    } else {
                                        Color32::from_rgb(36, 43, 46)
                                    },
                                );
                                ui.painter().circle_stroke(
                                    icon.center(),
                                    8.0,
                                    egui::Stroke::new(1.5, GOLD),
                                );
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(*label).size(18.0).color(if active {
                                        PAPER
                                    } else {
                                        Color32::from_gray(138)
                                    }));
                                    ui.label(
                                        RichText::new("XavChess · Classical")
                                            .size(10.0)
                                            .color(Color32::from_gray(135)),
                                    );
                                });
                                if active {
                                    ui.label(RichText::new("•").size(25.0).color(GOLD));
                                }
                            });
                            ui.add_space(10.0);
                        }
                        ui.separator();
                        ui.add_space(10.0);
                        let status = if let Some(result) = &self.ended {
                            result.clone()
                        } else if self.paused {
                            "Exhibition paused".to_owned()
                        } else if self.animation.is_some() {
                            "On the board".to_owned()
                        } else if self.game.in_check() {
                            "Check · considering the reply".to_owned()
                        } else {
                            "Considering the next move".to_owned()
                        };
                        ui.label(RichText::new(status).size(12.0).color(PAPER));
                        ui.label(
                            RichText::new(&self.info)
                                .size(10.0)
                                .color(Color32::from_gray(135)),
                        );
                        ui.add_space(12.0);
                        let balance =
                            (0.5 + (self.evaluation / 6.0).tanh() * 0.46).clamp(0.04, 0.96);
                        let (bar, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 5.0),
                            egui::Sense::hover(),
                        );
                        ui.painter()
                            .rect_filled(bar, 3, Color32::from_rgb(64, 71, 73));
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                bar.min,
                                egui::vec2(bar.width() * balance, bar.height()),
                            ),
                            3,
                            PAPER,
                        );
                        ui.add_space(5.0);
                        ui.label(
                            RichText::new(format!("POSITION    {:+.2}", self.evaluation))
                                .size(9.0)
                                .color(GOLD),
                        );
                        ui.add_space(15.0);
                        ui.label(RichText::new("RECENT MOVES").size(10.0).color(GOLD));
                        ui.add_space(5.0);
                        let start = self.game.notation.len().saturating_sub(12) / 2;
                        egui::Grid::new("notation")
                            .num_columns(3)
                            .min_col_width(48.0)
                            .spacing(egui::vec2(8.0, 7.0))
                            .show(ui, |ui| {
                                for row in start..self.game.notation.len().div_ceil(2) {
                                    ui.label(
                                        RichText::new(format!("{}.", row + 1))
                                            .size(11.0)
                                            .color(Color32::from_gray(125)),
                                    );
                                    ui.label(
                                        RichText::new(&self.game.notation[row * 2])
                                            .size(12.0)
                                            .color(PAPER),
                                    );
                                    ui.label(
                                        RichText::new(
                                            self.game
                                                .notation
                                                .get(row * 2 + 1)
                                                .map(String::as_str)
                                                .unwrap_or("…"),
                                        )
                                        .size(12.0)
                                        .color(PAPER),
                                    );
                                    ui.end_row();
                                }
                            });
                        ui.add_space(15.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "SESSION   {} / {} / {}",
                                self.wins[0], self.wins[2], self.wins[1]
                            ))
                            .size(10.0)
                            .color(GOLD),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .button(if self.paused {
                                    "▶ Resume"
                                } else {
                                    "Ⅱ Pause"
                                })
                                .clicked()
                            {
                                self.paused = !self.paused;
                            }
                            if ui.button("New game").clicked() {
                                self.reset_requested = true;
                            }
                        });
                        ui.add_space(5.0);
                        ui.add(
                            egui::Slider::new(&mut self.speed, 0.5..=4.0)
                                .text("Pace")
                                .suffix("×"),
                        );
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.orbit, "Camera drift");
                            ui.checkbox(&mut self.sound, "Sound");
                        });
                        if let Some(error) = &self.error {
                            ui.colored_label(Color32::LIGHT_RED, error);
                        }
                    });
            });
        egui::Area::new("chess_footer".into())
            .fixed_pos(egui::pos2(34.0, rect.bottom() - 39.0))
            .show(&ctx, |ui| {
                ui.label(
                    RichText::new("SPACE  pause     R  new game     C  camera     M  sound")
                        .size(10.0)
                        .color(Color32::from_rgb(184, 181, 169)),
                );
            });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,chess_spectator=info"),
    )
    .init();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    std::env::set_current_dir(&root)?;
    let engine = std::env::var_os("CHESS_UCI_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/chess-uci/release/xavchess-hce"));
    if !engine.is_file() {
        return Err(format!(
            "Missing HCE engine at {}. Run ./run-chess.sh to build it.",
            engine.display()
        )
        .into());
    }
    run_application(Spectator::new(root, engine))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu_cube::asset::Handle;

    fn fixture() -> (Spectator, Scene) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut app = Spectator::new(
            root.clone(),
            root.join("target/chess-uci/release/xavchess-hce"),
        );
        app.sound = false;
        app.orbit = false;
        let mut scene = Scene::new();
        for square in 0..64 {
            if let Some(piece) = app.game.board.get_piece_at_square(square) {
                scene.world_mut().spawn((
                    Name::new(
                        format!(
                            "Chess.{}.{:?}.{square}",
                            if piece.is_white() { "White" } else { "Black" },
                            piece.piece_type()
                        )
                        .replace(
                            &format!("{:?}", piece.piece_type()),
                            &format!("{:?}", piece.piece_type()).to_lowercase(),
                        ),
                    ),
                    TransformComponent(Transform::from_translation(square_position(
                        square as usize,
                    ))),
                    MeshComponent(Handle::new(piece.piece_type() as usize)),
                    MeshBounds::new(Vec3::ZERO, Vec3::ONE),
                    Visible(true),
                ));
            }
        }
        app.map_scene(&scene).unwrap();
        (app, scene)
    }

    fn play(app: &mut Spectator, scene: &mut Scene, moves: &[&str]) {
        for uci in moves {
            let delta = app.game.apply(uci).unwrap();
            app.animate(scene, delta).unwrap();
            app.tick(scene, 1.0);
            for square in 0..64 {
                assert_eq!(
                    app.squares[square].is_some(),
                    app.game.board.get_piece_at_square(square as u16).is_some()
                );
                if let Some(entity) = app.squares[square] {
                    let position = scene
                        .world()
                        .get::<&TransformComponent>(entity)
                        .unwrap()
                        .0
                        .translation;
                    assert!((position - square_position(square)).length() < 0.001);
                }
            }
        }
    }

    #[test]
    fn scene_tracks_captures_castling_and_reset() {
        let (mut app, mut scene) = fixture();
        play(
            &mut app,
            &mut scene,
            &["e2e4", "a7a6", "e4e5", "d7d5", "e5d6"],
        );
        assert_eq!(app.captures, [0, 1]);
        app.reset(&mut scene);
        assert_eq!(app.squares.iter().flatten().count(), 32);
        assert!(app.game.history.is_empty());
        play(
            &mut app,
            &mut scene,
            &[
                "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1g1", "f8c5", "d2d3", "e8g8",
            ],
        );
        app.reset(&mut scene);
        play(
            &mut app,
            &mut scene,
            &[
                "d2d4", "d7d5", "b1c3", "b8c6", "c1f4", "c8f5", "d1d2", "d8d7", "e1c1", "e8c8",
            ],
        );
    }

    #[test]
    fn promotion_mesh_restores_and_stale_search_is_ignored() {
        let (mut app, mut scene) = fixture();
        let pawn = app.squares[8].unwrap();
        let white_king = app.squares[4].unwrap();
        let black_king = app.squares[60].unwrap();
        app.squares = [None; 64];
        app.squares[48] = Some(pawn);
        app.squares[7] = Some(white_king);
        app.squares[63] = Some(black_king);
        app.game.board.set_from_fen("7k/P7/8/8/8/8/8/7K w - - 0 1");
        let initial_mesh = scene.world().get::<&MeshComponent>(pawn).unwrap().0;
        let delta = app.game.apply("a7a8n").unwrap();
        app.animate(&mut scene, delta).unwrap();
        app.tick(&mut scene, 1.0);
        assert_eq!(
            scene.world().get::<&MeshComponent>(pawn).unwrap().0,
            app.meshes["knight"].0 .0
        );
        app.reset(&mut scene);
        assert_eq!(
            scene.world().get::<&MeshComponent>(pawn).unwrap().0,
            initial_mesh
        );
        app.engine.search(app.generation, &[], 60).unwrap();
        app.thinking = true;
        app.reset(&mut scene);
        loop {
            match app
                .engine
                .events
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap()
            {
                uci::Event::Best { generation, .. } => {
                    assert_ne!(generation, app.generation);
                    break;
                }
                uci::Event::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        }
        app.collect_engine();
        assert!(app.game.history.is_empty());
        assert!(app.pending_best.is_none());
    }
}
