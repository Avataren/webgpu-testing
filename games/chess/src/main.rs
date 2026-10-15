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
    graphics_settings::graphics_settings,
    project::ProjectManifest,
    remote::{RemoteContext, RemoteResult},
    render_application::{run_application, DefaultUI, RenderApplication},
    scene::{
        MeshBounds, MeshComponent, Name, Parent, Scene, SceneLibrary, Transform,
        TransformComponent, Visible, WorldTransform,
    },
};

const GOLD: Color32 = Color32::from_rgb(205, 175, 116);
const INK: Color32 = Color32::from_rgb(17, 22, 25);
const PAPER: Color32 = Color32::from_rgb(238, 230, 212);

/// The scene to show: the study's scene, or the project's first.
const SCENE_NAME: &str = "Chess Study";

/// Where the squares are, in the space the pieces are placed in (their parent's).
/// White starts at +Z. Worked out from the starting position, so a set can be any
/// size and stand anywhere.
#[derive(Clone, Copy, Debug)]
struct BoardFrame {
    center: Vec3,
    square: f32,
}

impl Default for BoardFrame {
    /// The original procedural board: 1.2 m squares around the origin.
    fn default() -> Self {
        Self {
            center: Vec3::new(0.0, 0.226, 0.0),
            square: 1.2,
        }
    }
}

impl BoardFrame {
    /// From the 32 pieces of the starting position: the centre is their mean
    /// (the set-up is symmetric) and the square size comes from the white rooks.
    fn from_start(pieces: &[(Vec3, bool, String)]) -> Option<Self> {
        if pieces.is_empty() {
            return None;
        }
        let center = pieces.iter().map(|p| p.0).sum::<Vec3>() / pieces.len() as f32;
        let rooks: Vec<f32> = pieces
            .iter()
            .filter(|(_, white, kind)| *white && kind == "rook")
            .map(|p| p.0.x)
            .collect();
        let (min, max) = rooks
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        let square = (max - min) / 7.0;
        (rooks.len() == 2 && square > 1e-4).then_some(Self { center, square })
    }

    fn position(&self, square: usize) -> Vec3 {
        self.center
            + Vec3::new(
                ((square % 8) as f32 - 3.5) * self.square,
                0.0,
                (3.5 - (square / 8) as f32) * self.square,
            )
    }

    fn square_at(&self, position: Vec3) -> Option<usize> {
        let file = ((position.x - self.center.x) / self.square + 3.5).round();
        let rank = (3.5 - (position.z - self.center.z) / self.square).round();
        ((0.0..8.0).contains(&file) && (0.0..8.0).contains(&rank))
            .then_some(rank as usize * 8 + file as usize)
    }
}

/// Where captured pieces of one colour go: `per_line` slots along `along`, then the
/// next line one `across` further on.
#[derive(Clone, Copy, Debug)]
struct CaptureRail {
    origin: Vec3,
    along: Vec3,
    across: Vec3,
    per_line: usize,
    scale: Option<Vec3>,
}

impl CaptureRail {
    /// Next to the board on the table, from an anchor placed in the scene: lines of
    /// four stepping away from the board, then away from the middle.
    fn from_anchor(anchor: Vec3, board: &BoardFrame) -> Self {
        let away_x = (anchor.x - board.center.x).signum();
        let away_z = if (anchor.z - board.center.z).abs() < 1e-5 {
            1.0
        } else {
            (anchor.z - board.center.z).signum()
        };
        Self {
            origin: anchor,
            along: Vec3::X * away_x * board.square * 0.82,
            across: Vec3::Z * away_z * board.square * 0.9,
            per_line: 4,
            scale: None,
        }
    }

    /// The procedural board's rails: shrunken pieces in lines of twelve on a ledge.
    fn legacy(white: bool, board: &BoardFrame) -> Self {
        let side = if white { -1.0 } else { 1.0 };
        let k = board.square / 1.2;
        Self {
            origin: board.center + Vec3::new(side * 6.2, -0.566, 4.65) * k,
            along: Vec3::NEG_Z * 0.82 * k,
            across: Vec3::X * side * 0.8 * k,
            per_line: 12,
            scale: Some(Vec3::splat(0.56)),
        }
    }

    fn slot(&self, index: usize) -> Vec3 {
        self.origin
            + self.along * (index % self.per_line) as f32
            + self.across * (index / self.per_line) as f32
    }
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
    board: BoardFrame,
    rails: [CaptureRail; 2],
    markers: Vec<Entity>,
    marker_height: f32,
    camera: Option<Entity>,
    camera_start: Option<Transform>,
    pivot: Option<Vec3>,
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
    /// Frames seen; start-up graphics choices wait until the renderer has reported
    /// what the GPU supports and the saved settings are loaded.
    frames: u32,
}

