//! Sound in a playing scene: the listener follows an `AudioListener` (or the
//! active camera), `AudioSource`s start from their entities, every sound
//! attached to an entity follows it, with velocities for doppler, and the
//! `AudioVolume`s around the listener change the mix.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use hecs::{Entity, World};

use super::components::{AudioListener, AudioSource, AudioVolume, AudioVolumeBus, SpatialSetting};
use super::layout::{MixerLayout, SnapshotLayout};
use super::mix::{Ambience, BusAmbience, OPEN_HZ};
use super::{Audio, PlayOptions};
use crate::scene::camera::CameraProjection;
use crate::scene::components::{CameraComponent, TransformComponent, VolumeShape, WorldTransform};

/// Moving faster than this (world units per second) is a teleport or a
/// respawn: the velocity counts as zero rather than as a doppler swoop.
const TELEPORT_SPEED: f32 = 250.0;
/// The 2D pan width without an orthographic camera or a listener's own.
const DEFAULT_PAN_WIDTH: f32 = 10.0;

#[derive(Default)]
pub(super) struct SceneAudio {
    /// The listener's last position.
    listener: Option<[f32; 3]>,
    /// Voices following an entity, with the entity's last position.
    attached: HashMap<u64, (Entity, [f32; 3])>,
    /// `AudioSource`s started this play session (or that failed to), with
    /// their voice, so a finished one-shot does not start again.
    started: HashMap<Entity, Option<u64>>,
    /// Snapshots fading in or out.
    snapshots: Vec<SnapshotFade>,
    /// How much each audio volume applied at the listener last frame.
    pub(super) volume_weights: Vec<(Entity, f32)>,
}

/// A snapshot on its way to `target` (1 on, 0 off) at `rate` per second.
#[derive(Clone, Debug, PartialEq)]
struct SnapshotFade {
    name: String,
    weight: f32,
    target: f32,
    rate: f32,
}

impl SceneAudio {
    pub(super) fn clear(&mut self) {
        *self = SceneAudio::default();
    }

    /// Fades to the named snapshot (or, with `None`, back to none) over
    /// `seconds`; the one active before fades out meanwhile.
    pub(super) fn set_snapshot(&mut self, name: Option<&str>, seconds: f32) {
        let rate = if seconds > 0.0 {
            1.0 / seconds
        } else {
            f32::INFINITY
        };
        for fade in &mut self.snapshots {
            fade.target = 0.0;
            fade.rate = rate;
        }
        if let Some(name) = name {
            match self.snapshots.iter_mut().find(|f| f.name == name) {
                Some(fade) => fade.target = 1.0,
                None => self.snapshots.push(SnapshotFade {
                    name: name.to_string(),
                    weight: 0.0,
                    target: 1.0,
                    rate,
                }),
            }
        }
    }

    /// The snapshot fading in (or on), if any.
    pub(super) fn snapshot(&self) -> Option<&str> {
        self.snapshots
            .iter()
            .find(|f| f.target > 0.0)
            .map(|f| f.name.as_str())
    }

    /// The snapshots heard now, with their weights.
    pub(super) fn snapshot_weights(&self) -> Vec<(String, f32)> {
        self.snapshots
            .iter()
            .map(|f| (f.name.clone(), f.weight))
            .collect()
    }

    fn advance_snapshots(&mut self, dt: f32) {
        for fade in &mut self.snapshots {
            if fade.rate.is_infinite() {
                fade.weight = fade.target;
                continue;
            }
            let step = fade.rate * dt.max(0.0);
            fade.weight = if fade.weight < fade.target {
                (fade.weight + step).min(fade.target)
            } else {
                (fade.weight - step).max(fade.target)
            };
        }
        self.snapshots.retain(|f| f.target > 0.0 || f.weight > 0.0);
    }

    pub(super) fn attach(&mut self, voice: u64, entity: Entity, position: [f32; 3]) {
        self.attached.insert(voice, (entity, position));
    }

    /// Records a source started by a script (and attaches it when spatial).
    pub(super) fn started(&mut self, entity: Entity, voice: u64, position: Option<[f32; 3]>) {
        self.started.insert(entity, Some(voice));
        if let Some(position) = position {
            self.attach(voice, entity, position);
        }
    }

    /// The voice an entity's `AudioSource` last started.
    pub(super) fn voice_of(&self, entity: Entity) -> Option<u64> {
        self.started.get(&entity).copied().flatten()
    }
}

fn place(world: &World, entity: Entity) -> Option<(Vec3, Quat)> {
    let transform = world.get::<&WorldTransform>(entity).ok()?;
    Some((transform.0.translation, transform.0.rotation))
}

