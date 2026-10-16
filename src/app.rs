use super::*;
use winit::window::Window;

pub struct App {
    pub octree: Octree,
    pub world: World,
    pub gen_settings: GenSettings,
    pub gpu: Gpu,
    pub render: Render,
    pub compute: Compute,
    pub procedural: Procedural,
    pub input: Input,
    pub character: Character,
    pub settings: Settings,
    walk: bool,
    grid: i32,
    vy: f32,
    on_ground: bool,
    jump_held: bool,
    use_held: bool,
    last_time: f64,
    play: Play,
    ui: Ui,
}

impl App {
    pub async fn new(window: &Window) -> Self {
        let input = Input::new();
        let mut character = Character::new();

        let settings = Settings {
            octree_depth: 12,
            fov: 90.0,
            sensitivity: 0.00005,
        };

        let gpu = Gpu::new(window).await;
        let procedural = Procedural::new(&gpu);

        let map = load_e1m1();
        let walk = map.walk;
        let grid = map.grid;
        let mut world = World::bare(String::new());
        world.chunks.insert(0, map.chunk);
        world.generate_mip_tree(0);
        character.pos = map.spawn;
        character.look = map.look;
        character.speed = -5.0;

        let gen_settings = GenSettings::default();
        // let cpu_octree = generate_world(&gen_settings, &blocks).unwrap();
        // let defualt_octree = CpuOctree::new(0b01011011);
        // let cpu_octree = match CpuOctree::load_file(octree_path, octree_depth, Some(&blocks)) {
        //     Ok(cpu_octree) => cpu_octree,
        //     Err(_) => defualt_octree,
        // };

        // let octree = Octree::new([Voxel::new(255, 255, 255); 8]);
        let mask = world.chunks.get(&0).unwrap().get_node_mask(0);
        let mut octree = Octree::new(mask);
        prime_view(
            &mut octree,
            &mut world,
            Vector3::new(character.pos.x, character.pos.y, character.pos.z),
            6,
        );

        let mut render = Render::new(&gpu, window, &octree).await;
        let play = Play::new(map.pictures);
        render.set_atlas(&gpu, &play.pictures.atlas, play.pictures.atlas_size);
        let compute = Compute::new(&gpu, &render);

        let app = Self {
            octree,
            world,
            gen_settings,
            gpu,
            render,
            compute,
            procedural,
            input,
            character,
            settings,
            walk,
            grid,
            vy: 0.0,
            on_ground: true,
            jump_held: false,
            use_held: false,
            last_time: 0.0,
            play,
            ui: Default::default(),
        };

        app
    }

