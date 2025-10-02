use crate::egui::EguiRenderer;
use crate::light::LightManager;
use crate::model::{self, DrawLight, ModelVertex, Vertex};
use crate::particle_system::{
    GeneratorType, InstanceRaw, ParticleSystem, ParticleSystemDesc, ParticleSystemManager,
};
use crate::scripting::ScriptEngine;
use crate::texture::GpuTexture;
use crate::world::{CameraData, LightParams, ParticleSystemData, WorldData};
use crate::{camera, resources};
use cgmath::{Deg, Matrix4, Point3, Rad};
use egui_wgpu::ScreenDescriptor;
use std::sync::{Mutex, mpsc};
use std::{iter, sync::Arc};
use wgpu::util::DeviceExt;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::Window;

#[cfg(not(target_arch = "wasm32"))]
use crate::engine_desktop::ScriptEngineDesktop;
#[cfg(target_arch = "wasm32")]
use crate::engine_web::ScriptEngineWeb;

#[cfg(not(target_arch = "wasm32"))]
type ScriptEnginePlatform = ScriptEngineDesktop;
#[cfg(target_arch = "wasm32")]
type ScriptEnginePlatform = ScriptEngineWeb;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_position: [f32; 4],
    view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    fn new() -> Self {
        use cgmath::SquareMatrix;
        Self {
            view_position: [0.0; 4],
            view_proj: Matrix4::identity().into(),
        }
    }

    fn update_view_proj(&mut self, camera: &camera::Camera, projection: &camera::Projection) {
        self.view_position = camera.position.to_homogeneous().into();
        self.view_proj = (projection.calc_matrix() * camera.calc_matrix()).into();
    }
}

fn create_render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    color_format: wgpu::TextureFormat,
    depth_format: Option<wgpu::TextureFormat>,
    vertex_layouts: &[wgpu::VertexBufferLayout],
    shader: wgpu::ShaderModuleDescriptor,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(shader);

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: vertex_layouts,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(wgpu::BlendState {
                    alpha: wgpu::BlendComponent::REPLACE,
                    color: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
            polygon_mode: wgpu::PolygonMode::Fill,
            // Requires Features::DEPTH_CLIP_CONTROL
            unclipped_depth: false,
            // Requires Features::CONSERVATIVE_RASTERIZATION
            conservative: false,
        },
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
        cache: None,
    })
}

pub struct State {
    // Put egui_renderer first so it gets dropped before GPU resources
    egui_renderer: EguiRenderer,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    is_surface_configured: bool,
    render_pipeline: wgpu::RenderPipeline,
    light_render_pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    camera: camera::Camera,
    projection: camera::Projection,
    camera_controller: camera::CameraController,
    mouse_pressed: bool,
    camera_uniform: CameraUniform,
    camera_buffer: wgpu::Buffer,
    per_frame_bind_group: wgpu::BindGroup,
    light_manager: LightManager,
    light_buffer: wgpu::Buffer,
    particle_system_manager: ParticleSystemManager,
    depth_texture: GpuTexture,
    window: Arc<Window>,
    clear_color: wgpu::Color,
    models: std::collections::HashMap<String, Arc<model::Model>>,
    materials: std::collections::HashMap<String, Arc<model::GpuMaterial>>,
    textures: Arc<Mutex<std::collections::HashMap<String, Arc<GpuTexture>>>>,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    #[cfg(not(target_arch = "wasm32"))]
    script_engine: ScriptEngineDesktop,
    #[cfg(target_arch = "wasm32")]
    script_engine: ScriptEngineWeb,
    elapsed_time: f32,
    pending_model_loads: std::collections::HashSet<String>,
    in_flight_model_loads: std::collections::HashSet<String>,
    ui_state: crate::app_ui::UiState,
    loaded_model_receiver: mpsc::Receiver<
        Result<
            (
                String,
                model::Model,
                std::collections::HashMap<String, model::GpuMaterial>,
            ),
            String,
        >,
    >,
    loaded_model_sender: mpsc::Sender<
        Result<
            (
                String,
                model::Model,
                std::collections::HashMap<String, model::GpuMaterial>,
            ),
            String,
        >,
    >,
}