impl Spectator {
    fn new(root: PathBuf, engine_path: PathBuf) -> Self {
        Self {
            project: root.join("games/chess/study"),
            game: Game::new(),
            engine: uci::Engine::new(engine_path),
            generation: 1,
            game_number: 1,
            pieces: Vec::new(),
            squares: [None; 64],
            meshes: HashMap::new(),
            board: BoardFrame::default(),
            rails: [
                CaptureRail::legacy(true, &BoardFrame::default()),
                CaptureRail::legacy(false, &BoardFrame::default()),
            ],
            markers: Vec::new(),
            marker_height: 0.01,
            camera: None,
            camera_start: None,
            pivot: None,
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
            frames: 0,
        }
    }

    fn map_scene(&mut self, scene: &Scene) -> Result<(), String> {
        let mut found = Vec::new();
        for (entity, (name, transform, mesh, bounds)) in scene
            .world()
            .query::<(&Name, &TransformComponent, &MeshComponent, &MeshBounds)>()
            .iter()
        {
            if name.0.starts_with("Chess.White.") || name.0.starts_with("Chess.Black.") {
                let kind = name.0.split('.').nth(2).unwrap_or_default().to_owned();
                found.push((entity, name.0.clone(), transform.0, *mesh, *bounds, kind));
            }
        }
        if found.len() != 32 {
            return Err(format!("Expected 32 pieces, found {}", found.len()));
        }
        let start: Vec<_> = found
            .iter()
            .map(|(_, name, t, _, _, kind)| {
                (
                    t.translation,
                    name.starts_with("Chess.White."),
                    kind.clone(),
                )
            })
            .collect();
        self.board = BoardFrame::from_start(&start).ok_or("Cannot find the two white rooks")?;
        for (entity, name, transform, mesh, bounds, _) in found {
            let white = name.starts_with("Chess.White.");
            let square = self
                .board
                .square_at(transform.translation)
                .ok_or_else(|| format!("Invalid starting square: {name}"))?;
            let piece = self
                .game
                .board
                .get_piece_at_square(square as u16)
                .ok_or_else(|| format!("No rules piece at {name}"))?;
            let kind = format!("{:?}", piece.piece_type()).to_lowercase();
            if piece.is_white() != white
                || !name.split('.').any(|part| part == kind)
                || self.squares[square].is_some()
            {
                return Err(format!("Piece mapping mismatch: {name}"));
            }
            self.squares[square] = Some(entity);
            self.pieces.push(PieceVisual {
                entity,
                initial: transform,
                mesh,
                bounds,
                square,
                white,
            });
            self.meshes.insert(kind, (mesh, bounds));
        }
        let anchor = |name: &str| {
            scene
                .world()
                .query::<(&Name, &TransformComponent)>()
                .iter()
                .find(|(_, (n, _))| n.0 == name)
                .map(|(_, (_, t))| t.0.translation)
        };
        for (index, (name, white)) in [
            ("Chess.Captures.White", true),
            ("Chess.Captures.Black", false),
        ]
        .into_iter()
        .enumerate()
        {
            self.rails[index] = anchor(name)
                .map(|origin| CaptureRail::from_anchor(origin, &self.board))
                .unwrap_or_else(|| CaptureRail::legacy(white, &self.board));
        }
        self.camera = scene.active_camera_entity();
        self.camera_start = self.camera.and_then(|camera| {
            scene
                .world()
                .get::<&TransformComponent>(camera)
                .ok()
                .map(|t| t.0)
        });
        let mut markers: Vec<_> = scene
            .world()
            .query::<(&Name, &TransformComponent)>()
            .iter()
            .filter(|(_, (name, _))| name.0.starts_with("Move.Inlay."))
            .map(|(e, (name, t))| (name.0.clone(), e, t.0.translation.y))
            .collect();
        markers.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some((_, _, y)) = markers.first() {
            self.marker_height = y - self.board.center.y;
        }
        self.markers = markers.into_iter().map(|(_, entity, _)| entity).collect();
        Ok(())
    }

