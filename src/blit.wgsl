struct VSOut {
    [[builtin(position)]] clip: vec4<f32>;
    [[location(0)]] uv: vec2<f32>;
};

[[group(0), binding(0)]]
var scene: texture_2d<f32>;

[[stage(vertex)]]
fn vs_main([[builtin(vertex_index)]] vertex: u32) -> VSOut {
    var out: VSOut;
    if (vertex == 0u) {
        out.clip = vec4<f32>(-1.0, -1.0, 0.0, 1.0);
        out.uv = vec2<f32>(0.0, 1.0);
    } else if (vertex == 1u) {
        out.clip = vec4<f32>(1.0, -1.0, 0.0, 1.0);
        out.uv = vec2<f32>(1.0, 1.0);
    } else if (vertex == 2u) {
        out.clip = vec4<f32>(-1.0, 1.0, 0.0, 1.0);
        out.uv = vec2<f32>(0.0, 0.0);
    } else {
        out.clip = vec4<f32>(1.0, 1.0, 0.0, 1.0);
        out.uv = vec2<f32>(1.0, 0.0);
    }
    return out;
}

[[stage(fragment)]]
fn fs_main(in: VSOut) -> [[location(0)]] vec4<f32> {
    let dims = vec2<i32>(textureDimensions(scene));
    let coord = clamp(vec2<i32>(in.uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    let colour_hdr = textureLoad(scene, coord, 0).rgb;
    let exposed = vec3<f32>(1.0) - exp(-colour_hdr * 0.8);
    let colour = pow(clamp(exposed, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    return vec4<f32>(colour, 1.0);
}
