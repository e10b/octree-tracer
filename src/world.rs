use super::*;
use dashmap::{DashMap, DashSet};
use std::collections::HashMap;
use std::sync::Arc;

pub struct World {
    pub path: String,
    pub chunks: Arc<DashMap<u32, CpuOctree>>,
    pub loading: Arc<DashSet<u32>>,
    /// Terrain sections are generated on demand instead of read from disk.
    pub procedural_chunks: bool,
    pub loads_this_frame: usize,
    pub load_budget: usize,
    /// Chunk coordinate of the octree corner at (-1, -1, -1).
    pub window_origin: (i32, i32, i32),
    block_colors: [Voxel; 9],
    coord_to_id: HashMap<(i32, i32, i32), u32>,
    id_to_coord: HashMap<u32, (i32, i32, i32)>,
    next_stream_id: u32,
}

impl World {
    pub fn bare(path: String) -> Self {
        Self {
            path,
            chunks: Arc::new(DashMap::new()),
            loading: Arc::new(DashSet::new()),
            procedural_chunks: false,
            loads_this_frame: 0,
            load_budget: 8,
            window_origin: (0, 0, 0),
            block_colors: [Voxel::new(0, 0, 0); 9],
            coord_to_id: HashMap::new(),
            id_to_coord: HashMap::new(),
            next_stream_id: STREAM_BASE,
        }
    }

