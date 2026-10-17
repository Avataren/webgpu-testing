//! Scene components for sound: an [`AudioSource`] plays a sound from its
//! entity, and an [`AudioListener`] is where the mix is heard from (the active
//! camera is, when no entity has one).

use serde::{Deserialize, Serialize};

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
