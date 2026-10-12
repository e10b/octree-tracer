use super::*;

pub const MAX_SUBDIVISIONS_PER_FRAME: usize = 1024000;
pub const MAX_UNSUBDIVISIONS_PER_FRAME: usize = 1024000;

pub fn process_subdivision(
    compute: &mut Compute,
    gpu: &Gpu,
    octree: &mut Octree,
    world: &mut World,
) {
    world.loads_this_frame = 0;

    let slice = compute.subdivision_buffer.slice(..);
    let future = slice.map_async(wgpu::MapMode::Read);

    gpu.device.poll(wgpu::Maintain::Wait);

    if let Ok(()) = pollster::block_on(future) {
        let mut data = slice.get_mapped_range_mut();
        let result: &mut [u32] = unsafe { reinterpret::reinterpret_mut_slice(&mut data) };

        // Reset atomic counter
        let len = (result[0] as usize).min(MAX_SUBDIVISIONS_PER_FRAME - 1);
        result[0] = 0;

        // if len > 0 {
        //     println!("Processing {} subdivisions", len);
        // }

        for i in 1..=len {
            let node_index = result[i] as usize;

            if octree.get_node(node_index) < VOXEL_OFFSET {
                println!("Doubleup!");
                continue;
            }

            subdivide_node(octree, world, node_index);
            result[i] = 0;
        }

        drop(data);
        compute.subdivision_buffer.unmap();
    } else {
        panic!("Failed to run get subdivision buffer!")
    }
}

pub fn process_unsubdivision(
    compute: &mut Compute,
    gpu: &Gpu,
    octree: &mut Octree,
    world: &mut World,
) {
    let slice = compute.unsubdivision_buffer.slice(..);
    let future = slice.map_async(wgpu::MapMode::Read);

    gpu.device.poll(wgpu::Maintain::Wait);

    if let Ok(()) = pollster::block_on(future) {
        let mut data = slice.get_mapped_range_mut();
        let result: &mut [u32] = unsafe { reinterpret::reinterpret_mut_slice(&mut data) };

        // Reset atomic counter
        let len = (result[0] as usize).min(MAX_UNSUBDIVISIONS_PER_FRAME - 1);
        result[0] = 0;

        // if len > 0 {
        //     println!("Processing {} unsubdivisions", len);
        // }

        for i in 1..=len {
            let node_index = result[i] as usize;
            octree.unsubdivide(node_index);

            let pos = octree.positions[node_index];
            let (_, voxel_depth, _) = octree.find_voxel(pos, None);
            let value = if let Some((cpu_chunk, cpu_index, _, _)) =
                world.find_voxel(pos, Some(voxel_depth))
            {
                if let Some(tnipt) = world.chunks.get(&cpu_chunk).map(|c| c.nodes[cpu_index]) {
                    if tnipt.pointer > CHUNK_OFFSET {
                        let chunk = tnipt.pointer - CHUNK_OFFSET;
                        if chunk >= CHUNK_OFFSET / 2 {
                            world.chunks.remove(&chunk);
                        }
                    }
                    tnipt.value
                } else {
                    Voxel::new(128, 128, 128)
                }
            } else {
                Voxel::new(128, 128, 128)
            };

            octree.nodes[node_index] = value.to_value();

            result[i] = 0;
        }

        drop(data);
        compute.unsubdivision_buffer.unmap();
    } else {
        panic!("failed to run compute on gpu!")
    }
}

fn subdivide_node(octree: &mut Octree, world: &mut World, node_index: usize) -> bool {
    if octree.get_node(node_index) < VOXEL_OFFSET {
        return false;
    }

    let pos = octree.positions[node_index];
    let (_, voxel_depth, _) = octree.find_voxel(pos, None);
    let Some((cpu_chunk, cpu_index, _, _)) = world.find_voxel(pos, Some(voxel_depth)) else {
        return false;
    };
    let tnipt = match world.chunks.get(&cpu_chunk) {
        Some(chunk) => chunk.nodes[cpu_index],
        None => return false,
    };

    if tnipt.pointer < CHUNK_OFFSET {
        let mask = world
            .chunks
            .get(&cpu_chunk)
            .unwrap()
            .get_node_mask(tnipt.pointer as usize);
        octree.subdivide(node_index, mask, voxel_depth + 1);
        true
    } else if tnipt.pointer > CHUNK_OFFSET {
        let chunk_id = tnipt.pointer - CHUNK_OFFSET;
        if !world.chunks.contains_key(&chunk_id) && !world.load_chunk(chunk_id) {
            return false;
        }
        let mask = world.chunks.get(&chunk_id).unwrap().get_node_mask(0);
        octree.subdivide(node_index, mask, voxel_depth + 1);
        true
    } else {
        false
    }
}

/// Open the octree around `focus` so the first frame is terrain rather than one flat cube.
pub fn prime_view(octree: &mut Octree, world: &mut World, focus: Vector3<f32>, max_depth: u32) {
    let saved_budget = world.load_budget;
    world.load_budget = usize::MAX;
    world.loads_this_frame = 0;

    let mut i = 0;
    while i < octree.nodes.len() {
        if octree.nodes.len() > 2_000_000 {
            break;
        }
        if octree.get_node(i) >= VOXEL_OFFSET {
            let pos = octree.positions[i];
            let (_, depth, _) = octree.find_voxel(pos, None);
            let cell = 2.0 / (1u32 << depth) as f32;
            let near = (pos - focus).magnitude() <= cell * 1.5;
            if depth < max_depth && (near || depth <= 2) {
                subdivide_node(octree, world, i);
            }
        }
        i += 1;
    }

    world.load_budget = saved_budget;
    world.loads_this_frame = 0;
}
