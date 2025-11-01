#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostProcessEffects {
    pub ssao: bool,
    pub bloom: bool,
    pub fxaa: bool,
    pub ssao_settings: SsaoSettings,
    pub bloom_settings: BloomSettings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoSettings {
    pub radius: f32,
    pub bias: f32,
    pub intensity: f32,
    pub power: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    pub threshold: f32,
    pub knee: f32,
    pub scatter: f32,
}

impl Default for PostProcessEffects {
    fn default() -> Self {
        Self {
            ssao: true,
            bloom: true,
            fxaa: true,
            ssao_settings: SsaoSettings::default(),
            bloom_settings: BloomSettings::default(),
        }
    }
}

impl PostProcessEffects {
    pub(crate) fn uniform_components(self) -> [f32; 4] {
        [
            if self.ssao { 1.0 } else { 0.0 },
            if self.bloom { 1.0 } else { 0.0 },
            if self.fxaa { 1.0 } else { 0.0 },
            0.0,
        ]
    }
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self {
            radius: 0.2,
            bias: 0.05,
            intensity: 0.75,
            power: 1.25,
        }
    }
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            threshold: 0.8,
            knee: 0.4,
            scatter: 0.95,
        }
    }
}
