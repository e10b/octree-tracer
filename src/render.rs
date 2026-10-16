use super::*;

const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct Render {
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    render_size: winit::dpi::PhysicalSize<u32>,
    pub render_pipeline: wgpu::RenderPipeline,
    pub uniforms: Uniforms,
    pub uniform_buffer: wgpu::Buffer,
    pub node_buffer: wgpu::Buffer,
    main_bind_groups: [wgpu::BindGroup; 2],
    main_bind_group_layout: wgpu::BindGroupLayout,
    history_buffers: [wgpu::Buffer; 2],
    history_index: usize,
    history_valid: bool,
    svgf: Svgf,
    g_illum: wgpu::TextureView,
    g_normal: wgpu::TextureView,
    g_albedo: wgpu::TextureView,
    g_moments: wgpu::TextureView,
    previous_camera: [[f32; 4]; 4],
    blue_noise_view: wgpu::TextureView,
    path_uniform_buffer: wgpu::Buffer,
    path_uniforms: PathUniforms,
    last_path_signature: Option<Vec<u32>>,
    pub path_bounces: u32,
    pub texture_emission: f32,
    pub temporal_history: f32,
    scene_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    upscale_layout: wgpu::BindGroupLayout,
    upscale_pipeline: wgpu::RenderPipeline,
    upscale_bind: wgpu::BindGroup,
    sprite_pipeline: wgpu::RenderPipeline,
    gun_pipeline: wgpu::RenderPipeline,
    sprite_layout: wgpu::BindGroupLayout,
    sprite_bind: wgpu::BindGroup,
    sprite_verts: wgpu::Buffer,
    sprite_count: u32,
    gun_count: u32,
    pub previous_frame_time: Option<f64>,
    pub egui_platform: egui_winit_platform::Platform,
    pub egui_rpass: egui_wgpu_backend::RenderPass,
}

impl Render {
    // Creating some of the wgpu types requires async code
    pub async fn new(gpu: &Gpu, window: &Window, octree: &Octree) -> Self {
        window.set_cursor_grab(true).unwrap();
        window.set_cursor_visible(false);

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gpu.surface.get_preferred_format(&gpu.adapter).unwrap(),
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
        };
        gpu.surface.configure(&gpu.device, &config);
        let render_size = render_resolution(size);

        let shader = gpu
            .device
            .create_shader_module(&wgpu::ShaderModuleDescriptor {
                label: Some("Shader"),
                source: wgpu::ShaderSource::Wgsl(
                    (concat!(include_str!("common.wgsl"), include_str!("shader.wgsl"))).into(),
                ),
            });