/// Velocity from the last position, or zero after a teleport.
fn velocity(last: [f32; 3], now: [f32; 3], dt: f32) -> [f32; 3] {
    if dt <= 0.0 {
        return [0.0; 3];
    }
    let v = (Vec3::from(now) - Vec3::from(last)) / dt;
    if v.length() > TELEPORT_SPEED {
        [0.0; 3]
    } else {
        v.to_array()
    }
}

pub(super) fn update(audio: &mut Audio, world: &World, camera: Option<Entity>, dt: f32) {
    let mut scene = std::mem::take(&mut audio.scene);
    update_listener(audio, &mut scene, world, camera, dt);
    scene.advance_snapshots(dt);
    let (ambience, weights) = ambience(
        world,
        scene.listener.map(Vec3::from),
        audio.output.layout(),
        &scene.snapshot_weights(),
    );
    scene.volume_weights = weights;
    audio.output.set_ambience(ambience);
    start_sources(audio, &mut scene, world);
    follow(audio, &mut scene, world, dt);
    audio.output.flush();
    audio.scene = scene;
}

fn update_listener(
    audio: &mut Audio,
    scene: &mut SceneAudio,
    world: &World,
    camera: Option<Entity>,
    dt: f32,
) {
    let own = world
        .query::<&AudioListener>()
        .iter()
        .next()
        .map(|(entity, listener)| (entity, listener.clone()));
    let (entity, settings) = match own {
        Some((entity, listener)) => (entity, listener),
        None => match camera {
            Some(camera) => (camera, AudioListener::default()),
            None => return,
        },
    };
    let Some((position, rotation)) = place(world, entity) else {
        return;
    };
    let ortho_half_width =
        world
            .get::<&CameraComponent>(entity)
            .ok()
            .and_then(|camera| match camera.projection {
                CameraProjection::Orthographic { left, right, .. } => {
                    Some((right - left).abs() / 2.0)
                }
                _ => None,
            });
    audio.listener_planar = match settings.mode {
        SpatialSetting::Planar => true,
        SpatialSetting::Full => false,
        SpatialSetting::Auto | SpatialSetting::Off => ortho_half_width.is_some(),
    };
    let pan_width = if settings.pan_width > 0.0 {
        settings.pan_width
    } else {
        ortho_half_width.unwrap_or(DEFAULT_PAN_WIDTH)
    };
    let position = position.to_array();
    let velocity = scene
        .listener
        .map_or([0.0; 3], |last| velocity(last, position, dt));
    scene.listener = Some(position);
    let spatial = &audio.output.layout().spatial;
    let listener = cubed_mixer::Listener {
        position,
        forward: (rotation * Vec3::NEG_Z).to_array(),
        up: (rotation * Vec3::Y).to_array(),
        velocity,
        pan_width,
        speed_of_sound: spatial.speed_of_sound,
        doppler: spatial.doppler,
    };
    audio.output.set_listener(listener);
}

/// Starts the `AudioSource`s that play on start and have not yet.
fn start_sources(audio: &mut Audio, scene: &mut SceneAudio, world: &World) {
    let waiting: Vec<(Entity, AudioSource)> = world
        .query::<&AudioSource>()
        .iter()
        .filter(|(entity, source)| {
            source.play_on_start && !source.clip.is_empty() && !scene.started.contains_key(entity)
        })
        .map(|(entity, source)| (entity, source.clone()))
        .collect();
    for (entity, source) in waiting {
        let position = place(world, entity).map_or([0.0; 3], |(p, _)| p.to_array());
        let options = PlayOptions::from_source(&source, position);
        let spatial = options.spatial.is_some();
        let played = super::resolve_path(&source.clip).and_then(|path| audio.play(path, options));
        match played {
            Ok(id) => {
                scene.started.insert(entity, Some(id));
                if spatial {
                    scene.attach(id, entity, position);
                }
            }
            Err(err) => {
                log::warn!(target: "audio", "AudioSource {}: {err}", source.clip);
                scene.started.insert(entity, None);
            }
        }
    }
}