impl State {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<State> {
        let size = window.inner_size();

        // Create channel for async model loading (used on web)
        let (loaded_model_sender, loaded_model_receiver) = mpsc::channel();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::PRIMARY,
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::GL,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let backend = adapter.get_info().backend;
        log::info!("Render backend: {}", backend);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: {
                    let mut limits = wgpu::Limits::downlevel_webgl2_defaults();
                    limits.max_texture_dimension_2d =
                        wgpu::Limits::default().max_texture_dimension_2d;
                    limits
                },
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off, // Trace path
            })
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        // Initialize and load script engine
        let mut script_engine = ScriptEnginePlatform::new();

        script_engine
            .load_javascript_file("gl-matrix.min.js".into())
            .await;
        script_engine.load_javascript_file("demo.js".into()).await;

        if let Err(e) = Self::call_demo_functions(&mut script_engine) {
            log::warn!("Demo functions failed: {}", e);
        }

        let depth_texture = GpuTexture::create_depth_texture(&device, &config, "Depth Texture");

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let camera = camera::Camera::new((0.0, 5.0, 10.0), cgmath::Deg(-90.0), cgmath::Deg(-20.0));
        let projection =
            camera::Projection::new(config.width, config.height, cgmath::Deg(45.0), 0.1, 1000.0);
        let camera_controller = camera::CameraController::new(20.0, 0.4);

        let mut camera_uniform = CameraUniform::new();
        camera_uniform.update_view_proj(&camera, &projection);

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Combined per-frame bind group layout (camera + lights)
        let per_frame_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("per_frame_bind_group_layout"),
            });

        // Initialize light manager with default lights
        let mut light_manager = LightManager::with_lights(&[
            ([2.0, 2.0, 2.0], [1.0, 1.0, 1.0, 1.0]),
            ([-2.0, 2.0, 2.0], [1.0, 0.0, 0.0, 1.0]),
        ]);
        light_manager.set_model_path(crate::defaults::LIGHT_MODEL_PATH.to_string());
        light_manager.set_material_key(crate::defaults::LIGHT_MATERIAL_KEY.to_string());

        let lights = light_manager.sync_to_gpu();
        let light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("light_buffer"),
            contents: bytemuck::cast_slice(&[lights]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create combined per-frame bind group (camera + lights)
        let per_frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &per_frame_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_buffer.as_entire_binding(),
                },
            ],
            label: Some("per_frame_bind_group"),
        });

        let shader_source = resources::load_string("shader.wgsl").await.unwrap();
        let shader = wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        };

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&per_frame_bind_group_layout, &texture_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = create_render_pipeline(
            &device,
            &render_pipeline_layout,
            config.format,
            Some(GpuTexture::DEPTH_FORMAT),
            &[ModelVertex::desc(), InstanceRaw::desc()],
            shader,
        );

        let light_render_pipeline = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Light Pipeline Layout"),
                bind_group_layouts: &[&per_frame_bind_group_layout],
                push_constant_ranges: &[],
            });
            let shader_source = resources::load_string("light.wgsl").await.unwrap();
            let shader = wgpu::ShaderModuleDescriptor {
                label: Some("Light Shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            };
            create_render_pipeline(
                &device,
                &layout,
                config.format,
                Some(GpuTexture::DEPTH_FORMAT),
                &[ModelVertex::desc()],
                shader,
            )
        };

        // Get particle system parameters from JS and create the system in Rust
        let system_desc: ParticleSystemDesc = script_engine
            .call_js("makeParticleSystem".into(), &())
            .unwrap();

        // NEW: Create particle system manager and add initial system
        let mut particle_system_manager = ParticleSystemManager::new();

        // Extract params from JS and create new-style grid system
        let params = match system_desc {
            ParticleSystemDesc::Grid { params, .. } => params,
        };

        let grid_system = ParticleSystem::new(
            &device,
            "main".to_string(),
            crate::defaults::INITIAL_MODEL_PATH.to_string(),
            crate::defaults::DEFAULT_MATERIAL_KEY.to_string(),
            GeneratorType::Grid(params),
        );

        particle_system_manager.add("main".to_string(), grid_system);

        // Create texture registry
        let textures = Arc::new(Mutex::new(std::collections::HashMap::new()));

        // Create default material
        let mut materials = std::collections::HashMap::new();
        let default_material = {
            let texture_name = "white.png";

            // Load texture into registry
            let diffuse_texture = {
                let mut registry = textures.lock().unwrap();
                if let Some(existing) = registry.get(texture_name) {
                    Arc::clone(existing)
                } else {
                    let diffuse_texture_bytes = resources::load_binary(texture_name).await?;
                    let texture = Arc::new(GpuTexture::from_bytes(
                        &device,
                        &queue,
                        &diffuse_texture_bytes,
                        texture_name,
                    )?);
                    registry.insert(texture_name.to_string(), Arc::clone(&texture));
                    texture
                }
            };

            let desc = model::MaterialDesc {
                name: "default".to_string(),
                texture_path: texture_name.to_string(),
                properties: std::cell::RefCell::new(model::MaterialProperties::default()),
                source: model::MaterialSource::System,
            };

            let properties_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("default_material_properties"),
                contents: bytemuck::cast_slice(&[*desc.properties.borrow()]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("default_material_bind_group"),
                layout: &texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: properties_buffer.as_entire_binding(),
                    },
                ],
            });

            model::GpuMaterial {
                desc,
                diffuse_texture,
                properties_buffer,
                bind_group,
            }
        };
        materials.insert("default".to_string(), Arc::new(default_material));

        // Load initial model into HashMap
        let mut models = std::collections::HashMap::new();

        let (initial_model, initial_materials) = model::load_model(
            crate::defaults::INITIAL_MODEL_PATH,
            &device,
            &queue,
            &texture_bind_group_layout,
            &textures,
        )
        .await
        .unwrap();

        // Move materials directly into registry (no cloning needed)
        for (key, material) in initial_materials {
            materials.insert(key, Arc::new(material));
        }

        models.insert(
            crate::defaults::INITIAL_MODEL_PATH.to_string(),
            Arc::new(initial_model),
        );

        let egui_renderer = EguiRenderer::new(
            &device,
            config.format,
            None, // egui doesn't need depth testing - it renders on top
            1,
            &window,
        );

        let limits = device.limits().clone();

        Ok(Self {
            egui_renderer,
            surface,
            device,
            queue,
            config,
            is_surface_configured: false,
            render_pipeline,
            light_render_pipeline,
            camera,
            projection,
            camera_controller,
            camera_buffer,
            per_frame_bind_group,
            camera_uniform,
            light_manager,
            light_buffer,
            particle_system_manager,
            depth_texture,
            window,
            mouse_pressed: false,
            clear_color: wgpu::Color {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 1.0,
            },
            script_engine,
            models,
            materials,
            textures,
            texture_bind_group_layout,
            elapsed_time: 0.0,
            pending_model_loads: std::collections::HashSet::new(),
            in_flight_model_loads: std::collections::HashSet::new(),
            ui_state: crate::app_ui::UiState::default(),
            loaded_model_receiver,
            loaded_model_sender,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Get or load a model by path. Returns Arc for cheap cloning.
    pub async fn get_or_load_model(&mut self, path: &str) -> anyhow::Result<Arc<model::Model>> {
        if let Some(model) = self.models.get(path) {
            Ok(Arc::clone(model))
        } else {
            let (model, materials) = model::load_model(
                path,
                &self.device,
                &self.queue,
                &self.texture_bind_group_layout,
                &self.textures,
            )
            .await?;

            // Register materials into the materials registry
            for (key, material) in materials {
                self.materials.insert(key, Arc::new(material));
            }

            let model = Arc::new(model);
            self.models.insert(path.to_string(), Arc::clone(&model));
            Ok(model)
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.is_surface_configured = true;
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.depth_texture =
                GpuTexture::create_depth_texture(&self.device, &self.config, "Depth Texture");
            self.projection.resize(width, height);
        }
    }

    pub fn mouse_movement(&mut self, dx: f64, dy: f64) {
        if self.mouse_pressed {
            self.camera_controller.handle_mouse(dx, dy);
        }
    }

    pub fn input(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent) -> bool {
        if self.egui_renderer.handle_input(&self.window, event) {
            return true;
        }

        match event {
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state,
                        ..
                    },
                ..
            } => match key {
                KeyCode::Escape => {
                    event_loop.exit();
                    true
                }
                _ => self.camera_controller.process_keyboard(*key, *state),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                self.camera_controller.handle_mouse_scroll(delta);
                true
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state,
                ..
            } => {
                self.mouse_pressed = *state == ElementState::Pressed;
                true
            }
            _ => false,
        }
    }

    pub fn update(&mut self, dt: web_time::Duration) {
        let dt_secs = dt.as_secs_f32();
        self.elapsed_time += dt_secs;

        // Update the camera
        self.camera_controller.update_camera(&mut self.camera, dt);
        self.camera_uniform
            .update_view_proj(&self.camera, &self.projection);
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );

        // Sync light manager to GPU only if dirty
        if self.light_manager.is_dirty() {
            let lights = self.light_manager.sync_to_gpu();
            self.queue
                .write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[lights]));
            self.light_manager.clear_dirty();
        }

        // Poll channel for loaded models (from async tasks)
        while let Ok(result) = self.loaded_model_receiver.try_recv() {
            match result {
                Ok((path, loaded_model, materials)) => {
                    log::info!("Registering loaded model: {}", path);

                    // Register materials
                    for (key, material) in materials {
                        self.materials.insert(key, Arc::new(material));
                    }

                    // Register model
                    self.models.insert(path.clone(), Arc::new(loaded_model));
                    self.in_flight_model_loads.remove(&path);
                    log::info!("Model '{}' registered successfully", path);
                }
                Err(error_msg) => {
                    log::error!("Model load failed: {}", error_msg);
                    // Extract path from error message if possible
                    if let Some(path) = error_msg.split('\'').nth(1) {
                        self.in_flight_model_loads.remove(path);
                    }
                }
            }
        }

        // Process pending model loads
        if !self.pending_model_loads.is_empty() {
            let paths_to_load: Vec<String> = self.pending_model_loads.drain().collect();

            for path in paths_to_load {
                // Check if already loaded or currently loading
                if self.models.contains_key(&path) {
                    log::info!("Model '{}' already loaded", path);
                    continue;
                }

                if self.in_flight_model_loads.contains(&path) {
                    log::info!("Model '{}' already loading", path);
                    continue;
                }

                log::info!("Starting load for model: {}", path);
                self.in_flight_model_loads.insert(path.clone());

                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Desktop: spawn thread and send result through channel
                    let device = self.device.clone();
                    let queue = self.queue.clone();
                    let texture_bind_group_layout = self.texture_bind_group_layout.clone();
                    let textures = Arc::clone(&self.textures);
                    let sender = self.loaded_model_sender.clone();
                    let path_clone = path.clone();

                    std::thread::spawn(move || {
                        let result = pollster::block_on(model::load_model(
                            &path_clone,
                            &device,
                            &queue,
                            &texture_bind_group_layout,
                            &textures,
                        ));

                        match result {
                            Ok((loaded_model, materials)) => {
                                log::info!("Model '{}' loaded in background thread", path_clone);
                                if let Err(e) =
                                    sender.send(Ok((path_clone.clone(), loaded_model, materials)))
                                {
                                    log::error!(
                                        "Failed to send loaded model '{}': {}",
                                        path_clone,
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                let error_msg =
                                    format!("Failed to load model '{}': {}", path_clone, e);
                                let _ = sender.send(Err(error_msg));
                            }
                        }
                    });
                }

                #[cfg(target_arch = "wasm32")]
                {
                    // Web: spawn async task and send result through channel
                    let device = self.device.clone();
                    let queue = self.queue.clone();
                    let texture_bind_group_layout = self.texture_bind_group_layout.clone();
                    let textures = Arc::clone(&self.textures);
                    let sender = self.loaded_model_sender.clone();

                    wasm_bindgen_futures::spawn_local(async move {
                        match model::load_model(
                            &path,
                            &device,
                            &queue,
                            &texture_bind_group_layout,
                            &textures,
                        )
                        .await
                        {
                            Ok((loaded_model, materials)) => {
                                log::info!("Model '{}' loaded, sending to main thread", path);
                                if let Err(e) =
                                    sender.send(Ok((path.clone(), loaded_model, materials)))
                                {
                                    log::error!("Failed to send loaded model '{}': {}", path, e);
                                }
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to load model '{}': {}", path, e);
                                let _ = sender.send(Err(error_msg));
                            }
                        }
                    });
                }
            }
        }

        // Call JS update function every frame and capture clear color
        match self.script_engine.call_js("update".into(), &()) {
            Ok(_color) => {
                let _color: [f32; 4] = _color;
                // self.set_clear_color(color);
            }
            Err(e) => {
                log::warn!("{}", e);
            }
        }
    }

    pub fn set_clear_color(&mut self, color: [f32; 4]) {
        self.clear_color = wgpu::Color {
            r: color[0].clamp(0.0, 1.0) as f64,
            g: color[1].clamp(0.0, 1.0) as f64,
            b: color[2].clamp(0.0, 1.0) as f64,
            a: color[3].clamp(0.0, 1.0) as f64,
        };
    }

    pub fn call_demo_functions(_script_engine: &mut ScriptEnginePlatform) -> Result<(), String> {
        // let result: Vec<f32> = _script_engine.call_js_float32array("makeInstances".into(), &())?;
        // log::info!("makeInstances(): {:?}", result);

        // // Demonstrate calling JavaScript functions from Rust with simple data
        // let result: String = script_engine.call_js("getInfo".into(), &())?;
        // log::info!("JS getInfo() returned: {}", result);

        // let result: String = script_engine.call_js("greet".into(), &"Rust".to_string())?;
        // log::info!("JS greet('Rust') returned: {}", result);

        // let result: f32 = script_engine.call_js("add".into(), &[5, 3])?;
        // log::info!("JS add([5, 3]) returned: {}", result);

        // // NEW: Demonstrate passing a Rust struct to JavaScript
        // let game_data = GameData {
        //     player_name: "Alice".to_string(),
        //     score: 1250,
        //     level: 5,
        //     position: [100.5, 200.0],
        // };

        // let _result: () = script_engine.call_js("processGameData".into(), &game_data)?;

        Ok(())
    }

    pub fn render(&mut self, dt: web_time::Duration) -> Result<(), wgpu::SurfaceError> {
        self.window.request_redraw();

        if !self.is_surface_configured {
            return Ok(());
        }

        // Rebuild particle systems if needed (before render pass)
        for (_name, system) in self.particle_system_manager.systems_mut() {
            if system.needs_rebuild() {
                system.rebuild(&self.device, &self.queue);
            }
        }

        let output = self.surface.get_current_texture()?;
        if output.suboptimal {
            return Err(wgpu::SurfaceError::Outdated);
        }

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            use model::DrawModel;

            // Render lights
            render_pass.set_pipeline(&self.light_render_pipeline);
            if let (Some(light_model), Some(_light_material)) = (
                self.models.get(self.light_manager.model_path()),
                self.materials.get(self.light_manager.material_key()),
            ) {
                // Draw first mesh of the light model with the specified material
                if let Some(mesh) = light_model.meshes.first() {
                    render_pass.draw_light_mesh_instanced(
                        mesh,
                        0..self.light_manager.num_lights(),
                        &self.per_frame_bind_group,
                    );
                }
            }

            // Render particle systems
            render_pass.set_pipeline(&self.render_pipeline);

            for (_name, system) in self.particle_system_manager.systems() {
                if let (Some(model), Some(material)) = (
                    self.models.get(system.model_path()),
                    self.materials.get(system.material_key()),
                ) {
                    render_pass.set_vertex_buffer(1, system.instance_buffer().slice(..));
                    // Draw first mesh with specified material
                    if let Some(mesh) = model.meshes.first() {
                        render_pass.draw_mesh_instanced(
                            mesh,
                            material,
                            0..system.num_instances(),
                            &self.per_frame_bind_group,
                        );
                    }
                }
            }
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: self.window().scale_factor() as f32,
        };

        let clear_color = &mut self.clear_color;
        let particle_system_manager = &mut self.particle_system_manager;
        let light_manager = &mut self.light_manager;
        let light_buffer = &self.light_buffer;
        let queue = &self.queue;
        let loading_models_count =
            self.pending_model_loads.len() + self.in_flight_model_loads.len();
        let ui_actions = self.egui_renderer.draw(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.window,
            &view,
            screen_descriptor,
            |ctx| {
                crate::app_ui::app_ui(
                    ctx,
                    clear_color,
                    particle_system_manager,
                    light_manager,
                    light_buffer,
                    dt.as_millis() as f32,
                    queue,
                    &self.device,
                    &self.models,
                    &self.materials,
                    &self.textures,
                    &mut self.ui_state,
                    loading_models_count,
                )
            },
        );

        // Handle UI actions after rendering
        if ui_actions.save_requested {
            if let Err(e) = self.save_world_to_file("world.json") {
                log::error!("Failed to save world: {}", e);
            }
        }
        if ui_actions.load_requested {
            if let Err(e) = self.load_world_from_file("world.json") {
                log::error!("Failed to load world: {}", e);
            }
        }
        if let Some(model_path) = ui_actions.model_to_load {
            self.pending_model_loads.insert(model_path);
        }
        if let Some((material_key, color)) = ui_actions.material_color_changed {
            if let Some(material) = self.materials.get(&material_key) {
                material.desc.properties.borrow_mut().color = color;
                self.queue.write_buffer(
                    &material.properties_buffer,
                    0,
                    bytemuck::cast_slice(&[*material.desc.properties.borrow()]),
                );
            }
        }
        if let Some((name, texture_path, color)) = ui_actions.material_to_create {
            match self.create_material(name, texture_path, color) {
                Ok(material_key) => {
                    log::info!("Successfully created material: {}", material_key);
                }
                Err(e) => {
                    log::error!("Failed to create material: {}", e);
                }
            }
        }
        if let Some((material_key, new_texture_path)) = ui_actions.material_texture_changed {
            if let Err(e) = self.change_material_texture(&material_key, &new_texture_path) {
                log::error!("Failed to change material texture: {}", e);
            }
        }

        self.queue.submit(iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// Export current world state to a serializable format
    pub fn export_world(&self) -> WorldData {
        // Export camera
        let camera_data = CameraData {
            position: [
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
            ],
            yaw_deg: Rad::from(self.camera.yaw).0.to_degrees(),
            pitch_deg: Rad::from(self.camera.pitch).0.to_degrees(),
            fovy_deg: Rad::from(self.projection.fovy).0.to_degrees(),
            znear: self.projection.znear,
            zfar: self.projection.zfar,
        };

        // Export lights
        let mut lights = Vec::new();
        for i in 0..self.light_manager.max_lights() {
            if let Some(light) = self.light_manager.get_light(i) {
                lights.push(LightParams {
                    position: [light.position[0], light.position[1], light.position[2]],
                    color: light.color,
                    model: self.light_manager.model_path().to_string(),
                    material_key: self.light_manager.material_key().to_string(),
                });
            }
        }

        // Export particle systems
        let mut particle_systems = Vec::new();
        for (name, system) in self.particle_system_manager.systems() {
            particle_systems.push(ParticleSystemData {
                name: name.clone(),
                model: system.model_path().to_string(),
                material_key: system.material_key().to_string(),
                generator: system.generator().clone(),
            });
        }

        // Export background color
        let background_color = [
            self.clear_color.r as f32,
            self.clear_color.g as f32,
            self.clear_color.b as f32,
            self.clear_color.a as f32,
        ];

        WorldData {
            background_color,
            camera: camera_data,
            lights,
            particle_systems,
        }
    }

    /// Load world state from serialized data
    pub fn load_world(&mut self, data: WorldData) {
        // Pre-load all models required by the world
        let mut required_models = std::collections::HashSet::new();
        for light_data in &data.lights {
            required_models.insert(light_data.model.clone());
        }
        for ps_data in &data.particle_systems {
            required_models.insert(ps_data.model.clone());
        }

        for model_path in required_models {
            if !self.models.contains_key(&model_path) {
                self.pending_model_loads.insert(model_path);
            }
        }

        // Load camera
        self.camera = camera::Camera::new(
            Point3::new(
                data.camera.position[0],
                data.camera.position[1],
                data.camera.position[2],
            ),
            Deg(data.camera.yaw_deg),
            Deg(data.camera.pitch_deg),
        );

        self.projection.fovy = Deg(data.camera.fovy_deg).into();
        self.projection.znear = data.camera.znear;
        self.projection.zfar = data.camera.zfar;

        // Update camera uniform
        self.camera_uniform
            .update_view_proj(&self.camera, &self.projection);
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );

        // Load lights
        self.light_manager = LightManager::new();
        if let Some(first_light) = data.lights.first() {
            self.light_manager.set_model_path(first_light.model.clone());
            self.light_manager
                .set_material_key(first_light.material_key.clone());
        }
        for light_data in data.lights {
            self.light_manager
                .add_light(light_data.position, light_data.color);
        }

        // Sync lights to GPU
        let lights = self.light_manager.sync_to_gpu();
        self.queue
            .write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[lights]));
        self.light_manager.clear_dirty();

        // Load particle systems
        self.particle_system_manager = ParticleSystemManager::new();
        for ps_data in data.particle_systems {
            let system = ParticleSystem::new(
                &self.device,
                ps_data.name.clone(),
                ps_data.model,
                ps_data.material_key,
                ps_data.generator,
            );
            self.particle_system_manager.add(ps_data.name, system);
        }

        // Load background color
        self.clear_color = wgpu::Color {
            r: data.background_color[0] as f64,
            g: data.background_color[1] as f64,
            b: data.background_color[2] as f64,
            a: data.background_color[3] as f64,
        };
    }
    /// Save world to JSON file (desktop) or LocalStorage (web)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_world_to_file(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let world = self.export_world();
        let json = serde_json::to_string_pretty(&world)?;
        std::fs::write(path, json)?;
        log::info!("World saved to {}", path);
        Ok(())
    }

    /// Save world to LocalStorage (web)
    #[cfg(target_arch = "wasm32")]
    pub fn save_world_to_file(&self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let world = self.export_world();
        let json = serde_json::to_string(&world)?;

        let window = web_sys::window().ok_or("No window object")?;
        let storage = window
            .local_storage()
            .map_err(|e| format!("Failed to get localStorage: {:?}", e))?
            .ok_or("localStorage not available")?;

        storage
            .set_item(key, &json)
            .map_err(|e| format!("Failed to save to localStorage: {:?}", e))?;

        log::info!("World saved to localStorage key: {}", key);
        Ok(())
    }

    /// Load world from JSON file (desktop)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_world_from_file(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let world: WorldData = serde_json::from_str(&json)?;
        self.load_world(world);
        log::info!("World loaded from {}", path);
        Ok(())
    }

    /// Load world from LocalStorage (web)
    #[cfg(target_arch = "wasm32")]
    pub fn load_world_from_file(&mut self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let window = web_sys::window().ok_or("No window object")?;
        let storage = window
            .local_storage()
            .map_err(|e| format!("Failed to get localStorage: {:?}", e))?
            .ok_or("localStorage not available")?;

        let json = storage
            .get_item(key)
            .map_err(|e| format!("Failed to read from localStorage: {:?}", e))?
            .ok_or_else(|| format!("No saved world found with key: {}", key))?;

        let world: WorldData = serde_json::from_str(&json)?;
        self.load_world(world);
        log::info!("World loaded from localStorage key: {}", key);
        Ok(())
    }

    /// Create a new material dynamically at runtime
    pub fn create_material(
        &mut self,
        name: String,
        texture_path: String,
        color: [f32; 4],
    ) -> Result<String, String> {
        // Generate unique material key
        let material_key = format!("custom/{}", name);

        // Check if material already exists
        if self.materials.contains_key(&material_key) {
            return Err(format!("Material '{}' already exists", material_key));
        }

        // Get or load texture from registry
        let diffuse_texture = {
            let mut registry = self.textures.lock().unwrap();
            if let Some(existing) = registry.get(&texture_path) {
                Arc::clone(existing)
            } else {
                return Err(format!(
                    "Texture '{}' not found in registry. Load it first.",
                    texture_path
                ));
            }
        };

        let desc = model::MaterialDesc {
            name: name.clone(),
            texture_path: texture_path.clone(),
            properties: std::cell::RefCell::new(model::MaterialProperties { color }),
            source: model::MaterialSource::Custom,
        };

        let properties_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{}_properties", name)),
                contents: bytemuck::cast_slice(&[*desc.properties.borrow()]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{}_bind_group", name)),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: properties_buffer.as_entire_binding(),
                },
            ],
        });

        let gpu_material = model::GpuMaterial {
            desc,
            diffuse_texture,
            properties_buffer,
            bind_group,
        };

        self.materials
            .insert(material_key.clone(), Arc::new(gpu_material));
        log::info!(
            "Created material '{}' with texture '{}'",
            material_key,
            texture_path
        );

        Ok(material_key)
    }

    /// Get all materials by source type
    pub fn materials_by_source(
        &self,
        source: model::MaterialSource,
    ) -> Vec<(String, Arc<model::GpuMaterial>)> {
        self.materials
            .iter()
            .filter(|(_, material)| material.desc.source == source)
            .map(|(key, material)| (key.clone(), Arc::clone(material)))
            .collect()
    }

    /// Check if a material can be edited (custom or modified model materials)
    pub fn is_material_editable(&self, material_key: &str) -> bool {
        if let Some(material) = self.materials.get(material_key) {
            match material.desc.source {
                model::MaterialSource::System => false, // System materials are read-only
                model::MaterialSource::Model(_) => true, // Can modify model materials
                model::MaterialSource::Custom => true,  // Can modify custom materials
            }
        } else {
            false
        }
    }

    /// Check if a material can be deleted
    pub fn is_material_deletable(&self, material_key: &str) -> bool {
        if let Some(material) = self.materials.get(material_key) {
            matches!(material.desc.source, model::MaterialSource::Custom)
        } else {
            false
        }
    }

    /// Change a material's texture at runtime
    pub fn change_material_texture(
        &mut self,
        material_key: &str,
        new_texture_path: &str,
    ) -> Result<(), String> {
        // Get the material
        let material = self
            .materials
            .get(material_key)
            .ok_or_else(|| format!("Material '{}' not found", material_key))?;

        // Check if texture is already the same
        if material.desc.texture_path == new_texture_path {
            return Ok(());
        }

        // Get or load new texture from registry
        let new_texture = {
            let registry = self.textures.lock().unwrap();
            registry.get(new_texture_path).cloned().ok_or_else(|| {
                format!(
                    "Texture '{}' not found in registry. Load it first.",
                    new_texture_path
                )
            })?
        };

        // Clone the current properties
        let current_properties = *material.desc.properties.borrow();

        // Create new material desc (preserve source)
        let new_desc = model::MaterialDesc {
            name: material.desc.name.clone(),
            texture_path: new_texture_path.to_string(),
            properties: std::cell::RefCell::new(current_properties),
            source: material.desc.source.clone(),
        };

        // Create new properties buffer (reuse same data)
        let properties_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{}_properties", material.desc.name)),
                contents: bytemuck::cast_slice(&[current_properties]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        // Create new bind group with new texture
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{}_bind_group", material.desc.name)),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&new_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&new_texture.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: properties_buffer.as_entire_binding(),
                },
            ],
        });

        // Create new GPU material
        let new_gpu_material = model::GpuMaterial {
            desc: new_desc,
            diffuse_texture: new_texture,
            properties_buffer,
            bind_group,
        };

        // Replace in registry
        self.materials
            .insert(material_key.to_string(), Arc::new(new_gpu_material));
        log::info!(
            "Changed material '{}' texture to '{}'",
            material_key,
            new_texture_path
        );

        Ok(())
    }
}
