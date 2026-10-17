// ============================================================================
// Constants
// ============================================================================

const PI: f32 = 3.14159265359;
const TWO_PI: f32 = 6.28318530718;

const MAX_DIRECTIONAL_LIGHTS: u32 = 4u;
// Shadow map cascades per directional light at most.
const MAX_SHADOW_CASCADES: u32 = 4u;
// Point and spot lights with shadow maps at once (slots of the shadows uniform).
const MAX_SHADOWED_LOCAL_LIGHTS: u32 = 64u;

const MAX_TEXTURES: u32 = 256u;