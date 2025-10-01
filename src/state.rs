use crate::egui::EguiRenderer;
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

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Light {
    pub position: [f32; 4],
    pub color: [f32; 4],
}

const MAX_LIGHTS: usize = 10;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightArrayGpu {
    lights: [Light; MAX_LIGHTS],
    num_lights: u32,
    _padding: [u32; 3],
}

impl Default for Light {
    fn default() -> Self {
        Self {
            position: [0.0; 4],
            color: [0.0; 4],
        }
    }
}

pub struct LightManager {
    lights: [Light; MAX_LIGHTS],
    active_mask: u32,
    dirty: bool,
    model_path: String,
    material_key: String,
}

impl LightManager {
    pub fn new() -> Self {
        Self {
            lights: [Light::default(); MAX_LIGHTS],
            active_mask: 0,
            dirty: false,
            model_path: "teapot.obj".to_string(),
            material_key: "teapot/default".to_string(),
        }
    }

    pub fn with_lights(lights: &[([f32; 3], [f32; 4])]) -> Self {
        let mut manager = Self::new();
        for (pos, color) in lights {
            manager.add_light(*pos, *color);
        }
        manager
    }

    pub fn model_path(&self) -> &str {
        &self.model_path
    }

    pub fn set_model_path(&mut self, path: String) {
        self.model_path = path;
    }

    pub fn material_key(&self) -> &str {
        &self.material_key
    }

    pub fn set_material_key(&mut self, key: String) {
        self.material_key = key;
    }

    pub fn add_light(&mut self, pos: [f32; 3], color: [f32; 4]) -> Option<usize> {
        for i in 0..MAX_LIGHTS {
            if self.active_mask & (1 << i) == 0 {
                self.lights[i] = Light {
                    position: [pos[0], pos[1], pos[2], 1.0],
                    color,
                };
                self.active_mask |= 1 << i;
                self.dirty = true;
                return Some(i);
            }
        }
        None
    }

    pub fn remove_light(&mut self, index: usize) {
        if index < MAX_LIGHTS {
            self.active_mask &= !(1 << index);
            self.dirty = true;
        }
    }

    pub fn update_light(&mut self, index: usize, pos: [f32; 3], color: [f32; 4]) {
        if self.is_active(index) {
            self.lights[index].position = [pos[0], pos[1], pos[2], 1.0];
            self.lights[index].color = color;
            self.dirty = true;
        }
    }

    pub fn get_light(&self, index: usize) -> Option<&Light> {
        if self.is_active(index) {
            Some(&self.lights[index])
        } else {
            None
        }
    }

    pub fn sync_to_gpu(&self) -> LightArrayGpu {
        let mut gpu_lights = [Light::default(); MAX_LIGHTS];
        let mut write_idx = 0;

        for i in 0..MAX_LIGHTS {
            if self.is_active(i) {
                gpu_lights[write_idx] = self.lights[i];
                write_idx += 1;
            }
        }

        LightArrayGpu {
            lights: gpu_lights,
            num_lights: write_idx as u32,
            _padding: [0; 3],
        }
    }

    pub fn is_active(&self, index: usize) -> bool {
        index < MAX_LIGHTS && (self.active_mask & (1 << index)) != 0
    }

    pub fn num_lights(&self) -> u32 {
        self.active_mask.count_ones()
    }

    pub fn max_lights(&self) -> usize {
        MAX_LIGHTS
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
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
    camera_bind_group: wgpu::BindGroup,
    light_manager: LightManager,
    light_buffer: wgpu::Buffer,
    light_bind_group: wgpu::BindGroup,
    particle_system_manager: ParticleSystemManager,
    depth_texture: GpuTexture,
    window: Arc<Window>,
    clear_color: wgpu::Color,
    models: std::collections::HashMap<String, Arc<model::Model>>,
    materials: std::collections::HashMap<String, Arc<model::Material>>,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    #[cfg(not(target_arch = "wasm32"))]
    script_engine: ScriptEngineDesktop,
    #[cfg(target_arch = "wasm32")]
    script_engine: ScriptEngineWeb,
    elapsed_time: f32,
}

impl State {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<State> {
        let size = window.inner_size();

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
                required_limits: if cfg!(target_arch = "wasm32") {
                    wgpu::Limits::downlevel_webgl2_defaults()
                } else {
                    wgpu::Limits::default()
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

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("camera_bind_group_layout"),
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
            label: Some("camera_bind_group"),
        });

        // Initialize light manager with default lights
        let light_manager = LightManager::with_lights(&[
            ([2.0, 2.0, 2.0], [1.0, 1.0, 1.0, 1.0]),
            ([-2.0, 2.0, 2.0], [1.0, 0.0, 0.0, 1.0]),
        ]);

        let lights = light_manager.sync_to_gpu();
        let light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("light_buffer"),
            contents: bytemuck::cast_slice(&[lights]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("light_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light_bind_group"),
            layout: &light_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: light_buffer.as_entire_binding(),
            }],
        });

        let shader_source = resources::load_string("shader.wgsl").await.unwrap();
        let shader = wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        };

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[
                    &texture_bind_group_layout,
                    &camera_bind_group_layout,
                    &light_bind_group_layout,
                ],
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
                bind_group_layouts: &[&camera_bind_group_layout, &light_bind_group_layout],
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
            "teapot.obj".to_string(),
            "teapot/default".to_string(),
            GeneratorType::Grid(params),
        );

        particle_system_manager.add("main".to_string(), grid_system);

        // Load initial model into HashMap
        let mut models = std::collections::HashMap::new();
        let mut materials = std::collections::HashMap::new();

        let (teapot_model, teapot_materials) =
            model::load_model("teapot.obj", &device, &queue, &texture_bind_group_layout)
                .await
                .unwrap();

        // Move materials directly into registry (no cloning needed)
        for (key, material) in teapot_materials {
            materials.insert(key, Arc::new(material));
        }

        models.insert("teapot.obj".to_string(), Arc::new(teapot_model));

        let egui_renderer = EguiRenderer::new(
            &device,
            config.format,
            None, // egui doesn't need depth testing - it renders on top
            1,
            &window,
        );

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
            camera_bind_group,
            camera_uniform,
            light_manager,
            light_buffer,
            light_bind_group,
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
            texture_bind_group_layout,
            elapsed_time: 0.0,
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
                system.rebuild(&self.device);
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
                        &self.camera_bind_group,
                        &self.light_bind_group,
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
                            &self.camera_bind_group,
                            &self.light_bind_group,
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
}