    pub fn update(&mut self, time: f64) {
        self.gui(time);

        let dt = (time - self.last_time) as f32;
        let dt = dt.clamp(0.0, 0.05);
        self.last_time = time;

        let forward: Vector3<f32> = self.character.look.normalize();
        let right = forward.cross(Vector3::unit_y()).normalize();
        let up = right.cross(forward);

        if self.walk {
            let mut flat = Vector3::new(forward.x, 0.0, forward.z);
            if flat.magnitude2() < 1.0e-8 {
                flat = Vector3::new(0.0, 0.0, 1.0);
            }
            flat = flat.normalize();
            let strafe = flat.cross(Vector3::unit_y());
            let wish_x = self.input.right as u32 as f32 - self.input.left as u32 as f32;
            let wish_z = self.input.forward as u32 as f32 - self.input.backward as u32 as f32;
            let mut wish = flat * wish_z + strafe * wish_x;
            let mag = (wish.x * wish.x + wish.z * wish.z).sqrt();
            if mag > 1.0 {
                wish /= mag;
            }
            // Doom's run settles near 16 map units a tic, about 520 a second.
            // Z keeps the old ten-times boost.
            let speed = if self.input.boost { 5200.0 } else { 520.0 };
            let jump = self.input.up && !self.jump_held;
            self.jump_held = self.input.up;
            let pos = self.character.pos;
            let vy = self.vy;
            let on_ground = self.on_ground;
            let grid = self.grid;
            let stepped = {
                let chunk = self.world.chunks.get(&0).unwrap();
                step_on_foot(&chunk, grid, pos, wish, speed, vy, dt, jump, on_ground)
            };
            let landed = stepped.0;
            if self.play.door_blocks(landed, grid) && !self.play.door_blocks(pos, grid) {
                let slide_x = Point3::new(landed.x, landed.y, pos.z);
                let slide_z = Point3::new(pos.x, landed.y, landed.z);
                if !self.play.door_blocks(slide_x, grid) {
                    self.character.pos = slide_x;
                } else if !self.play.door_blocks(slide_z, grid) {
                    self.character.pos = slide_z;
                } else {
                    self.character.pos = Point3::new(pos.x, landed.y, pos.z);
                }
            } else {
                self.character.pos = landed;
            }
            self.vy = stepped.1;
            self.on_ground = stepped.2;
        } else {
            let boost = if self.input.boost { 10.0 } else { 1.0 };
            let input = Vector3::new(
                self.input.right as u32 as f32 - self.input.left as u32 as f32,
                self.input.up as u32 as f32 - self.input.down as u32 as f32,
                self.input.forward as u32 as f32 - self.input.backward as u32 as f32,
            ) * std::f32::consts::E.powf(self.character.speed)
                * boost;
            self.character.pos += forward * input.z + right * input.x + up * input.y;
        }

        if let Some(pos) = self.world.recenter(self.character.pos) {
            self.character.pos = pos;
            let mask = self.world.chunks.get(&0).unwrap().get_node_mask(0);
            self.octree = Octree::new(mask);
            prime_view(
                &mut self.octree,
                &mut self.world,
                Vector3::new(pos.x, pos.y, pos.z),
                6,
            );
            let nodes = self.octree.raw_data();
            self.gpu
                .queue
                .write_buffer(&self.render.node_buffer, 0, bytemuck::cast_slice(&nodes));
        }

        if self.character.cursour_grabbed {
            let delta = self.settings.sensitivity * self.input.mouse_delta * self.settings.fov;
            let rotation = Quaternion::from_axis_angle(right, Rad(-delta.y))
                * Quaternion::from_axis_angle(Vector3::unit_y(), Rad(-delta.x));

            self.input.mouse_delta = Vector2::zero();
            self.character.look = (rotation * self.character.look).normalize();
        }

        let terrain_origin = if self.world.procedural_chunks {
            let origin = self.world.window_origin;
            Some([
                (origin.0 * BLOCKS_PER_CHUNK) as f32,
                (origin.1 * BLOCKS_PER_CHUNK) as f32,
                (origin.2 * BLOCKS_PER_CHUNK) as f32,
            ])
        } else {
            None
        };
        self.render.update(
            &self.gpu,
            time,
            &mut self.settings,
            &self.character,
            terrain_origin,
        );

        if self.walk {
            if self.character.cursour_grabbed && self.input.use_door && !self.use_held {
                let pos = self.character.pos;
                let look = self.character.look;
                let grid = self.grid;
                self.play.use_facing(pos, look, grid);
            }
            self.use_held = self.input.use_door;
            if self.character.cursour_grabbed && self.input.fire {
                let pos = self.character.pos;
                let look = self.character.look;
                let grid = self.grid;
                let chunk = self.world.chunks.get(&0).unwrap();
                self.play.fire(time, pos, look, &chunk, grid);
            }
            let pain = {
                let pos = self.character.pos;
                let grid = self.grid;
                let chunk = self.world.chunks.get(&0).unwrap();
                self.play.tick(dt, pos, &chunk, grid)
            };
            self.play.hurt_player(pain);
            self.play.kick = (self.play.kick - dt * 0.45).max(0.0);
            self.play.take_pickups(self.character.pos, self.grid, time);
            let moving =
                self.input.forward || self.input.backward || self.input.left || self.input.right;
            let (world_quads, gun_quads) = self.play.quads(
                self.character.pos,
                self.character.look,
                self.settings.fov,
                self.render.size.width as f32,
                self.render.size.height as f32,
                time as f32,
                moving,
            );
            self.render.set_sprites(&self.gpu, &world_quads, &gun_quads);
        }

        if !self.render.uniforms.pause_adaptive {
            self.compute.update(&self.gpu, &self.octree);

            process_subdivision(
                &mut self.compute,
                &self.gpu,
                &mut self.octree,
                &mut self.world,
            );
            process_unsubdivision(
                &mut self.compute,
                &self.gpu,
                &mut self.octree,
                &mut self.world,
            );

            // Write octree to gpu
            let nodes = self.octree.raw_data();

            self.gpu
                .queue
                .write_buffer(&self.render.node_buffer, 0, bytemuck::cast_slice(&nodes));
        }
    }