        // #region Buffers
        let uniforms = Uniforms::new();
        let uniform_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Camera Buffer"),
                contents: bytemuck::cast_slice(&[uniforms]),
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            });

        let nodes = octree.expanded(10_000_000);

        let node_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&nodes),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });

        let path_uniforms = PathUniforms {
            frame_index: 0,
            max_bounces: 3,
            texture_emission: 4.0,
            temporal_history: 0.95,
            history_valid: 0,
            padding: [0; 3],
            previous_camera: [[0.0; 4]; 4],
        };
        let path_uniform_buffer =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Path tracing uniforms"),
                    contents: bytemuck::bytes_of(&path_uniforms),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
        let history_buffers = [
            history_buffer(&gpu.device, render_size.width, render_size.height),
            history_buffer(&gpu.device, render_size.width, render_size.height),
        ];
        let blue_noise_view = blue_noise_texture(&gpu.device, &gpu.queue);

        let main_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 5,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                    label: Some("main_bind_group_layout"),
                });

        let g_illum = filter_target(&gpu.device, "SVGF illumination", render_size.width, render_size.height);
        let g_normal = filter_target(&gpu.device, "SVGF normal", render_size.width, render_size.height);
        let g_albedo = filter_target(&gpu.device, "SVGF albedo", render_size.width, render_size.height);
        let g_moments = filter_target(&gpu.device, "SVGF moments", render_size.width, render_size.height);
        let svgf = Svgf::new(
            &gpu.device,
            render_size.width,
            render_size.height,
            &g_illum,
            &g_normal,
            &g_albedo,
            &g_moments,
        );
        let main_bind_groups = history_bind_groups(
            &gpu.device,
            &main_bind_group_layout,
            &uniform_buffer,
            &node_buffer,
            &history_buffers,
            &path_uniform_buffer,
            &blue_noise_view,
        );
        // #endregion

        let render_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Render Pipeline Layout"),
                    bind_group_layouts: &[&main_bind_group_layout],
                    push_constant_ranges: &[],
                });

        let render_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Render Pipeline"),
                layout: Some(&render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[
                        replace_target(SCENE_FORMAT),
                        replace_target(SCENE_FORMAT),
                        replace_target(SCENE_FORMAT),
                        replace_target(SCENE_FORMAT),
                    ],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Always)),
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
            });

        let scene_view = scene_texture(
            &gpu.device,
            SCENE_FORMAT,
            render_size.width,
            render_size.height,
        );
        let depth_view = depth_texture(&gpu.device, render_size.width, render_size.height);
        let (upscale_layout, upscale_pipeline, upscale_bind) =
            upscale_pass(&gpu.device, config.format, &scene_view);
        let (sprite_layout, sprite_pipeline, gun_pipeline, sprite_bind, sprite_verts) =
            sprite_pass(&gpu.device, SCENE_FORMAT);

        // egui
        let size = window.inner_size();
        let egui_platform =
            egui_winit_platform::Platform::new(egui_winit_platform::PlatformDescriptor {
                physical_width: size.width as u32,
                physical_height: size.height as u32,
                scale_factor: window.scale_factor(),
                font_definitions: egui::FontDefinitions::default(),
                style: Default::default(),
            });

        // We use the egui_wgpu_backend crate as the render backend.
        let egui_rpass = egui_wgpu_backend::RenderPass::new(&gpu.device, config.format, 1);

        let previous_frame_time = None;

        Self {
            config,
            size,
            render_size,
            render_pipeline,
            uniforms,
            uniform_buffer,
            node_buffer,
            main_bind_groups,
            main_bind_group_layout,
            history_buffers,
            history_index: 0,
            history_valid: false,
            svgf,
            g_illum,
            g_normal,
            g_albedo,
            g_moments,
            previous_camera: [[0.0; 4]; 4],
            blue_noise_view,
            path_uniform_buffer,
            path_uniforms,
            last_path_signature: None,
            path_bounces: 3,
            texture_emission: 4.0,
            temporal_history: 0.95,
            scene_view,
            depth_view,
            upscale_layout,
            upscale_pipeline,
            upscale_bind,
            sprite_pipeline,
            gun_pipeline,
            sprite_layout,
            sprite_bind,
            sprite_verts,
            sprite_count: 0,
            gun_count: 0,
            previous_frame_time,
            egui_platform,
            egui_rpass,
        }
    }

    pub fn resize(&mut self, gpu: &Gpu, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            gpu.surface.configure(&gpu.device, &self.config);
            self.render_size = render_resolution(new_size);
            self.scene_view = scene_texture(
                &gpu.device,
                SCENE_FORMAT,
                self.render_size.width,
                self.render_size.height,
            );
            self.depth_view =
                depth_texture(&gpu.device, self.render_size.width, self.render_size.height);
            self.history_buffers = [
                history_buffer(&gpu.device, self.render_size.width, self.render_size.height),
                history_buffer(&gpu.device, self.render_size.width, self.render_size.height),
            ];
            let g_illum = filter_target(&gpu.device, "SVGF illumination", self.render_size.width, self.render_size.height);
            let g_normal = filter_target(&gpu.device, "SVGF normal", self.render_size.width, self.render_size.height);
            let g_albedo = filter_target(&gpu.device, "SVGF albedo", self.render_size.width, self.render_size.height);
            let g_moments = filter_target(&gpu.device, "SVGF moments", self.render_size.width, self.render_size.height);
            self.svgf.resize(
                &gpu.device,
                self.render_size.width,
                self.render_size.height,
                &g_illum,
                &g_normal,
                &g_albedo,
                &g_moments,
            );
            self.g_illum = g_illum;
            self.g_normal = g_normal;
            self.g_albedo = g_albedo;
            self.g_moments = g_moments;
            self.main_bind_groups = history_bind_groups(
                &gpu.device,
                &self.main_bind_group_layout,
                &self.uniform_buffer,
                &self.node_buffer,
                &self.history_buffers,
                &self.path_uniform_buffer,
                &self.blue_noise_view,
            );
            self.history_index = 0;
            self.history_valid = false;
            self.upscale_bind = upscale_bind_group(
                &gpu.device,
                &self.upscale_layout,
                &self.scene_view,
            );
            self.path_uniforms.frame_index = 0;
            self.last_path_signature = None;
            gpu.queue.write_buffer(
                &self.path_uniform_buffer,
                0,
                bytemuck::bytes_of(&self.path_uniforms),
            );
        }
    }

    pub fn set_atlas(&mut self, gpu: &Gpu, rgba: &[u8], size: u32) {
        let side = size.max(1);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite atlas"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });
        upload_rgba(&gpu.queue, &texture, rgba, side, side);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        self.sprite_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite bind"),
            layout: &self.sprite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
    }

    pub fn set_sprites(
        &mut self,
        gpu: &Gpu,
        world: &[crate::play::SpriteVert],
        gun: &[crate::play::SpriteVert],
    ) {
        let mut all = Vec::with_capacity(world.len() + gun.len());
        all.extend_from_slice(world);
        all.extend_from_slice(gun);
        if all.len() > 2048 {
            all.truncate(2048);
        }
        if !all.is_empty() {
            gpu.queue
                .write_buffer(&self.sprite_verts, 0, bytemuck::cast_slice(&all));
        }
        let gun_count = gun.len().min(all.len()) as u32;
        self.sprite_count = all.len() as u32 - gun_count;
        self.gun_count = gun_count;
    }

    pub fn update(
        &mut self,
        gpu: &Gpu,
        time: f64,
        settings: &mut Settings,
        character: &Character,
        terrain_origin: Option<[f32; 3]>,
    ) {
        let dimensions = [
            self.render_size.width as f32,
            self.render_size.height as f32,
        ];

        let view = Matrix4::<f32>::look_at_rh(
            character.pos,
            character.pos + character.look,
            Vector3::unit_y(),
        );
        // let proj = perspective(Deg(settings.fov), dimensions[0] / dimensions[1], 0.00001, 0.0001);
        let proj = create_proj_matrix(settings.fov, dimensions[1] / dimensions[0]);
        let camera = proj * view;
        let camera_inverse = camera.invert().unwrap();

        // dimensions.zw and sun_dir.w carry the block-space corner of the traced cube
        // so the shader can continue a ray onto the height sheet. NaN disables it.
        let (ox, oy, oz) = match terrain_origin {
            Some([x, y, z]) => (x, y, z),
            None => (0.0, f32::NAN, 0.0),
        };
        self.uniforms.dimensions = [dimensions[0], dimensions[1], ox, oz];
        self.uniforms.sun_dir[3] = oy;
        self.uniforms.camera = camera.into();
        self.uniforms.camera_inverse = camera_inverse.into();

        let mut signature = Vec::with_capacity(42);
        signature.extend(self.uniforms.dimensions.map(f32::to_bits));
        signature.extend(self.uniforms.sun_dir.map(f32::to_bits));
        signature.extend([
            self.uniforms.show_steps as u32,
            self.uniforms.show_hits as u32,
            self.uniforms.shadows as u32,
            self.uniforms.misc_bool as u32,
            self.path_bounces,
            self.texture_emission.to_bits(),
            self.temporal_history.to_bits(),
        ]);
        let mut history_valid = self.history_valid;
        if self.last_path_signature.as_ref() == Some(&signature) {
            self.path_uniforms.frame_index = self.path_uniforms.frame_index.saturating_add(1);
        } else {
            self.path_uniforms.frame_index = 0;
            self.last_path_signature = Some(signature);
            history_valid = false;
        }
        self.path_uniforms.max_bounces = self.path_bounces;
        self.path_uniforms.texture_emission = self.texture_emission;
        self.path_uniforms.temporal_history = self.temporal_history;
        self.path_uniforms.history_valid = history_valid as u32;
        self.path_uniforms.previous_camera = self.previous_camera;

        gpu.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[self.uniforms]),
        );
        gpu.queue.write_buffer(
            &self.path_uniform_buffer,
            0,
            bytemuck::bytes_of(&self.path_uniforms),
        );

        self.egui_platform.update_time(time);
    }

    pub fn accumulated_samples(&self) -> u32 {
        self.path_uniforms.frame_index.saturating_add(1)
    }

    pub fn render(&mut self, gpu: &Gpu, window: &Window) -> Result<(), wgpu::SurfaceError> {
        let output = gpu.surface.get_current_texture()?;
        let size = window.inner_size();

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // Path trace into the history buffer. The colour attachment is replaced
        // by the SVGF output before sprites are drawn.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("480p scene pass"),
                color_attachments: &[
                    color_attachment(&self.g_illum),
                    color_attachment(&self.g_normal),
                    color_attachment(&self.g_albedo),
                    color_attachment(&self.g_moments),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.main_bind_groups[self.history_index], &[]);
            render_pass.draw(0..4, 0..1);
        }
        // Finish the gbuffer attachments before SVGF samples them.
        gpu.queue.submit(std::iter::once(encoder.finish()));

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("SVGF encoder"),
            });
        self.svgf
            .encode(&mut encoder, self.history_index ^ 1, &self.scene_view);

        if self.sprite_count > 0 || self.gun_count > 0 {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprite pass"),
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: &self.scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                }],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });
            render_pass.set_pipeline(&self.sprite_pipeline);
            render_pass.set_bind_group(0, &self.sprite_bind, &[]);
            render_pass.set_vertex_buffer(0, self.sprite_verts.slice(..));
            if self.sprite_count > 0 {
                render_pass.draw(0..self.sprite_count, 0..1);
            }
            render_pass.set_pipeline(&self.gun_pipeline);
            if self.gun_count > 0 {
                render_pass.draw(self.sprite_count..self.sprite_count + self.gun_count, 0..1);
            }
        }

        // Upscale only the game image. Egui is drawn afterwards at native resolution.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene upscale pass"),
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });
            render_pass.set_pipeline(&self.upscale_pipeline);
            render_pass.set_bind_group(0, &self.upscale_bind, &[]);
            render_pass.draw(0..4, 0..1);
        }

        // Draw the UI frame.
        self.egui_platform.begin_frame();

        // End the UI frame. We could now handle the output and draw the UI with the backend.
        let (_output, paint_commands) = self.egui_platform.end_frame(Some(window));
        let paint_jobs = self.egui_platform.context().tessellate(paint_commands);

        // Upload all resources for the GPU.
        let screen_descriptor = egui_wgpu_backend::ScreenDescriptor {
            physical_width: size.width,
            physical_height: size.height,
            scale_factor: window.scale_factor() as f32,
        };
        self.egui_rpass.update_texture(
            &gpu.device,
            &gpu.queue,
            &self.egui_platform.context().font_image(),
        );
        self.egui_rpass
            .update_user_textures(&gpu.device, &gpu.queue);
        self.egui_rpass
            .update_buffers(&gpu.device, &gpu.queue, &paint_jobs, &screen_descriptor);

        // Record all render passes.
        self.egui_rpass
            .execute(&mut encoder, &view, &paint_jobs, &screen_descriptor, None)
            .unwrap();

        // Submit the command buffer.
        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        self.history_index ^= 1;
        self.previous_camera = self.uniforms.camera;
        self.history_valid = true;

        Ok(())
    }
}

