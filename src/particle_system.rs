use cgmath::{InnerSpace, Matrix3, Matrix4, Quaternion, Rotation3, Vector3};
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
    model: [[f32; 4]; 4],
    normal: [[f32; 3]; 3],
}

impl InstanceRaw {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        use wgpu::{
            BufferAddress, VertexAttribute, VertexBufferLayout, VertexFormat, VertexStepMode,
        };

        VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as BufferAddress,
            step_mode: VertexStepMode::Instance,
            attributes: &[
                VertexAttribute {
                    offset: 0,
                    shader_location: 5,
                    format: VertexFormat::Float32x4,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 4]>() as BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 8]>() as BufferAddress,
                    shader_location: 7,
                    format: VertexFormat::Float32x4,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 12]>() as BufferAddress,
                    shader_location: 8,
                    format: VertexFormat::Float32x4,
                },
                // Normal matrix
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 16]>() as BufferAddress,
                    shader_location: 9,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 19]>() as BufferAddress,
                    shader_location: 10,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 22]>() as BufferAddress,
                    shader_location: 11,
                    format: VertexFormat::Float32x3,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridParams {
    pub rows: usize,
    pub spacing: f32,
    pub center: [f32; 3],
}

// GPU uniform for grid transform (spacing + center)
// Organized to minimize padding: vec3 + f32 = 16 bytes (single vec4)
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GridTransformUniform {
    pub center: [f32; 3],
    pub spacing: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParticleSystemDesc {
    #[serde(rename = "grid")]
    Grid { count: usize, params: GridParams },
}

// HOT DATA: Accessed every frame during rendering
pub struct ParticleRenderData {
    pub instance_buffer: wgpu::Buffer,
    pub num_instances: u32,
    pub grid_transform_buffer: wgpu::Buffer,
    pub grid_transform_bind_group: wgpu::BindGroup,
}

// COLD DATA: Only accessed during rebuild operations
pub struct ParticleSystemConfig {
    pub name: String,
    pub desc: ParticleSystemDesc,
    needs_rebuild: bool,
    last_edit_time: web_time::Instant,
}

pub struct ParticleSystem {
    pub render: ParticleRenderData,
    pub config: ParticleSystemConfig,
}

const DEBOUNCE_MS: u64 = 20;

impl ParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        name: String,
        desc: ParticleSystemDesc,
        grid_transform_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let instances = Self::generate_instances(&desc);
        let num_instances = instances.len();

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Particle System '{}' Instance Buffer", name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Create grid transform uniform buffer
        let grid_transform = match &desc {
            ParticleSystemDesc::Grid { params, .. } => GridTransformUniform {
                center: params.center,
                spacing: params.spacing,
            },
        };

        let grid_transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Particle System '{}' Grid Transform Buffer", name)),
            contents: bytemuck::cast_slice(&[grid_transform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let grid_transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!(
                "Particle System '{}' Grid Transform Bind Group",
                name
            )),
            layout: grid_transform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: grid_transform_buffer.as_entire_binding(),
            }],
        });

        Self {
            render: ParticleRenderData {
                instance_buffer,
                num_instances: num_instances as u32,
                grid_transform_buffer,
                grid_transform_bind_group,
            },
            config: ParticleSystemConfig {
                name,
                desc,
                needs_rebuild: false,
                last_edit_time: web_time::Instant::now(),
            },
        }
    }

    fn generate_instances(desc: &ParticleSystemDesc) -> Vec<InstanceRaw> {
        match desc {
            ParticleSystemDesc::Grid { count, params } => {
                Self::generate_grid_instances(*count, params)
            }
        }
    }

    fn generate_grid_instances(count: usize, params: &GridParams) -> Vec<InstanceRaw> {
        let rows = params.rows;

        // Generate instances at UNIT spacing - shader will apply spacing/center transform
        let displacement = Vector3::new(rows as f32 * 0.5, 0.0, rows as f32 * 0.5);

        let mut instances = Vec::with_capacity(count);

        for x in 0..rows {
            for z in 0..rows {
                // Unit-spaced position (will be scaled by shader)
                let position = Vector3::new(x as f32, 0.0, z as f32) - displacement;

                let rotation = if position.magnitude2() < 0.001 {
                    Quaternion::new(1.0, 0.0, 0.0, 0.0)
                } else {
                    let axis = position.normalize();
                    Quaternion::from_axis_angle(axis, cgmath::Rad(std::f32::consts::PI / 4.0))
                };

                // Compute transformation matrices
                let model_matrix = Matrix4::from_translation(position) * Matrix4::from(rotation);
                let normal_matrix = Matrix3::from(rotation);

                instances.push(InstanceRaw {
                    model: model_matrix.into(),
                    normal: normal_matrix.into(),
                });

                if instances.len() >= count {
                    break;
                }
            }
            if instances.len() >= count {
                break;
            }
        }

        instances
    }

    pub fn mark_dirty(&mut self) {
        self.config.needs_rebuild = true;
        self.config.last_edit_time = web_time::Instant::now();
    }

    pub fn should_rebuild(&self) -> bool {
        self.config.needs_rebuild
            && self.config.last_edit_time.elapsed().as_millis() >= DEBOUNCE_MS as u128
    }

    pub fn rebuild(&mut self, device: &wgpu::Device) {
        let instances = Self::generate_instances(&self.config.desc);
        self.render.num_instances = instances.len() as u32;

        self.render.instance_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!(
                    "Particle System '{}' Instance Buffer",
                    self.config.name
                )),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            });

        self.config.needs_rebuild = false;
    }

    pub fn update_if_ready(&mut self, device: &wgpu::Device) {
        if self.should_rebuild() {
            self.rebuild(device);
        }
    }
}