    pub fn gui(&mut self, time: f64) {
        let fps = if let Some(previous_frame_time) = self.render.previous_frame_time {
            let fps = 1.0 / (time - previous_frame_time);
            self.render.previous_frame_time = Some(time);
            fps
        } else {
            self.render.previous_frame_time = Some(time);
            0.0
        };

        let hole_percentage =
            100.0 * (8.0 * self.octree.hole_stack.len() as f32) / self.octree.nodes.len() as f32;

        let ctx = self.render.egui_platform.context();
        if self.walk {
            self.play.hud(&ctx, time);
        }

        egui::Window::new("Info").show(&ctx, |ui| {
            ui.label(format!("FPS: {:.0}", fps));
            egui::CollapsingHeader::new("Render")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Open File").clicked() {
                            let path = native_dialog::FileDialog::new()
                                .add_filter("Magica Voxel RSVO File", &["rsvo"])
                                .add_filter("Magica Voxel Vox File", &["vox"])
                                .show_open_single_file()
                                .unwrap();

                            match path {
                                Some(path) => match CpuOctree::load_file(
                                    path.into_os_string().into_string().unwrap(),
                                    self.settings.octree_depth,
                                ) {
                                    Ok(chunk) => {
                                        self.world.procedural_chunks = false;
                                        self.world.chunks.remove(&0);
                                        self.world.chunks.insert(0, chunk);
                                        self.world.generate_mip_tree(0);

                                        // Reset octree
                                        let mask =
                                            self.world.chunks.get(&0).unwrap().get_node_mask(0);
                                        self.octree = Octree::new(mask);

                                        let nodes = self.octree.raw_data();
                                        self.gpu.queue.write_buffer(
                                            &self.render.node_buffer,
                                            0,
                                            bytemuck::cast_slice(&nodes),
                                        );

                                        self.ui.error_string = "".to_string();
                                    }
                                    Err(e) => {
                                        self.ui.error_string = e;
                                    }
                                },
                                None => self.ui.error_string = "No file selected".to_string(),
                            }
                        }

                        if ui.button("Open World").clicked() {
                            let path = native_dialog::FileDialog::new()
                                .add_filter("Bin in world folder", &["bin"])
                                .show_open_single_file()
                                .unwrap();

                            match path {
                                Some(path) => {
                                    self.world = World::load_world(path.parent().unwrap()).unwrap();

                                    // Reset octree
                                    let mask = self.world.chunks.get(&0).unwrap().get_node_mask(0);
                                    self.octree = Octree::new(mask);

                                    let nodes = self.octree.raw_data();
                                    self.gpu.queue.write_buffer(
                                        &self.render.node_buffer,
                                        0,
                                        bytemuck::cast_slice(&nodes),
                                    );

                                    self.ui.error_string = "".to_string();
                                }
                                None => self.ui.error_string = "No file selected".to_string(),
                            }
                        }

                        // if ui.button("Save File").clicked() {
                        //     let path = native_dialog::FileDialog::new()
                        //         .show_save_single_file()
                        //         .unwrap();

                        //     match path {
                        //         Some(path) => match self.world.save_world(path) {
                        //             Ok(_) => self.ui.error_string = "".to_string(),
                        //             Err(e) => self.ui.error_string = e,
                        //         },
                        //         None => self.ui.error_string = "No file selected".to_string(),
                        //     }
                        // }

                        if ui.button("Regenerate").clicked() {
                            let path = native_dialog::FileDialog::new()
                                .show_save_single_file()
                                .unwrap();

                            // let path = Some(std::path::PathBuf::from("/Users/brian/Documents/Code/Rust/octree-tracer/worlds/tmp"));

                            match path {
                                Some(path) => {
                                    World::generate_world(&path, &mut self.procedural, &self.gpu)
                                        .unwrap();

                                    self.world = World::load_world(path).unwrap();

                                    // Reset octree
                                    let mask = self.world.chunks.get(&0).unwrap().get_node_mask(0);
                                    self.octree = Octree::new(mask);

                                    let nodes = self.octree.raw_data();
                                    self.gpu.queue.write_buffer(
                                        &self.render.node_buffer,
                                        0,
                                        bytemuck::cast_slice(&nodes),
                                    );

                                    self.ui.error_string = "".to_string();
                                }
                                None => self.ui.error_string = "No file selected".to_string(),
                            }
                        }
                    });

                    if self.ui.error_string != "" {
                        ui.colored_label(egui::Color32::RED, &self.ui.error_string);
                    }

