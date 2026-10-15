// Per-instance data and this frame's materials (renderer/internal/buffers.rs,
// DynamicObjectsBuffer): bind group 1. Needs the material module (MaterialData).
struct Object {
    model: mat4x4<f32>,
    material_index: u32,
    // 64-bit pick identifier split across two u32 values.
    pick_id: array<u32, 2u>,
    _padding: u32,
    // Keeps the std430 stride at ObjectData's 96 bytes.
    _padding2: array<u32, 4>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;
@group(1) @binding(1) var<storage, read> materials: array<MaterialData>;
