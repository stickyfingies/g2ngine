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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParticleSystemDesc {
    #[serde(rename = "grid")]
    Grid { count: usize, params: GridParams },
}

pub struct ParticleSystem {
    pub name: String,
    pub desc: ParticleSystemDesc,
    pub instance_buffer: wgpu::Buffer,
    pub num_instances: usize,
    needs_rebuild: bool,
    last_edit_time: web_time::Instant,
}

const DEBOUNCE_MS: u64 = 20;

impl ParticleSystem {
    pub fn new(device: &wgpu::Device, name: String, desc: ParticleSystemDesc) -> Self {
        let instances = Self::generate_instances(&desc);
        let num_instances = instances.len();

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Particle System '{}' Instance Buffer", name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            name,
            desc,
            instance_buffer,
            num_instances,
            needs_rebuild: false,
            last_edit_time: web_time::Instant::now(),
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
        let spacing = params.spacing;
        let center = Vector3::new(params.center[0], params.center[1], params.center[2]);

        let displacement = Vector3::new(rows as f32 * 0.5, 0.0, rows as f32 * 0.5);

        let mut instances = Vec::with_capacity(count);

        for x in 0..rows {
            for z in 0..rows {
                let position =
                    (Vector3::new(x as f32, 0.0, z as f32) - displacement) * spacing + center;

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
        self.needs_rebuild = true;
        self.last_edit_time = web_time::Instant::now();
    }

    pub fn should_rebuild(&self) -> bool {
        self.needs_rebuild && self.last_edit_time.elapsed().as_millis() >= DEBOUNCE_MS as u128
    }

    pub fn rebuild(&mut self, device: &wgpu::Device) {
        let instances = Self::generate_instances(&self.desc);
        self.num_instances = instances.len();

        self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Particle System '{}' Instance Buffer", self.name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.needs_rebuild = false;
    }

    pub fn update_if_ready(&mut self, device: &wgpu::Device) {
        if self.should_rebuild() {
            self.rebuild(device);
        }
    }
}