                    ui.add(
                        egui::Slider::new(&mut self.settings.octree_depth, 0..=20)
                            .text("Octree depth"),
                    );

                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.render.uniforms.sun_dir[0])
                                .speed(0.05)
                                .prefix("x: "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.render.uniforms.sun_dir[1])
                                .speed(0.05)
                                .prefix("y: "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.render.uniforms.sun_dir[2])
                                .speed(0.05)
                                .prefix("z: "),
                        );
                    });

                    ui.checkbox(&mut self.render.uniforms.show_steps, "Show ray steps");
                    ui.checkbox(&mut self.render.uniforms.show_hits, "Show ray hits");
                    ui.checkbox(&mut self.render.uniforms.shadows, "Sun shadows");
                    ui.add(
                        egui::Slider::new(&mut self.render.path_bounces, 1..=6)
                            .text("Path bounces"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.render.texture_emission, 0.0..=100.0)
                            .text("Emission strength"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.render.temporal_history, 0.0..=0.99)
                            .text("SVGF history"),
                    );
                    ui.label(format!(
                        "Temporal frames: {}",
                        self.render.accumulated_samples()
                    ));
                    ui.checkbox(&mut self.render.uniforms.pause_adaptive, "Pause adaptive");
                    ui.add(
                        egui::Slider::new(&mut self.render.uniforms.misc_value, 0.0000001..=0.0001)
                            .text("Misc")
                            .logarithmic(true),
                    );
                    ui.checkbox(&mut self.render.uniforms.misc_bool, "Misc");

                    ui.label(format!(
                        "Nodes: {:.2} million ({:.0}% holes)",
                        self.octree.nodes.len() as f32 / 1000000.0,
                        hole_percentage,
                    ));
                    if self.world.procedural_chunks {
                        let (cx, cy, cz) = self.world.chunk_at_octree_pos(self.character.pos);
                        ui.label(format!("Chunk: {cx}, {cy}, {cz}"));
                        ui.label(format!(
                            "Sections in memory: {} ({}³ blocks each)",
                            self.world.terrain_chunk_count(),
                            BLOCKS_PER_CHUNK,
                        ));
                    } else {
                        if self.walk {
                            ui.label("E1M1 from WAD  ·  WASD walk, Z run, Space jump");
                        } else {
                            ui.label("Doom E1M1");
                        }
                    }
                });

            egui::CollapsingHeader::new("Character")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.settings.fov, 0.01..=100.0)
                            .prefix("FOV: ")
                            .logarithmic(true),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.settings.sensitivity, 0.00001..=0.0001)
                            .prefix("Sensitivity")
                            .logarithmic(true),
                    );
                });

            // fn update_world_gen(app: &mut App) {
            //     app.world = World::generate_world(&app.procedural, &app.gpu, &app.blocks);;\
            //     app.octree = Octree::new(app.world.get_node_mask(0));
            // }

            // egui::CollapsingHeader::new("World gen")
            //     .default_open(false)
            //     .show(ui, |ui| {
            //         if ui
            //             .add(
            //                 egui::Slider::new(&mut self.procedural.uniforms.misc1, 0.0..=1.0)
            //                     .prefix("Misc1: "),
            //             )
            //             .changed()
            //         {
            //             update_world_gen(self)
            //         }
            //         if ui
            //             .add(
            //                 egui::Slider::new(&mut self.procedural.uniforms.misc2, 0.0..=10.0)
            //                     .prefix("Misc2: "),
            //             )
            //             .changed()
            //         {
            //             update_world_gen(self)
            //         }
            //         if ui
            //             .add(
            //                 egui::Slider::new(&mut self.procedural.uniforms.misc3, 0.0..=16777216.0)
            //                     .prefix("Misc3: "),
            //             )
            //             .changed()
            //         {
            //             update_world_gen(self)
            //         }
            //     });
        });
    }

    pub fn input(&mut self, window: &Window, event: &Event<()>) {
        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::KeyboardInput {
                    input:
                        KeyboardInput {
                            state,
                            virtual_keycode,
                            ..
                        },
                    ..
                } => match virtual_keycode {
                    Some(VirtualKeyCode::W) => {
                        self.input.forward = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::S) => {
                        self.input.backward = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::D) => {
                        self.input.right = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::A) => {
                        self.input.left = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::Space) => {
                        self.input.up = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::LShift) => {
                        self.input.down = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::Z) => {
                        self.input.boost = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::E) => {
                        self.input.use_door = *state == ElementState::Pressed;
                    }
                    Some(VirtualKeyCode::Escape) => {
                        if *state == ElementState::Pressed {
                            window.set_cursor_visible(self.character.cursour_grabbed);
                            self.character.cursour_grabbed = !self.character.cursour_grabbed;
                            window
                                .set_cursor_grab(self.character.cursour_grabbed)
                                .unwrap();
                        }
                    }
                    _ => {}
                },
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                } => {
                    self.input.fire = *state == ElementState::Pressed;
                }
                _ => {}
            },
            Event::DeviceEvent { event, .. } => match event {
                DeviceEvent::MouseMotion { delta } => {
                    self.input.mouse_delta = Vector2::new(delta.0 as f32, delta.1 as f32);
                }
                DeviceEvent::Button { button, state } if *button == 0 => {
                    self.input.fire = *state == ElementState::Pressed;
                }
                DeviceEvent::MouseWheel {
                    delta:
                        winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition {
                            y,
                            ..
                        }),
                    ..
                } => {
                    self.character.speed += *y as f32 / 200.0;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[derive(Default)]
struct Ui {
    error_string: String,
}
