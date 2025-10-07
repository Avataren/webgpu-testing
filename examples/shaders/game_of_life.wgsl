const WORKGROUP_SIZE_X : u32 = 8u;
const WORKGROUP_SIZE_Y : u32 = 8u;

@group(0) @binding(0)
var state_src : texture_storage_2d<rgba8unorm, read>;

@group(0) @binding(1)
var state_dst : texture_storage_2d<rgba8unorm, write>;

fn load_cell(x: i32, y: i32, width: u32, height: u32) -> f32 {
    // Wrap coordinates to create toroidal topology
    let w = i32(width);
    let h = i32(height);
    
    var wrapped_x = x % w;
    var wrapped_y = y % h;
    
    if (wrapped_x < 0) {
        wrapped_x = wrapped_x + w;
    }
    if (wrapped_y < 0) {
        wrapped_y = wrapped_y + h;
    }
    
    // Convert back to u32 for texture access
    let coords = vec2<u32>(u32(wrapped_x), u32(wrapped_y));
    return textureLoad(state_src, coords).r;
}

@compute @workgroup_size(WORKGROUP_SIZE_X, WORKGROUP_SIZE_Y)
fn main(@builtin(global_invocation_id) global_id : vec3<u32>) {
    let dims = textureDimensions(state_src);
    
    // Bounds check
    if (global_id.x >= dims.x || global_id.y >= dims.y) {
        return;
    }

    let x = i32(global_id.x);
    let y = i32(global_id.y);

    // Count alive neighbors
    var alive_neighbors = 0;
    for (var dy : i32 = -1; dy <= 1; dy++) {
        for (var dx : i32 = -1; dx <= 1; dx++) {
            if (dx == 0 && dy == 0) {
                continue;
            }
            
            if (load_cell(x + dx, y + dy, dims.x, dims.y) > 0.5) {
                alive_neighbors++;
            }
        }
    }

    // Get current cell state
    let current_alive = load_cell(x, y, dims.x, dims.y) > 0.5;

    // Apply Conway's Game of Life rules
    var next_state = 0.0;
    if (alive_neighbors == 3 || (alive_neighbors == 2 && current_alive)) {
        next_state = 1.0;
    }

    // Write result
    let color = vec4<f32>(next_state, next_state, next_state, 1.0);
    textureStore(state_dst, global_id.xy, color);
}