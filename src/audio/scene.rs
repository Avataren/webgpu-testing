//! Sound in a playing scene: the listener follows an `AudioListener` (or the
//! active camera), `AudioSource`s start from their entities, and every sound
//! attached to an entity follows it, with velocities for doppler.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use hecs::{Entity, World};

use super::components::{AudioListener, AudioSource, SpatialSetting};
use super::{Audio, PlayOptions};
use crate::scene::camera::CameraProjection;
use crate::scene::components::{CameraComponent, WorldTransform};

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
}

impl SceneAudio {
    pub(super) fn clear(&mut self) {
        *self = SceneAudio::default();
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

/// The ranges of the selected `AudioSource`s: rings in the three planes at the
/// min distance (bright) and the max distance (faint).
pub(crate) fn debug_geometry(
    world: &World,
    selected: &[Entity],
) -> Option<crate::renderer::debug_draw::DebugGeometry> {
    let mut geometry = crate::renderer::debug_draw::DebugGeometry::default();
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
    fn a_teleport_has_no_velocity() {
        assert_eq!(velocity([0.0; 3], [1.0, 0.0, 0.0], 0.1), [10.0, 0.0, 0.0]);
        assert_eq!(velocity([0.0; 3], [100.0, 0.0, 0.0], 0.1), [0.0; 3]);
        assert_eq!(velocity([0.0; 3], [1.0, 0.0, 0.0], 0.0), [0.0; 3]);
    }
}
