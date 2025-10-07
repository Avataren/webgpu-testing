const WORKGROUP_SIZE_X : u32 = 8u;
const WORKGROUP_SIZE_Y : u32 = 8u;

@group(0) @binding(0)
var<storage, read> state_src : texture_storage_2d<rgba8unorm, read>;

@group(0) @binding(1)
var<storage, write> state_dst : texture_storage_2d<rgba8unorm, write>;

fn wrap_coord(value : i32, max_value : i32) -> i32 {
    var result = value % max_value;
    if (result < 0) {
        result = result + max_value;
    }
    return result;
}

fn load_cell(position : vec2<i32>, dims : vec2<i32>) -> f32 {
    let wrapped = vec2<i32>(
        wrap_coord(position.x, dims.x),
        wrap_coord(position.y, dims.y),
    );
    return textureLoad(state_src, wrapped, 0).r;
}

@compute @workgroup_size(WORKGROUP_SIZE_X, WORKGROUP_SIZE_Y)
fn main(@builtin(global_invocation_id) global_id : vec3<u32>) {
    let dims_u = textureDimensions(state_src);
    if (global_id.x >= dims_u.x || global_id.y >= dims_u.y) {
        return;
    }

    let dims = vec2<i32>(dims_u.xy);
    let coords = vec2<i32>(global_id.xy);

    var alive_neighbors = 0.0;
    for (var offset_y : i32 = -1; offset_y <= 1; offset_y = offset_y + 1) {
        for (var offset_x : i32 = -1; offset_x <= 1; offset_x = offset_x + 1) {
            if (offset_x == 0 && offset_y == 0) {
                continue;
            }

            let neighbor_offset = vec2<i32>(offset_x, offset_y);
            alive_neighbors = alive_neighbors + load_cell(coords + neighbor_offset, dims);
        }
    }

    let current = load_cell(coords, dims);
    var next_state = 0.0;
    if (alive_neighbors == 3.0 || (alive_neighbors == 2.0 && current > 0.0)) {
        next_state = 1.0;
    }

    let color = vec4<f32>(next_state, next_state, next_state, 1.0);
    textureStore(state_dst, coords, color);
}
