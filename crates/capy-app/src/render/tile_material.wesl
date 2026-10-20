// Textured map tiles: one fragment per covered pixel, sampled with the
// nearest filter from layer `in.color.x` of the atlas array.
//
// The atlas holds premultiplied texels in display (gamma) space and the
// camera composites in gamma space (`CompositingSpace::Srgb`), so the colour
// goes out unchanged and the blend state (ONE, ONE_MINUS_SRC_ALPHA) does the
// rest. See `render/atlas.rs`.
import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var atlas: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var atlas_sampler: sampler;

// Level light: the colour multiplier (white = unchanged).
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let layer = i32(in.color.x + 0.5);
    let texel = textureSampleLevel(atlas, atlas_sampler, in.uv, layer, 0.0);
    // Premultiplied texels: scaling the colour channels keeps alpha as is.
    return vec4<f32>(texel.rgb * tint.rgb, texel.a);
}
