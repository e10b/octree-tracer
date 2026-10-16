struct VSOut {
    [[builtin(position)]] clip: vec4<f32>;
    [[location(0)]] uv: vec2<f32>;
};

[[group(0), binding(0)]]
var scene: texture_2d<f32>;
[[group(0), binding(1)]]
var scene_sampler: sampler;

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
    let colour_hdr = textureSample(scene, scene_sampler, in.uv).rgb;
    let exposed = vec3<f32>(1.0) - exp(-colour_hdr * 0.8);
    let colour = pow(clamp(exposed, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    return vec4<f32>(colour, 1.0);
}