const SPRITE_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32];

fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: write,
        depth_compare: compare,
        stencil: wgpu::StencilState {
            front: wgpu::StencilFaceState::IGNORE,
            back: wgpu::StencilFaceState::IGNORE,
            read_mask: 0,
            write_mask: 0,
        },
        bias: wgpu::DepthBiasState {
            constant: 0,
            slope_scale: 0.0,
            clamp: 0.0,
        },
    }
}

fn render_resolution(size: winit::dpi::PhysicalSize<u32>) -> winit::dpi::PhysicalSize<u32> {
    let height = size.height.min(480).max(1);
    let aspect_width =
        (size.width as f64 * height as f64 / size.height.max(1) as f64).round() as u32;
    // Even widths match conventional resolutions such as 854x480 and avoid an
    // asymmetric last column when the image is upscaled.
    let width = (aspect_width.max(1) + 1) & !1;
    winit::dpi::PhysicalSize::new(width.max(2), height)
}

fn replace_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState::REPLACE),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn color_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: true,
        },
    }
}

fn scene_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("480p scene"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn upscale_pass(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    scene_view: &wgpu::TextureView,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline, wgpu::BindGroup) {
    let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
        label: Some("upscale shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("upscale layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("upscale pipeline layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("upscale pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            }],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
    });
    let bind = upscale_bind_group(device, &layout, scene_view);
    (layout, pipeline, bind)
}

