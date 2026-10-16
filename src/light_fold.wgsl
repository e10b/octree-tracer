// pad0 == 0: unload. Downsample the eight children's radiance into the parent.
// pad0 == 1: reload. Seed each child with the parent's radiance and a short
// count, so the coarse light shows immediately and new samples refine it.
// Record layout matches the path tracer: xyz mean, w count.

struct Records {
    data: [[stride(16)]] array<vec4<f32>>;
};

struct U32s {
    data: [[stride(4)]] array<u32>;
};

struct FoldUniforms {
    count: u32;
    capacity: u32;
    pad0: u32;
    pad1: u32;
};

[[group(0), binding(0)]]
var<storage, read_write> light: Records;
[[group(0), binding(1)]]
var<storage, read> pairs: U32s;
[[group(0), binding(2)]]
var<uniform> fold: FoldUniforms;

[[stage(compute), workgroup_size(64)]]
fn main([[builtin(global_invocation_id)]] id: vec3<u32>) {
    if (id.x >= fold.count) {
        return;
    }
    let parent = pairs.data[id.x * 2u];
    let child = pairs.data[id.x * 2u + 1u];
    if (parent >= fold.capacity || child + 7u >= fold.capacity) {
        return;
    }
    if (fold.pad0 == 1u) {
        let rec = light.data[parent];
        var i = 0u;
        if (rec.w <= 0.0) {
            loop {
                if (i >= 8u) { break; }
                light.data[child + i] = vec4<f32>(0.0);
                i = i + 1u;
            }
            return;
        }
        var prior = rec.w;
        if (prior > 8.0) {
            prior = 8.0;
        }
        let seeded = vec4<f32>(rec.xyz, prior);
        loop {
            if (i >= 8u) { break; }
            light.data[child + i] = seeded;
            i = i + 1u;
        }
        return;
    }
    var acc = vec3<f32>(0.0);
    var k = 0u;
    var j = 0u;
    loop {
        if (j >= 8u) { break; }
        let rec = light.data[child + j];
        if (rec.w > 0.0) {
            acc = acc + rec.xyz;
            k = k + 1u;
        }
        j = j + 1u;
    }
    if (k == 0u) {
        return;
    }
    light.data[parent] = vec4<f32>(acc / f32(k), 8.0);
}
