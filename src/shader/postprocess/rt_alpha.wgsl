// Cut-out surfaces (glTF MASK) for ray queries. Their geometry is built non-opaque
// (raytracing/scene.rs), so a ray query reports its hits as candidates; each query's loop
// confirms the ones where the surface is there:
//
//     loop {
//         if !rayQueryProceed(&query) {
//             break;
//         }
//         if rt_candidate_blocks(rayQueryGetCandidateIntersection(&query)) {
//             rayQueryConfirmIntersection(&query);
//         }
//     }
//
// Needs rt_scene.wgsl, the material module and one of rt_textures_bindless.wgsl /
// rt_textures_none.wgsl (without textures only the material's own alpha counts).

// Whether a candidate hit stops the ray: a cut-out surface where its alpha passes the
// cutoff (particles and anything else non-opaque count as solid).
fn rt_candidate_blocks(hit : RayIntersection) -> bool {
    let instance = rt_instances[hit.instance_custom_data];
    if (instance.flags & RT_INSTANCE_PARTICLES) != 0u {
        return true;
    }
    let material = rt_materials[instance.material_index];
    let cutoff = material_alpha_cutoff(material.material_flags);
    if cutoff <= 0.0 {
        return true;
    }
    var alpha = material.color.a;
    if (material.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        let first = instance.index_base + hit.primitive_index * 3u;
        let a = rt_vertices[instance.vertex_base + rt_indices[first]];
        let b = rt_vertices[instance.vertex_base + rt_indices[first + 1u]];
        let c = rt_vertices[instance.vertex_base + rt_indices[first + 2u]];
        let w = vec3<f32>(1.0 - hit.barycentrics.x - hit.barycentrics.y, hit.barycentrics);
        let uv = vec2<f32>(a.u, a.v) * w.x + vec2<f32>(b.u, b.v) * w.y + vec2<f32>(c.u, c.v) * w.z;
        let nearest = (material.material_flags & FLAG_USE_NEAREST_FILTERING) != 0u;
        // The finest level: thin cut-out detail (leaves, wire) would blur away.
        alpha = alpha * rt_sample_texture(material.base_color_texture, uv, -64.0, nearest).a;
    }
    return alpha >= cutoff;
}
