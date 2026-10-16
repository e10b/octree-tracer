struct VSIn {
    [[location(0)]] pos: vec2<f32>;
    [[location(1)]] uv: vec2<f32>;
    [[location(2)]] dist: f32;
};

struct VSOut {
    [[builtin(position)]] clip: vec4<f32>;
    [[location(0)]] uv: vec2<f32>;
    [[location(1)]] dist: f32;
};

struct FSOut {
    [[location(0)]] colour: vec4<f32>;
    [[builtin(frag_depth)]] depth: f32;
};

[[group(0), binding(0)]]
var atlas: texture_2d<f32>;
[[group(0), binding(1)]]
var atlas_sampler: sampler;

[[stage(vertex)]]
fn vs_main(in: VSIn) -> VSOut {
    var out: VSOut;
    out.clip = vec4<f32>(in.pos, 0.0, 1.0);
    out.uv = in.uv;
    out.dist = in.dist;
    return out;
}

[[stage(fragment)]]
fn fs_main(in: VSOut) -> FSOut {
    let colour = textureSample(atlas, atlas_sampler, in.uv);
    if (colour.a < 0.5) {
        discard;
    }
    // Match the world pass, which raises the palette colour to 2.2.
    let rgb = pow(colour.rgb, vec3<f32>(2.2));
    let depth = in.dist / (in.dist + 1.0);
    return FSOut(vec4<f32>(rgb, depth), depth);
}