/// Moves every attached sound to its entity; a sound whose entity is gone
/// stops, and finished sounds are forgotten.
fn follow(audio: &mut Audio, scene: &mut SceneAudio, world: &World, dt: f32) {
    let mut gone = Vec::new();
    for (&id, (entity, last)) in scene.attached.iter_mut() {
        if !audio.output.is_playing(id) {
            gone.push((id, false));
            continue;
        }
        match place(world, *entity) {
            Some((position, _)) => {
                let position = position.to_array();
                let velocity = velocity(*last, position, dt);
                *last = position;
                audio.output.move_voice(id, position, velocity);
            }
            None => gone.push((id, true)),
        }
    }
    for (id, stop) in gone {
        scene.attached.remove(&id);
        if stop {
            audio.output.stop(id);
        }
    }
}

/// A change to the mix: what a volume or a snapshot does.
struct MixChange<'a> {
    buses: &'a [AudioVolumeBus],
    room_size: Option<f32>,
    damping: Option<f32>,
}

impl<'a> MixChange<'a> {
    /// A volume's own changes, or those of the snapshot it names.
    fn of_volume(volume: &'a AudioVolume, layout: &'a MixerLayout) -> Option<MixChange<'a>> {
        match &volume.snapshot {
            Some(name) => layout.snapshot(name).map(MixChange::of_snapshot),
            None => Some(MixChange {
                buses: &volume.buses,
                room_size: volume.reverb_room_size,
                damping: volume.reverb_damping,
            }),
        }
    }

    fn of_snapshot(snapshot: &'a SnapshotLayout) -> MixChange<'a> {
        MixChange {
            buses: &snapshot.buses,
            room_size: snapshot.reverb_room_size,
            damping: snapshot.reverb_damping,
        }
    }

    /// Moves `ambience` (and the reverb's room size and damping) toward this
    /// change by `a`: gains in dB, reverb sends in linear level, low-pass
    /// cutoffs on a log scale.
    fn apply(&self, ambience: &mut Ambience, reverb: &mut Option<[f32; 2]>, a: f32) {
        for change in self.buses {
            let bus = ambience.buses.entry(change.bus.clone()).or_default();
            bus.gain_db += (change.gain_db - bus.gain_db) * a;
            let send = change.reverb_db.map_or(0.0, db_to_gain);
            bus.reverb += (send - bus.reverb) * a;
            let cutoff = change.low_pass_hz.unwrap_or(OPEN_HZ).clamp(20.0, OPEN_HZ);
            bus.low_pass_hz =
                (bus.low_pass_hz.ln() + (cutoff.ln() - bus.low_pass_hz.ln()) * a).exp();
        }
        if let (Some(r), Some(room)) = (reverb.as_mut(), self.room_size) {
            r[0] += (room.clamp(0.0, 1.0) - r[0]) * a;
        }
        if let (Some(r), Some(damping)) = (reverb.as_mut(), self.damping) {
            r[1] += (damping.clamp(0.0, 1.0) - r[1]) * a;
        }
    }
}