fn upscale_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("upscale sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("upscale bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

fn blue_noise_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let rgba = image::load_from_memory(include_bytes!("../assets/blue-noise/LDR_RG01_0.png"))
        .expect("embedded CC0 blue-noise texture must decode")
        .to_rgba8();
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("64x64 void-and-cluster blue noise"),
            size: wgpu::Extent3d {
                width: rgba.width(),
                height: rgba.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
        },
        rgba.as_raw(),
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// `HistoryPixel` in shader.wgsl and svgf.wgsl: radiance, position, normal, moments, albedo.
const HISTORY_VEC4S: u64 = 5;

const SVGF_PASSES: [(u32, u32); 5] = [(1, 0), (2, 1), (4, 1), (8, 1), (16, 1)];

struct Svgf {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    modulate_pipeline: wgpu::RenderPipeline,
    views: [wgpu::TextureView; 2],
    illumination: [wgpu::TextureView; 2],
    dummy: wgpu::TextureView,
    uniforms: [wgpu::Buffer; 5],
    binds: [[wgpu::BindGroup; 5]; 2],
    modulate_binds: [wgpu::BindGroup; 2],
}

impl Svgf {
    fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        illum: &wgpu::TextureView,
        normal: &wgpu::TextureView,
        albedo: &wgpu::TextureView,
        moments: &wgpu::TextureView,
    ) -> Self {
        let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
            label: Some("SVGF shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("svgf.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SVGF layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                unfiltered_texture(1),
                unfiltered_texture(2),
                unfiltered_texture(3),
                unfiltered_texture(4),
                unfiltered_texture(5),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SVGF pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SVGF pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[wgpu::ColorTargetState {
                    format: SCENE_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        let modulate_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SVGF remodulate pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_modulate",
                targets: &[wgpu::ColorTargetState {
                    format: SCENE_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        let dummy = filter_target(device, "SVGF dummy", 1, 1);
        let views = [
            filter_target(device, "SVGF ping", width, height),
            filter_target(device, "SVGF pong", width, height),
        ];
        let illumination = [
            filter_target(device, "SVGF illumination history 0", width, height),
            filter_target(device, "SVGF illumination history 1", width, height),
        ];
        let uniforms = svgf_uniforms(device, width, height);
        let binds = svgf_binds(
            device, &layout, &uniforms, &views, &dummy, illum, normal, albedo, moments,
        );
        let modulate_binds =
            svgf_modulate_binds(device, &layout, &uniforms, &illumination, albedo);
        Self {
            layout,
            pipeline,
            modulate_pipeline,
            views,
            illumination,
            dummy,
            uniforms,
            binds,
            modulate_binds,
        }
    }

    fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        illum: &wgpu::TextureView,
        normal: &wgpu::TextureView,
        albedo: &wgpu::TextureView,
        moments: &wgpu::TextureView,
    ) {
        let views = [
            filter_target(device, "SVGF ping", width, height),
            filter_target(device, "SVGF pong", width, height),
        ];
        let illumination = [
            filter_target(device, "SVGF illumination history 0", width, height),
            filter_target(device, "SVGF illumination history 1", width, height),
        ];
        let uniforms = svgf_uniforms(device, width, height);
        let binds = svgf_binds(
            device, &self.layout, &uniforms, &views, &self.dummy, illum, normal, albedo, moments,
        );
        let modulate_binds =
            svgf_modulate_binds(device, &self.layout, &uniforms, &illumination, albedo);
        self.views = views;
        self.illumination = illumination;
        self.uniforms = uniforms;
        self.binds = binds;
        self.modulate_binds = modulate_binds;
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        history_slot: usize,
        scene: &wgpu::TextureView,
    ) {
        let labels = [
            "SVGF variance and à-trous stride 1",
            "SVGF à-trous stride 2",
            "SVGF à-trous stride 4",
            "SVGF à-trous stride 8",
            "SVGF à-trous stride 16",
        ];
        for pass in 0..5 {
            let target = if pass == 4 {
                &self.illumination[history_slot]
            } else if pass % 2 == 0 {
                &self.views[0]
            } else {
                &self.views[1]
            };
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(labels[pass]),
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.binds[history_slot][pass], &[]);
            render_pass.draw(0..4, 0..1);
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SVGF remodulate"),
            color_attachments: &[wgpu::RenderPassColorAttachment {
                view: scene,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: true,
                },
            }],
            depth_stencil_attachment: None,
        });
        render_pass.set_pipeline(&self.modulate_pipeline);
        render_pass.set_bind_group(0, &self.modulate_binds[history_slot], &[]);
        render_pass.draw(0..4, 0..1);
    }
}

fn filter_target(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn svgf_uniforms(device: &wgpu::Device, width: u32, height: u32) -> [wgpu::Buffer; 5] {
    let mut buffers = Vec::with_capacity(5);
    for (step, mode) in SVGF_PASSES {
        let uniforms = SvgfUniforms {
            step,
            width,
            height,
            mode,
        };
        buffers.push(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SVGF uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        }));
    }
    buffers.try_into().unwrap()
}

