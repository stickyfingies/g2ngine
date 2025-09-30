use crate::app_ui::UIState;
use crate::egui::EguiRenderer;
use crate::model::{self, DrawLight, ModelVertex, Vertex};
use crate::particle_system::{InstanceRaw, ParticleSystem, ParticleSystemDesc};
use crate::scripting::ScriptEngine;
use crate::texture::GpuTexture;
use crate::{camera, resources};
use cgmath::prelude::*;
use cgmath::{Matrix4, Quaternion, Vector3};
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
struct LightUniform {
    position: [f32; 3],
    _padding: u32,
    color: [f32; 3],
    _padding2: u32,
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
    light_uniform: LightUniform,
    light_buffer: wgpu::Buffer,
    light_bind_group: wgpu::BindGroup,
    particle_system: ParticleSystem,
    depth_texture: GpuTexture,
    window: Arc<Window>,
    clear_color: wgpu::Color,
    obj_model: model::Model,
    #[cfg(not(target_arch = "wasm32"))]
    script_engine: ScriptEngineDesktop,
    #[cfg(target_arch = "wasm32")]
    script_engine: ScriptEngineWeb,
    ui_state: UIState,
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

        let light_uniform = LightUniform {
            position: [2.0, 2.0, 2.0],
            _padding: 0,
            color: [1.0, 1.0, 1.0],
            _padding2: 0,
        };

        let light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("light_buffer"),
            contents: bytemuck::cast_slice(&[light_uniform]),
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

        let grid_transform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grid_transform_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
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
                    &grid_transform_bind_group_layout,
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

        let particle_system = ParticleSystem::new(
            &device,
            "main".to_string(),
            system_desc,
            &grid_transform_bind_group_layout,
        );

        let obj_model = model::load_model("cube.obj", &device, &queue, &texture_bind_group_layout)
            .await
            .unwrap();

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
            light_uniform,
            light_buffer,
            light_bind_group,
            particle_system,
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
            obj_model,
            ui_state: UIState::default(),
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
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
        // Update the camera
        self.camera_controller.update_camera(&mut self.camera, dt);
        self.camera_uniform
            .update_view_proj(&self.camera, &self.projection);
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );

        // Update the light
        let old_position: Vector3<_> = self.light_uniform.position.into();
        self.light_uniform.position =
            (Quaternion::from_axis_angle((0.0, 1.0, 0.0).into(), cgmath::Deg(1.0)) * old_position)
                .into();
        self.queue.write_buffer(
            &self.light_buffer,
            0,
            bytemuck::cast_slice(&[self.light_uniform]),
        );

        // Call JS update function every frame and capture clear color
        match self.script_engine.call_js("update".into(), &()) {
            Ok(color) => {
                let color: [f32; 4] = color;
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

            let render_data = &self.particle_system.render;
            render_pass.set_vertex_buffer(1, render_data.instance_buffer.slice(..));

            render_pass.set_pipeline(&self.light_render_pipeline);
            render_pass.draw_light_model_instanced(
                &self.obj_model,
                0..1,
                &self.camera_bind_group,
                &self.light_bind_group,
            );

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(3, &render_data.grid_transform_bind_group, &[]);
            render_pass.draw_model_instanced(
                &self.obj_model,
                0..render_data.num_instances,
                &self.camera_bind_group,
                &self.light_bind_group,
            );
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: self.window().scale_factor() as f32,
        };

        let ui_state = &mut self.ui_state;
        let clear_color = &mut self.clear_color;
        let particle_system = &mut self.particle_system;
        let queue = &self.queue;
        self.egui_renderer.draw(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.window,
            &view,
            screen_descriptor,
            |ctx| {
                crate::app_ui::app_ui(
                    ctx,
                    ui_state,
                    clear_color,
                    particle_system,
                    dt.as_millis() as f32,
                    queue,
                );
            },
        );

        // Update particle system if needed (after UI has marked it dirty)
        self.particle_system.update_if_ready(&self.device);

        self.queue.submit(iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