    pub fn new(path: String) -> Self {
        let mut world = Self::bare(path);

        world.chunks.insert(
            1,
            CpuOctree::load_file("blocks/stone.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(1);
        world.chunks.insert(
            2,
            CpuOctree::load_file("blocks/dirt.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(2);
        world.chunks.insert(
            3,
            CpuOctree::load_file("blocks/grass.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(3);
        world.chunks.insert(
            4,
            CpuOctree::load_file("blocks/wood.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(4);
        world.chunks.insert(
            5,
            CpuOctree::load_file("blocks/leaf.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(5);
        world.chunks.insert(
            6,
            CpuOctree::load_file("blocks/slate.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(6);
        world.chunks.insert(
            7,
            CpuOctree::load_file("blocks/crystal.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(7);
        world.chunks.insert(
            8,
            CpuOctree::load_file("blocks/glass.vox".to_string(), 0).unwrap(),
        );
        world.generate_mip_tree(8);

        world
    }

    /// Infinite block world. The raytracer sees one window of sections; the window
    /// recenters when you fly toward its edge, so chunk coordinates never run out.
    pub fn minecraft() -> Self {
        let mut world = Self::new(String::new());
        world.procedural_chunks = true;

        for id in 1..=8 {
            world.block_colors[id as usize] = world.chunks.get(&id).unwrap().top_mip;
        }

        let spawn = spawn_blocks();
        let origin = window_origin_for_chunk(
            (spawn.x).div_euclid(BLOCKS_PER_CHUNK as f32) as i32,
            (spawn.y).div_euclid(BLOCKS_PER_CHUNK as f32) as i32,
            (spawn.z).div_euclid(BLOCKS_PER_CHUNK as f32) as i32,
        );
        world.place_window(origin);
        let sections = world
            .chunks
            .get(&0)
            .unwrap()
            .nodes
            .iter()
            .filter(|n| n.pointer > CHUNK_OFFSET)
            .count();
        println!(
            "Infinite block world: tracing a {WINDOW_CHUNKS}³ chunk window ({sections} sections of {BLOCKS_PER_CHUNK}³ in view, more generate as you fly)"
        );
        world
    }

    pub fn ensure_chunk_id(&mut self, cx: i32, cy: i32, cz: i32) -> u32 {
        if let Some(id) = self.coord_to_id.get(&(cx, cy, cz)) {
            return *id;
        }
        let id = self.next_stream_id;
        self.next_stream_id += 1;
        self.coord_to_id.insert((cx, cy, cz), id);
        self.id_to_coord.insert(id, (cx, cy, cz));
        id
    }

    /// Point the traced cube at chunks starting at `origin`. Section data already
    /// in memory is kept; sections that fell outside the window are dropped.
    pub fn place_window(&mut self, origin: (i32, i32, i32)) {
        self.window_origin = origin;
        let mut root = CpuOctree::new(0);
        let mut keep = Vec::new();
        for ly in 0..WINDOW_CHUNKS {
            for lx in 0..WINDOW_CHUNKS {
                for lz in 0..WINDOW_CHUNKS {
                    let cx = origin.0 + lx;
                    let cy = origin.1 + ly;
                    let cz = origin.2 + lz;
                    let Some(kind) = chunk_kind(cx, cy, cz) else {
                        continue;
                    };
                    let id = self.ensure_chunk_id(cx, cy, cz);
                    keep.push(id);
                    let pos = chunk_center(lx, ly, lz);
                    root.put_in_block(pos, id, WINDOW_DEPTH);
                    let (node, _, _) = root.find_voxel(pos, None);
                    root.nodes[node].value = self.block_colors[kind as usize];
                }
            }
        }

        let keep_coords: Vec<(i32, i32, i32)> = keep
            .iter()
            .filter_map(|id| self.id_to_coord.get(id).copied())
            .collect();
        self.chunks.retain(|k, _| *k < STREAM_BASE || keep.contains(k));
        self.coord_to_id
            .retain(|coord, _| keep_coords.contains(coord));
        self.id_to_coord
            .retain(|_, coord| keep_coords.contains(coord));

        self.chunks.insert(0, root);
        self.generate_mip_tree(0);
    }

    /// If the camera has walked out of the middle of the traced cube, slide the
    /// cube so that world position stays put and new chunks come into range.
    /// Returns the camera position in the new cube.
    pub fn recenter(&mut self, octree_pos: Point3<f32>) -> Option<Point3<f32>> {
        if !self.procedural_chunks {
            return None;
        }
        let blocks = octree_to_blocks(
            Vector3::new(octree_pos.x, octree_pos.y, octree_pos.z),
            self.window_origin,
        );
        let cx = blocks.x.div_euclid(BLOCKS_PER_CHUNK as f32) as i32;
        let cy = blocks.y.div_euclid(BLOCKS_PER_CHUNK as f32) as i32;
        let cz = blocks.z.div_euclid(BLOCKS_PER_CHUNK as f32) as i32;
        let lx = cx - self.window_origin.0;
        let ly = cy - self.window_origin.1;
        let lz = cz - self.window_origin.2;
        let margin = WINDOW_CHUNKS / 4;
        let inside = lx >= margin
            && lx < WINDOW_CHUNKS - margin
            && ly >= margin
            && ly < WINDOW_CHUNKS - margin
            && lz >= margin
            && lz < WINDOW_CHUNKS - margin;
        if inside {
            return None;
        }

        let origin = window_origin_for_chunk(cx, cy, cz);
        self.place_window(origin);
        let shifted = blocks_to_octree(blocks, origin);
        Some(Point3::new(shifted.x, shifted.y, shifted.z))
    }

    pub fn chunk_at_octree_pos(&self, octree_pos: Point3<f32>) -> (i32, i32, i32) {
        let blocks = octree_to_blocks(
            Vector3::new(octree_pos.x, octree_pos.y, octree_pos.z),
            self.window_origin,
        );
        (
            blocks.x.div_euclid(BLOCKS_PER_CHUNK as f32) as i32,
            blocks.y.div_euclid(BLOCKS_PER_CHUNK as f32) as i32,
            blocks.z.div_euclid(BLOCKS_PER_CHUNK as f32) as i32,
        )
    }

    pub fn terrain_chunk_count(&self) -> usize {
        self.chunks.iter().filter(|e| *e.key() >= STREAM_BASE).count()
    }

    pub fn generate_world<S: AsRef<std::ffi::OsStr> + Sized>(
        path: S,
        procedual: &mut Procedural,
        gpu: &Gpu,
    ) -> Result<(), String> {
        // Write chunk to file
        let path = std::path::Path::new(&path);
        if path.exists() {
            if path.file_stem().unwrap() == "tmp" {
                std::fs::remove_dir_all(path).unwrap();
                std::fs::create_dir(path).unwrap();
            } else {
                return Err("File already exists".to_string());
            }
        } else {
            std::fs::create_dir(path).unwrap();
        }

        // let root = procedual.generate_chunk(gpu, Vector3::new(-1.0, -1.0, -1.0), 0);
        let mut root = CpuOctree::new(0);
        let world_depth = 1;

        let world_size = 1 << world_depth;
        let voxel_size = 2.0 / world_size as f32;
        let total_iterations = world_size * world_size * world_size;

        use indicatif::{ProgressBar, ProgressStyle};
        let pb = ProgressBar::new(total_iterations);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] [{wide_bar:.green/blue}] {pos}/{len} chunks generated ({eta_precise})")
                .progress_chars("=> "),
        );
        pb.set_position(0);

        let mut tmp_world = World::new(path.to_str().unwrap().to_string());

        let mut i = 0;
        // let (x, y, z) = (2, 0, 1);
        for x in 0..world_size {
            for y in 0..world_size {
                for z in 0..world_size {
                    let pos = (Vector3::new(x as f32, y as f32, z as f32)) * voxel_size
                        - Vector3::new(1.0, 1.0, 1.0);

                    let index = CHUNK_OFFSET / 2 + i as u32;
                    let chunk = procedual.generate_chunk(gpu, pos, world_depth);
                    if let Some(chunk) = chunk {
                        println!(
                            "({}, {}, {}): {} million",
                            x,
                            y,
                            z,
                            chunk.nodes.len() as f32 / 1000000.0
                        );

                        tmp_world.chunks.insert(index, chunk);
                        tmp_world.generate_mip_tree(index);
                        tmp_world.save_chunk(index);
                        tmp_world.chunks.get_mut(&index).unwrap().nodes = Vec::new(); // To free the ram while keeping the top_mip
                        root.put_in_block(pos, index, world_depth);
                    }

                    i += 1;
                    pb.set_position(i);
                }
            }
        }

        println!();

        tmp_world.chunks.insert(0, root);
        tmp_world.generate_mip_tree(0);
        tmp_world.save_chunk(0);

        Ok(())
    }

    // pub fn save_world<S: AsRef<std::ffi::OsStr> + Sized>(&mut self, path: S) -> Result<(), String> {
    //     // Write chunk to file
    //     let path = std::path::Path::new(&path);
    //     self.path = path.to_str().unwrap().to_string();
    //     if path.exists() {
    //         return Err("File already exists".to_string());
    //     }

    //     std::fs::create_dir(path).unwrap();
    //     for (index, _) in self.chunks {
    //         if index == 0 || index >= CHUNK_OFFSET / 2 {
    //             self.save_chunk(index)
    //         }
    //     }

    //     Ok(())
    // }

    pub fn load_world<S: AsRef<std::ffi::OsStr> + Sized>(path: S) -> Result<Self, String> {
        let path = std::path::Path::new(&path);
        let world = World::new(path.to_str().unwrap().to_string());
        if !path.exists() {
            return Err("File doesn't exist!".to_string());
        }

        let file = std::fs::read(path.join("0.bin")).unwrap();
        let root = unsafe { CpuOctree::from_bin(file) };
        world.chunks.insert(0, root);

        Ok(world)
    }

    pub fn save_chunk(&self, index: u32) {
        let path = self.path.clone() + "/" + &index.to_string() + ".bin";
        let mut file = std::fs::File::create(path).unwrap();
        let chunk = self.chunks.get(&index).unwrap();
        let data = unsafe { chunk.bin() };

        use std::io::Write;
        file.write_all(data).unwrap();
    }

    pub fn load_chunk(&mut self, index: u32) -> bool {
        if self.chunks.contains_key(&index) {
            return true;
        }

        if self.procedural_chunks && index >= STREAM_BASE {
            if self.loads_this_frame >= self.load_budget {
                return false;
            }
            self.loads_this_frame += 1;

            let Some((cx, cy, cz)) = self.id_to_coord.get(&index).copied() else {
                return false;
            };
            let chunk = generate_block_chunk(cx, cy, cz);
            self.chunks.insert(index, chunk);
            self.generate_mip_tree(index);

            let top = self.chunks.get(&index).unwrap().top_mip;
            let lx = cx - self.window_origin.0;
            let ly = cy - self.window_origin.1;
            let lz = cz - self.window_origin.2;
            if top != Voxel::new(0, 0, 0)
                && (0..WINDOW_CHUNKS).contains(&lx)
                && (0..WINDOW_CHUNKS).contains(&ly)
                && (0..WINDOW_CHUNKS).contains(&lz)
            {
                let pos = chunk_center(lx, ly, lz);
                if let Some((0, node, _, _)) = self.find_voxel(pos, Some(WINDOW_DEPTH)) {
                    self.chunks.get_mut(&0).unwrap().nodes[node].value = top;
                }
            }
            return true;
        }

        if self.loading.contains(&index) {
            return false;
        }
        self.loading.insert(index);

        let chunks = self.chunks.clone();
        let loading = self.loading.clone();
        let path = self.path.clone() + "/" + &index.to_string() + ".bin";
        tokio::task::spawn(async move {
            let file = std::fs::read(path).unwrap();
            let root = unsafe { CpuOctree::from_bin(file) };
            chunks.insert(index, root);
            loading.remove(&index);
        });
        false
    }

    /// Returns (chunk, index, depth, pos). None if a chunk on the path is not resident.
    pub fn find_voxel(
        &self,
        pos: Vector3<f32>,
        max_depth: Option<u32>,
    ) -> Option<(u32, usize, u32, Vector3<f32>)> {
        let mut node_index = 0;
        let mut chunk = 0;
        let mut node_pos = Vector3::zero();
        let mut depth = 0;
        loop {
            depth += 1;

            let p = Vector3::new(
                (pos.x >= node_pos.x) as usize,
                (pos.y >= node_pos.y) as usize,
                (pos.z >= node_pos.z) as usize,
            );
            let child_index = p.x * 4 + p.y * 2 + p.z;

            node_pos += Octree::pos_offset(child_index, depth);

            let tnipt = {
                let nodes = self.chunks.get(&chunk)?;
                nodes.nodes.get(node_index + child_index)?.pointer
            };
            if tnipt == CHUNK_OFFSET || depth == max_depth.unwrap_or(u32::MAX) {
                return Some((chunk, node_index + child_index, depth, node_pos));
            } else if tnipt > CHUNK_OFFSET {
                chunk = tnipt - CHUNK_OFFSET;
                node_index = 0;
            } else {
                node_index = tnipt as usize;
            }
        }
    }

    pub fn generate_mip_tree(&mut self, id: u32) {
        // let chunk = self
        //     .chunks
        //     .get_mut(&chunk_id)
        //     .expect("Tried to generate mip tree for chunk that doesn't exist");

        let mut voxels_in_each_level: Vec<Vec<usize>> = Vec::new();
        voxels_in_each_level.push(vec![0]);

        use std::collections::VecDeque;
        let mut queue = VecDeque::new();

        for child_index in 0..8 {
            let node = self.chunks.get(&id).unwrap().nodes[child_index];
            if node.pointer < CHUNK_OFFSET {
                queue.push_back((child_index, 1));
            } else if node.pointer > CHUNK_OFFSET {
                let index = node.pointer - CHUNK_OFFSET;
                if let Some(top_mip) = self.chunks.get(&index).map(|c| c.top_mip) {
                    self.chunks.get_mut(&id).unwrap().nodes[child_index].value = top_mip;
                }
            }
        }

        let mut i = 0;
        while let Some((node_index, depth)) = queue.pop_front() {
            loop {
                if i > 0 && i % 1000000 == 0 {
                    println!("Cataloged {} million nodes", i / 1000000);
                }
                i += 1;

                if let Some(level) = voxels_in_each_level.get_mut(depth as usize) {
                    level.push(node_index);

                    let node = self.chunks.get(&id).unwrap().nodes[node_index as usize];
                    for child_index in 0..8 {
                        let child_node = self.chunks.get(&id).unwrap().nodes
                            [node.pointer as usize + child_index];
                        if child_node.pointer < CHUNK_OFFSET {
                            queue.push_back((node.pointer as usize + child_index, depth + 1));
                        } else if child_node.pointer > CHUNK_OFFSET {
                            let index = child_node.pointer - CHUNK_OFFSET;
                            if let Some(top_mip) = self.chunks.get(&index).map(|c| c.top_mip) {
                                self.chunks.get_mut(&id).unwrap().nodes
                                    [node.pointer as usize + child_index]
                                    .value = top_mip;
                            }
                        }
                    }

                    break;
                } else {
                    voxels_in_each_level.push(Vec::new());
                }
            }
        }

        // for i in 0..voxels_in_each_level.len() {
        //     println!("Level {}: ({})", i, voxels_in_each_level[i].len());
        //     for j in 0..voxels_in_each_level[i].len() {
        //         println!("  {}", voxels_in_each_level[i][j]);
        //     }
        // }

        for i in (0..voxels_in_each_level.len()).rev() {
            for node_index in &voxels_in_each_level[i] {
                // Average the colours of the 8 children
                let node = if i != 0 {
                    self.chunks.get(&id).unwrap().nodes[*node_index as usize]
                } else {
                    Node::new(0, Voxel::new(0, 0, 0))
                };
                let mut colour = Vector3::new(0.0, 0.0, 0.0);
                let mut divisor = 0.0;
                let mut emission_levels = 0u32;
                let mut has_authored_emission = false;
                let mut roughness = 0.0;
                let mut metal_count = 0.0;
                let mut normal_sum = [0.0f32; 3];
                let mut normal_count = 0.0f32;

                for i in 0..8 {
                    let child = self.chunks.get(&id).unwrap().nodes[node.pointer as usize + i];
                    if child.value != Voxel::new(0, 0, 0) {
                        let voxel = child.value;
                        colour += Vector3::new(voxel.r as f32, voxel.g as f32, voxel.b as f32);
                        if voxel.emission > 0 {
                            has_authored_emission = true;
                            emission_levels += voxel.emission.saturating_sub(1) as u32;
                        }
                        roughness += voxel.roughness as f32;
                        if voxel.metal > 0 {
                            metal_count += 1.0;
                        }
                        if let Some(normal) = crate::octree::unpack_oct_normal(voxel.normal) {
                            normal_sum[0] += normal[0];
                            normal_sum[1] += normal[1];
                            normal_sum[2] += normal[2];
                            normal_count += 1.0;
                        }
                        divisor += 1.0;
                    }
                }

                if divisor == 0.0 {
                    continue;
                }
                colour /= divisor;
                // A mip voxel covers all eight child cells. Average emissive energy over
                // that full footprint so a small light does not illuminate an entire
                // coarse octree brick when its leaves are not resident.
                let emission = if has_authored_emission {
                    1 + ((emission_levels / 8) as u8).min(2)
                } else {
                    0
                };

                let roughness = if roughness == 0.0 {
                    0
                } else {
                    (roughness / divisor).round().clamp(1.0, 3.0) as u8
                };
                let metal = if metal_count * 2.0 >= divisor { 1 } else { 0 };
                let normal = if normal_count == 0.0 {
                    0
                } else {
                    crate::octree::pack_oct_normal([
                        normal_sum[0] / normal_count,
                        normal_sum[1] / normal_count,
                        normal_sum[2] / normal_count,
                    ])
                };
                let voxel = Voxel::surface(
                    (colour.x as u8).max(1),
                    (colour.y as u8).max(1),
                    (colour.z as u8).max(1),
                    emission,
                    roughness,
                    metal,
                    normal,
                );

                if i != 0 {
                    self.chunks.get_mut(&id).unwrap().nodes[*node_index as usize].value = voxel;
                } else {
                    self.chunks.get_mut(&id).unwrap().top_mip = voxel;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_ray_reaches_the_block_model() {
        let mut world = World::minecraft();

        let mask = world.chunks.get(&0).unwrap().get_node_mask(0);
        assert!(mask.iter().any(|v| *v != Voxel::new(0, 0, 0)));
        let mut octree = Octree::new(mask);
        let (spawn, _) = player_start(world.window_origin);
        prime_view(
            &mut octree,
            &mut world,
            Vector3::new(spawn.x, spawn.y, spawn.z),
            6,
        );
        assert!(octree.nodes.len() > 8);
        assert!(world.terrain_chunk_count() > 0);

        let check = |world: &mut World, x: i32, z: i32| {
            let h = terrain_height(x, z);
            let origin = window_origin_for_chunk(
                chunk_of_block(x),
                chunk_of_block(h),
                chunk_of_block(z),
            );
            world.place_window(origin);
            for (wy, expect) in [(h, GRASS), (h - 1, DIRT), (h - 5, STONE)] {
                let pos = block_window_pos(x, wy, z, origin);
                let section = world.ensure_chunk_id(
                    chunk_of_block(x),
                    chunk_of_block(wy),
                    chunk_of_block(z),
                );

                let (chunk, index, depth, _) = world.find_voxel(pos, Some(WINDOW_DEPTH)).unwrap();
                assert_eq!(chunk, 0, "section pointer lives on the root");
                assert_eq!(depth, WINDOW_DEPTH);
                let pointer = world.chunks.get(&0).unwrap().nodes[index].pointer;
                assert_eq!(pointer - CHUNK_OFFSET, section);

                assert!(world.load_chunk(section));
                let (chunk, index, depth, _) = world
                    .find_voxel(pos, Some(WINDOW_DEPTH + CHUNK_DEPTH))
                    .unwrap();
                assert_eq!(depth, WINDOW_DEPTH + CHUNK_DEPTH);
                let node = world.chunks.get(&chunk).unwrap().nodes[index];
                assert_eq!(node.pointer, CHUNK_OFFSET + expect, "({x}, {wy}, {z})");
                assert_ne!(node.value, Voxel::new(0, 0, 0));

                let (model, index, depth, _) = world.find_voxel(pos, None).unwrap();
                assert_eq!(model, expect);
                assert!(depth > WINDOW_DEPTH + CHUNK_DEPTH);
                let leaf = world.chunks.get(&model).unwrap().nodes[index];
                assert_eq!(leaf.pointer, CHUNK_OFFSET);
            }
        };

        check(&mut world, 128, 90);
        check(&mut world, -80, 40);
        check(&mut world, 0, 0);
    }

    #[test]
    fn sliding_the_window_keeps_the_same_world_position() {
        let mut world = World::minecraft();
        let blocks = spawn_blocks() + Vector3::new(48.0, 0.0, -32.0);
        let octree_pos = blocks_to_octree(blocks, world.window_origin);
        let shifted = world
            .recenter(Point3::new(octree_pos.x, octree_pos.y, octree_pos.z))
            .expect("flying two chunks over should slide the window");
        let back = octree_to_blocks(
            Vector3::new(shifted.x, shifted.y, shifted.z),
            world.window_origin,
        );
        assert!((back.x - blocks.x).abs() < 0.01);
        assert!((back.y - blocks.y).abs() < 0.01);
        assert!((back.z - blocks.z).abs() < 0.01);

        let h = terrain_height(blocks.x as i32, blocks.z as i32);
        let pos = block_window_pos(blocks.x as i32, h, blocks.z as i32, world.window_origin);
        let (chunk, _, depth, _) = world.find_voxel(pos, Some(WINDOW_DEPTH)).unwrap();
        assert_eq!(chunk, 0);
        assert_eq!(depth, WINDOW_DEPTH);
    }
}