fn unfiltered_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn svgf_entries<'a>(
    uniforms: &'a wgpu::Buffer,
    filtered: &'a wgpu::TextureView,
    illum: &'a wgpu::TextureView,
    normal: &'a wgpu::TextureView,
    albedo: &'a wgpu::TextureView,
    moments: &'a wgpu::TextureView,
) -> [wgpu::BindGroupEntry<'a>; 6] {
    [
        wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(illum),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(normal),
        },
        wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::TextureView(albedo),
        },
        wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::TextureView(moments),
        },
        wgpu::BindGroupEntry {
            binding: 5,
            resource: wgpu::BindingResource::TextureView(filtered),
        },
    ]
}

fn svgf_binds(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &[wgpu::Buffer; 5],
    views: &[wgpu::TextureView; 2],
    dummy: &wgpu::TextureView,
    illum: &wgpu::TextureView,
    normal: &wgpu::TextureView,
    albedo: &wgpu::TextureView,
    moments: &wgpu::TextureView,
) -> [[wgpu::BindGroup; 5]; 2] {
    let inputs = [dummy, &views[0], &views[1], &views[0], &views[1]];
    let mut groups = Vec::with_capacity(2);
    for _slot in 0..2 {
        let mut passes = Vec::with_capacity(5);
        for pass in 0..5 {
            let entries = svgf_entries(
                &uniforms[pass],
                inputs[pass],
                illum,
                normal,
                albedo,
                moments,
            );
            passes.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SVGF bind group"),
                layout,
                entries: &entries,
            }));
        }
        let passes: [wgpu::BindGroup; 5] = passes.try_into().unwrap();
        groups.push(passes);
    }
    let groups: [[wgpu::BindGroup; 5]; 2] = groups.try_into().unwrap();
    groups
}