/// The mix's changes at `ear`: each enabled audio volume, lowest priority first
/// (infinite before bounded at the same priority), moves the buses it names
/// toward its settings by how much it applies there; then the snapshots, by
/// their weights. The reverb's character starts from the layout's. Also
/// returns each volume's weight.
pub(super) fn ambience(
    world: &World,
    ear: Option<Vec3>,
    layout: &MixerLayout,
    snapshots: &[(String, f32)],
) -> (Ambience, Vec<(Entity, f32)>) {
    let mut volumes: Vec<(Entity, AudioVolume, f32)> = match ear {
        None => Vec::new(),
        Some(ear) => world
            .query::<(
                &AudioVolume,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
            .filter(|(_, (volume, ..))| volume.enabled)
            .filter_map(|(entity, (volume, world_transform, local))| {
                let transform = world_transform
                    .map(|t| t.0.matrix())
                    .or_else(|| local.map(|t| t.0.matrix()))
                    .unwrap_or(Mat4::IDENTITY);
                let amount = crate::scene::shape_influence(
                    &volume.shape,
                    volume.blend_distance,
                    transform,
                    ear,
                );
                (amount > 0.0).then(|| (entity, volume.clone(), amount))
            })
            .collect(),
    };
    let mut ambience = Ambience::default();
    let weights = volumes.iter().map(|(e, _, a)| (*e, *a)).collect();
    if volumes.is_empty() && snapshots.is_empty() {
        return (ambience, weights);
    }
    volumes.sort_by_key(|(_, volume, _)| {
        (
            volume.priority,
            !matches!(volume.shape, VolumeShape::Infinite),
        )
    });
    if let Some((_, top, _)) = volumes.last() {
        if !top.reverb_bus.is_empty() {
            ambience.reverb_bus = top.reverb_bus.clone();
        }
    }
    let base = layout_reverb(layout, &ambience.reverb_bus);
    let mut reverb = base;
    for (_, volume, amount) in &volumes {
        if let Some(change) = MixChange::of_volume(volume, layout) {
            change.apply(&mut ambience, &mut reverb, *amount);
        }
    }
    for (name, weight) in snapshots {
        if let Some(snapshot) = layout.snapshot(name) {
            MixChange::of_snapshot(snapshot).apply(&mut ambience, &mut reverb, *weight);
        }
    }
    // Blending back to neutral leaves rounding behind: snap it, and leave out
    // the buses that end up untouched.
    for bus in ambience.buses.values_mut() {
        if bus.gain_db.abs() < 1e-3 {
            bus.gain_db = 0.0;
        }
        if bus.reverb < 1e-5 {
            bus.reverb = 0.0;
        }
        if bus.low_pass_hz > OPEN_HZ * 0.999 {
            bus.low_pass_hz = OPEN_HZ;
        }
    }
    ambience
        .buses
        .retain(|_, bus| *bus != BusAmbience::default());
    if reverb != base {
        ambience.reverb = reverb;
    }
    (ambience, weights)
}

fn db_to_gain(db: f32) -> f32 {
    if db <= cubed_mixer::MIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// The room size and damping of the named bus's first reverb.
fn layout_reverb(layout: &MixerLayout, bus: &str) -> Option<[f32; 2]> {
    layout.bus(bus)?.effects.iter().find_map(|e| {
        e.effect()
            .filter(|e| e.kind == cubed_mixer::EffectKind::Reverb)
            .map(|e| [e.params[0], e.params[1]])
    })
}

/// The ranges of the selected `AudioSource`s (rings in the three planes at the
/// min distance, bright, and the max distance, faint) and the shapes of the
/// selected `AudioVolume`s.
pub(crate) fn debug_geometry(
    world: &World,
    selected: &[Entity],
) -> Option<crate::renderer::debug_draw::DebugGeometry> {
    let mut geometry = crate::renderer::debug_draw::DebugGeometry::default();
    for &entity in selected {
        volume_outline(&mut geometry, world, entity);
    }
    for &entity in selected {
        let Ok(source) = world.get::<&AudioSource>(entity) else {
            continue;
        };
        if source.spatial == SpatialSetting::Off {
            continue;
        }
        let Some((center, _)) = place(world, entity) else {
            continue;
        };
        for (radius, color) in [
            (source.min_distance, [0.95, 0.85, 0.35, 0.9]),
            (source.max_distance, [0.95, 0.85, 0.35, 0.35]),
        ] {
            for (u, v) in [(Vec3::X, Vec3::Z), (Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z)] {
                ring(&mut geometry, center, u, v, radius, color);
            }
        }
    }
    (!geometry.is_empty()).then_some(geometry)
}

/// A selected audio volume's box or sphere, in its entity's space.
fn volume_outline(
    geometry: &mut crate::renderer::debug_draw::DebugGeometry,
    world: &World,
    entity: Entity,
) {
    let Ok(volume) = world.get::<&AudioVolume>(entity) else {
        return;
    };
    let Ok(transform) = world.get::<&WorldTransform>(entity) else {
        return;
    };
    let matrix = transform.0.matrix();
    let color = [0.4, 0.75, 1.0, 0.85];
    match volume.shape {
        VolumeShape::Infinite => {}
        VolumeShape::Box { size } => {
            let half = Vec3::from(size) * 0.5;
            let corner = |i: usize| {
                let sign = |bit: usize| if i & bit != 0 { 1.0 } else { -1.0 };
                matrix.transform_point3(half * Vec3::new(sign(1), sign(2), sign(4)))
            };
            for i in 0..8 {
                for bit in [1, 2, 4] {
                    if i & bit == 0 {
                        geometry.line(corner(i), corner(i | bit), 1.5, color);
                    }
                }
            }
        }
        VolumeShape::Sphere { radius } => {
            let (scale, _, center) = matrix.to_scale_rotation_translation();
            let radius = radius.abs() * scale.abs().max_element();
            for (u, v) in [(Vec3::X, Vec3::Z), (Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z)] {
                ring(geometry, center, u, v, radius, color);
            }
        }
    }
}

fn ring(
    geometry: &mut crate::renderer::debug_draw::DebugGeometry,
    center: Vec3,
    u: Vec3,
    v: Vec3,
    radius: f32,
    color: [f32; 4],
) {
    const SEGMENTS: usize = 48;
    let point = |i: usize| {
        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        center + (u * angle.cos() + v * angle.sin()) * radius
    };
    for i in 0..SEGMENTS {
        geometry.line(point(i), point(i + 1), 1.5, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_blend_by_distance_and_priority() {
        use super::super::components::AudioVolumeBus;
        let layout = MixerLayout::default();
        let mut world = World::new();
        let cave = AudioVolume {
            shape: VolumeShape::Sphere { radius: 5.0 },
            blend_distance: 4.0,
            reverb_room_size: Some(0.95),
            buses: vec![AudioVolumeBus {
                bus: "sfx".into(),
                gain_db: -6.0,
                reverb_db: Some(0.0),
                low_pass_hz: Some(1_000.0),
            }],
            ..AudioVolume::default()
        };
        world.spawn((cave,));

        let (inside, weights) = ambience(&world, Some(Vec3::ZERO), &layout, &[]);
        assert_eq!(weights.len(), 1);
        assert_eq!(weights[0].1, 1.0);
        let sfx = inside.buses["sfx"];
        assert!((sfx.gain_db + 6.0).abs() < 1e-4 && (sfx.reverb - 1.0).abs() < 1e-4);
        assert!((sfx.low_pass_hz - 1_000.0).abs() < 1.0);
        assert!((inside.reverb.unwrap()[0] - 0.95).abs() < 1e-4);

        // Halfway through the blend distance: half of it, the cutoff on a log scale.
        let (edge, _) = ambience(&world, Some(Vec3::new(7.0, 0.0, 0.0)), &layout, &[]);
        let sfx = edge.buses["sfx"];
        assert!((sfx.gain_db + 3.0).abs() < 1e-3 && (sfx.reverb - 0.5).abs() < 1e-3);
        assert!((sfx.low_pass_hz - (1_000f32 * 20_000.0).sqrt()).abs() < 5.0);

        assert_eq!(
            ambience(&world, Some(Vec3::new(20.0, 0.0, 0.0)), &layout, &[]).0,
            Ambience::default()
        );

        // A dry room inside the cave, at a higher priority, takes the reverb away.
        let room = AudioVolume {
            shape: VolumeShape::Box {
                size: [2.0, 2.0, 2.0],
            },
            blend_distance: 0.0,
            priority: 1,
            reverb_room_size: None,
            buses: vec![AudioVolumeBus {
                bus: "sfx".into(),
                ..AudioVolumeBus::default()
            }],
            ..AudioVolume::default()
        };
        world.spawn((room,));
        let (dry, _) = ambience(&world, Some(Vec3::ZERO), &layout, &[]);
        assert!(!dry.buses.contains_key("sfx"), "{:?}", dry.buses);
    }

    #[test]
    fn snapshots_fade_in_and_out_over_the_volumes() {
        use super::super::components::AudioVolumeBus;
        let mut layout = MixerLayout::default();
        layout.snapshots.push(SnapshotLayout {
            name: "paused".into(),
            buses: vec![AudioVolumeBus {
                bus: "music".into(),
                gain_db: -12.0,
                low_pass_hz: Some(500.0),
                ..AudioVolumeBus::default()
            }],
            ..SnapshotLayout::default()
        });
        let world = World::new();
        let mut scene = SceneAudio::default();
        scene.set_snapshot(Some("paused"), 1.0);
        scene.advance_snapshots(0.5);
        let (half, _) = ambience(&world, None, &layout, &scene.snapshot_weights());
        assert!((half.buses["music"].gain_db + 6.0).abs() < 1e-4);
        scene.advance_snapshots(0.6);
        let (full, _) = ambience(&world, None, &layout, &scene.snapshot_weights());
        assert!((full.buses["music"].gain_db + 12.0).abs() < 1e-4);
        assert_eq!(scene.snapshot(), Some("paused"));

        scene.set_snapshot(None, 0.0);
        scene.advance_snapshots(0.01);
        assert!(scene.snapshot_weights().is_empty());
        assert!(ambience(&world, None, &layout, &[]).0.buses.is_empty());
        // An unknown snapshot changes nothing.
        assert!(ambience(&world, None, &layout, &[("nope".into(), 1.0)])
            .0
            .buses
            .is_empty());
    }

    #[test]
    fn a_teleport_has_no_velocity() {
        assert_eq!(velocity([0.0; 3], [1.0, 0.0, 0.0], 0.1), [10.0, 0.0, 0.0]);
        assert_eq!(velocity([0.0; 3], [100.0, 0.0, 0.0], 0.1), [0.0; 3]);
        assert_eq!(velocity([0.0; 3], [1.0, 0.0, 0.0], 0.0), [0.0; 3]);
    }
}
