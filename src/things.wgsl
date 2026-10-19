struct ThingVert {
    [[location(0)]] pos: vec3<f32>;
    [[location(1)]] uv: vec2<f32>;
    [[location(2)]] atlas: vec4<f32>;
    [[location(3)]] tint: vec3<f32>;
    [[location(4)]] light: f32;
};

struct Uniforms {
    camera: mat4x4<f32>;
    eye: vec4<f32>;
};

[[group(0), binding(0)]]
var<uniform> u: Uniforms;

[[group(0), binding(1)]]
var atlas: texture_2d<f32>;

[[group(0), binding(2)]]
var atlas_sampler: sampler;

struct VSOut {
    [[builtin(position)]] clip: vec4<f32>;
    [[location(0)]] uv: vec2<f32>;
    [[location(1)]] atlas: vec4<f32>;
    [[location(2)]] tint: vec3<f32>;
    [[location(3)]] light: f32;
    [[location(4)]] world: vec3<f32>;
};

[[stage(vertex)]]
fn vs_main(in: ThingVert) -> VSOut {
    var out: VSOut;
    // The raytracer's matrix keeps depth out of w. Divide x and y by that
    // depth so a monster lands on the same pixel as the ray through it.
    let clip = u.camera * vec4<f32>(in.pos, 1.0);
    let dist = length(in.pos - u.eye.xyz);
    let depth = dist / (dist + 1.0);
    out.clip = vec4<f32>(clip.x, clip.y, depth * clip.z, clip.z);
    out.uv = in.uv;
    out.atlas = in.atlas;
    out.tint = in.tint;
    out.light = in.light;
    out.world = in.pos;
    return out;
}

struct FSOut {
    [[location(0)]] colour: vec4<f32>;
    [[builtin(frag_depth)]] depth: f32;
};

[[stage(fragment)]]
fn fs_main(in: VSOut) -> FSOut {
    let span = in.atlas.zw - in.atlas.xy;
    let uv = in.atlas.xy + fract(in.uv) * span;
    let texel = textureSample(atlas, atlas_sampler, uv);
    if (texel.a < 0.5) {
        discard;
    }
    let rgb = pow(clamp(texel.rgb * in.tint * max(in.light, 0.04), vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    let dist = length(in.world - u.eye.xyz);
    let depth = dist / (dist + 1.0);
    return FSOut(vec4<f32>(rgb, 1.0), depth);
}