fn svgf_modulate_binds(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &[wgpu::Buffer; 5],
    illumination: &[wgpu::TextureView; 2],
    albedo: &wgpu::TextureView,
) -> [wgpu::BindGroup; 2] {
    let mut groups = Vec::with_capacity(2);
    for slot in 0..2 {
        let entries = svgf_entries(
            &uniforms[0],
            &illumination[slot],
            &illumination[slot],
            &illumination[slot],
            albedo,
            &illumination[slot],
        );
        groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SVGF remodulate bind group"),
            layout,
            entries: &entries,
        }));
    }
    groups.try_into().unwrap()
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SvgfUniforms {
    step: u32,
    width: u32,
    height: u32,
    mode: u32,
}

fn history_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
    let pixel_count = u64::from(width.max(1)) * u64::from(height.max(1));
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("SVGF history"),
        size: pixel_count * HISTORY_VEC4S * std::mem::size_of::<[f32; 4]>() as u64,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

fn history_bind_groups(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    node_buffer: &wgpu::Buffer,
    history_buffers: &[wgpu::Buffer; 2],
    path_uniform_buffer: &wgpu::Buffer,
    blue_noise_view: &wgpu::TextureView,
) -> [wgpu::BindGroup; 2] {
    let make = |previous: usize, current: usize| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Reprojected path tracing bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: node_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: history_buffers[previous].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: path_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: history_buffers[current].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(blue_noise_view),
                },
            ],
        })
    };
    [make(0, 1), make(1, 0)]
}

