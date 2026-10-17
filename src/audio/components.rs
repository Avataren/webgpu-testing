//! Scene components for sound: an [`AudioSource`] plays a sound from its
//! entity, an [`AudioListener`] is where the mix is heard from (the active
//! camera is, when no entity has one), and an [`AudioVolume`] changes the mix
//! while the listener is in it.

use serde::{Deserialize, Serialize};

use crate::scene::components::VolumeShape;

/// How a sound is placed in the world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpatialSetting {
    /// Heard as it is, wherever its entity is (music, UI, ambience beds).
    #[serde(rename = "off")]
    Off,
    /// 2D when the listener's camera is orthographic, 3D when it is
    /// perspective.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// In the view plane: the depth along the camera's view does not count,
    /// and the pan follows the offset across the screen (side-scrollers,
    /// top-down games).
    #[serde(rename = "2d")]
    Planar,
    /// Full 3D: distance in space, panning by direction.
    #[serde(rename = "3d")]
    Full,
}

/// How a sound fades beyond its `min_distance`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloffSetting {
    /// `min_distance / distance`, as sound falls off in air, down to the level
    /// at `max_distance`.
    #[default]
    Inverse,
    /// Straight down to silence at `max_distance`.
    Linear,
}

/// Plays a sound from its entity: positioned there (unless `spatial` is
/// `off`) and following it as it moves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSource {
    /// The sound file (wav, ogg, mp3 or flac), relative to the project.
    pub clip: String,
    /// The mixer bus it plays on.
    pub bus: String,
    /// Linear gain; 1 is the file's own level.
    pub volume: f32,
    /// Playback speed, which also shifts the pitch; 1 is unchanged.
    pub pitch: f32,
    #[serde(rename = "loop")]
    pub looped: bool,
    /// Start when play mode starts (or when the entity appears during play).
    pub play_on_start: bool,
    pub spatial: SpatialSetting,
    /// Full volume within this distance (world units).
    pub min_distance: f32,
    pub max_distance: f32,
    pub rolloff: RolloffSetting,
    /// Scales the doppler shift; 0 turns it off for this source.
    pub doppler: f32,
    /// Who loses when a bus runs out of voices: lower priorities first.
    pub priority: u8,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self {
            clip: String::new(),
            bus: super::DEFAULT_BUS.to_string(),
            volume: 1.0,
            pitch: 1.0,
            looped: false,
            play_on_start: true,
            spatial: SpatialSetting::Auto,
            min_distance: 1.0,
            max_distance: 50.0,
            rolloff: RolloffSetting::Inverse,
            doppler: 1.0,
            priority: 128,
        }
    }
}

/// Where the mix is heard from, instead of the active camera (a character's
/// head in a third-person game, for example).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioListener {
    /// 2D or 3D for sounds set to `auto`. `auto` follows a camera on this
    /// entity (orthographic: 2D), else 3D.
    pub mode: SpatialSetting,
    /// The offset along the listener's right that pans a 2D sound fully to
    /// one side. 0 is half an orthographic camera's view width (10 units
    /// without one).
    pub pan_width: f32,
}

impl Default for AudioListener {
    fn default() -> Self {
        Self {
            mode: SpatialSetting::Auto,
            pan_width: 0.0,
        }
    }
}

/// Changes the mix while the listener is inside it: a cave's reverb, the
/// muffled sound of a room behind a wall, quieter music indoors. Overlapping
/// volumes blend in order of priority, as post-process volumes do: a bounded
/// one applies fully inside its shape and fades out over `blend_distance`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioVolume {
    pub enabled: bool,
    pub shape: VolumeShape,
    /// Higher-priority volumes blend over lower ones.
    pub priority: i32,
    /// Distance outside a bounded shape over which the volume fades out.
    pub blend_distance: f32,
    /// The bus the buses' reverb sends go to (the default layout's reverb
    /// return).
    pub reverb_bus: String,
    /// The reverb's room size (0-1, longer as it grows) in here; `None` keeps
    /// the layout's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_room_size: Option<f32>,
    /// How dull the reverb's tail is (0-1); `None` keeps the layout's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_damping: Option<f32>,
    /// What the volume does to each bus it names; the others are left as
    /// they are.
    pub buses: Vec<AudioVolumeBus>,
    /// A snapshot of the project's mixer to apply instead of `buses` and the
    /// reverb settings, so several volumes share one sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

impl Default for AudioVolume {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: VolumeShape::Box {
                size: [10.0, 4.0, 10.0],
            },
            priority: 0,
            blend_distance: 2.0,
            reverb_bus: super::layout::REVERB_BUS.to_string(),
            reverb_room_size: Some(0.85),
            reverb_damping: None,
            buses: vec![AudioVolumeBus {
                bus: super::DEFAULT_BUS.to_string(),
                reverb_db: Some(-6.0),
                ..AudioVolumeBus::default()
            }],
            snapshot: None,
        }
    }
}

/// An [`AudioVolume`]'s change to one bus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioVolumeBus {
    pub bus: String,
    /// Added to the bus's fader, in dB.
    pub gain_db: f32,
    /// How much of the bus goes to the reverb, in dB; `None` sends none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_db: Option<f32>,
    /// A low-pass on the bus at this cutoff (Hz); `None` leaves it open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_pass_hz: Option<f32>,
}

impl Default for AudioVolumeBus {
    fn default() -> Self {
        Self {
            bus: super::DEFAULT_BUS.to_string(),
            gain_db: 0.0,
            reverb_db: None,
            low_pass_hz: None,
        }
    }
}