    /// The board centre in world space, for the camera drift.
    fn board_pivot(&self, scene: &Scene) -> Option<Vec3> {
        let piece = self.pieces.first()?.entity;
        let world = scene.world();
        let parent = world.get::<&Parent>(piece).ok().map(|p| p.0);
        let frame = match parent {
            Some(parent) => world.get::<&WorldTransform>(parent).ok()?.0,
            None => Transform::IDENTITY,
        };
        Some(frame.matrix().transform_point3(self.board.center))
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
        let s = self.board.square;
        let mut motions = vec![Self::motion(
            scene,
            moving,
            self.board.position(delta.to),
            None,
            0.4 * s,
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
            let rail = self.rails[side];
            motions.push(Self::motion(
                scene,
                captured,
                rail.slot(index),
                rail.scale,
                0.83 * s,
            )?);
        }
        self.squares[delta.to] = Some(moving);
        if let Some((from, to)) = delta.rook {
            let rook = self.squares[from].take().ok_or("Missing castling rook")?;
            self.squares[to] = Some(rook);
            motions.push(Self::motion(
                scene,
                rook,
                self.board.position(to),
                None,
                0.27 * s,
            )?);
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
            let edge_offset = 0.4667 * s;
            let offset = match edge {
                0 => Vec3::new(0.0, 0.0, -edge_offset),
                1 => Vec3::new(0.0, 0.0, edge_offset),
                2 => Vec3::new(-edge_offset, 0.0, 0.0),
                _ => Vec3::new(edge_offset, 0.0, 0.0),
            };
            if let Ok(mut t) = scene.world().get::<&mut TransformComponent>(*marker) {
                t.0.translation =
                    self.board.position(square) + offset + Vec3::Y * self.marker_height;
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
            if self.pivot.is_none() {
                self.pivot = self.board_pivot(scene);
            }
            // Swing the authored camera slowly around the board and back.
            if let (Some(camera), Some(start), Some(pivot)) =
                (self.camera, self.camera_start, self.pivot)
            {
                let swing = Quat::from_rotation_y((self.orbit_time * 0.022).sin() * 0.26);
                let offset = start.translation - pivot;
                let bob = (self.orbit_time * 0.019).sin() * 0.03 * offset.length();
                if let Ok(mut transform) = scene.world().get::<&mut TransformComponent>(camera) {
                    transform.0.translation = pivot + swing * offset + Vec3::Y * bob;
                    transform.0.rotation = swing * start.rotation;
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

    /// The study is lit for path tracing: start with it where the GPU can (unless
    /// `CHESS_PATH_TRACING=0`), size the window from `CHESS_WINDOW=<w>x<h>`, and let P
    /// switch between path tracing and the raster lighting.
    fn apply_graphics(&mut self, toggle: bool) {
        self.frames = self.frames.saturating_add(1);
        let mut graphics = graphics_settings();
        let mut settings = graphics.settings.clone();
        if self.frames == 3 {
            let wanted = std::env::var("CHESS_PATH_TRACING").map_or(true, |v| v != "0");
            settings.path_tracing = wanted && graphics.capabilities.path_tracing;
            if let Some(size) = std::env::var("CHESS_WINDOW").ok().and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
            }) {
                settings.window_size = Some(size);
            }
        } else if toggle && self.frames > 3 && graphics.capabilities.path_tracing {
            settings.path_tracing = !settings.path_tracing;
        } else {
            return;
        }
        graphics.set(settings);
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
            let scene_id = manifest
                .scenes
                .iter()
                .find(|entry| entry.name == SCENE_NAME)
                .or_else(|| manifest.scenes.first())
                .map(|entry| entry.id.clone())
                .ok_or("The project has no scenes")?;
            workspace
                .set_active_scene(&scene_id)
                .map_err(|e| e.to_string())?;
            {
                let mut scene = workspace.active_scene_mut().ok_or("No board scene")?;
                self.map_scene(&scene)?;
                // Last-move markers appear with the first move.
                for marker in &self.markers {
                    let _ = scene.world_mut().insert_one(*marker, Visible(false));
                }
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
        self.apply_graphics(input.is_key_just_pressed("P"));
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
    // Graphics settings and save data are kept per game: the spectator has its own.
    wgpu_cube::scripting::lua::api::set_save_namespace("chess-spectator");
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
                    TransformComponent(Transform::from_translation(
                        BoardFrame::default().position(square as usize),
                    )),
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
                    assert!((position - BoardFrame::default().position(square)).length() < 0.001);
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
    fn board_frame_and_capture_rails_follow_a_real_scale_set() {
        let truth = BoardFrame {
            center: Vec3::new(0.3, 0.77, -0.2),
            square: 0.055,
        };
        let back = [
            "rook", "knight", "bishop", "queen", "king", "bishop", "knight", "rook",
        ];
        let mut start = Vec::new();
        for (file, piece) in back.iter().enumerate() {
            for (rank, white, kind) in [
                (0, true, *piece),
                (1, true, "pawn"),
                (6, false, "pawn"),
                (7, false, *piece),
            ] {
                start.push((truth.position(rank * 8 + file), white, kind.to_owned()));
            }
        }
        let frame = BoardFrame::from_start(&start).unwrap();
        assert!((frame.center - truth.center).length() < 1e-5);
        assert!((frame.square - truth.square).abs() < 1e-5);
        for square in 0..64 {
            assert_eq!(frame.square_at(frame.position(square)), Some(square));
        }
        // An anchor off the h-file towards black: lines step away from the board, then
        // towards black's end.
        let rail = CaptureRail::from_anchor(frame.center + Vec3::new(0.3, -0.02, -0.05), &frame);
        assert!(rail.slot(1).x > rail.slot(0).x);
        assert!(rail.slot(4).z < rail.slot(0).z);
        assert_eq!(rail.scale, None);
        assert!((rail.slot(3).x - rail.slot(0).x - 3.0 * 0.82 * 0.055).abs() < 1e-5);
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