fn upload_rgba(queue: &wgpu::Queue, texture: &wgpu::Texture, rgba: &[u8], width: u32, height: u32) {
    let row = width * 4;
    let stride = (row + 255) & !255;
    let mut padded = vec![0u8; stride as usize * height as usize];
    let copy = row as usize;
    for y in 0..height as usize {
        let src = y * copy;
        let dst = y * stride as usize;
        if src + copy <= rgba.len() {
            padded[dst..dst + copy].copy_from_slice(&rgba[src..src + copy]);
        }
    }
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &padded,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: std::num::NonZeroU32::new(stride),
            rows_per_image: std::num::NonZeroU32::new(height.max(1)),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn sprite_pass(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (
    wgpu::BindGroupLayout,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::BindGroup,
    wgpu::Buffer,
) {
    let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
        label: Some("sprite shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sprite layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprite placeholder"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sprite placeholder bind"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite pipeline layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<crate::play::SpriteVert>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &SPRITE_ATTRS,
    }];
    let make = |write: bool, compare: wgpu::CompareFunction, label: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(depth_state(write, compare)),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        })
    };
    let sprite_pipeline = make(true, wgpu::CompareFunction::Less, "sprite pipeline");
    let gun_pipeline = make(false, wgpu::CompareFunction::Always, "gun pipeline");
    let verts = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite verts"),
        size: (2048 * std::mem::size_of::<crate::play::SpriteVert>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (layout, sprite_pipeline, gun_pipeline, bind, verts)
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Zeroable)]
pub struct Uniforms {
    pub camera: [[f32; 4]; 4],
    pub camera_inverse: [[f32; 4]; 4],
    pub dimensions: [f32; 4],
    pub sun_dir: [f32; 4],
    pub pause_adaptive: bool,
    pub show_steps: bool,
    pub show_hits: bool,
    pub shadows: bool,
    pub misc_value: f32,
    pub misc_bool: bool,
    pub junk: [u32; 8],
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct PathUniforms {
    frame_index: u32,
    max_bounces: u32,
    texture_emission: f32,
    temporal_history: f32,
    history_valid: u32,
    padding: [u32; 3],
    previous_camera: [[f32; 4]; 4],
}

// For bool
unsafe impl bytemuck::Pod for Uniforms {}

impl Uniforms {
    fn new() -> Self {
        Self {
            camera: [[0.0; 4]; 4],
            camera_inverse: [[0.0; 4]; 4],
            dimensions: [0.0, 0.0, 0.0, 0.0],
            sun_dir: [-1.7, -1.0, 0.8, f32::NAN],
            pause_adaptive: false,
            show_steps: false,
            show_hits: false,
            shadows: true,
            misc_value: 0.0,
            misc_bool: false,
            junk: [0; 8],
        }
    }
}
