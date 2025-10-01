use cgmath::{InnerSpace, Matrix3, Matrix4, Quaternion, Rotation3, Vector3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

const DEBOUNCE_MS: u64 = 20;

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

// ============================================================================
// GENERATOR PARAMETERS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridParams {
    pub rows: usize,
    pub spacing: f32,
    pub center: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SphereParams {
    pub count: usize,
    pub radius: f32,
    pub center: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GeneratorType {
    #[serde(rename = "grid")]
    Grid(GridParams),
    #[serde(rename = "sphere")]
    Sphere(SphereParams),
}

/// This is used by demo.js to return a description of the original starting particle system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParticleSystemDesc {
    #[serde(rename = "grid")]
    Grid { count: usize, params: GridParams },
}

// ============================================================================
// UNIFIED PARTICLE SYSTEM
// ============================================================================

pub struct ParticleSystem {
    name: String,
    model_path: String,
    material_key: String,
    generator: GeneratorType,
    instance_buffer: wgpu::Buffer,
    num_instances: u32,
    needs_rebuild: bool,
    last_edit_time: web_time::Instant,
}

impl ParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        name: String,
        model_path: String,
        material_key: String,
        generator: GeneratorType,
    ) -> Self {
        let instances = Self::generate_instances(&generator);

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Particle System '{}' Instance Buffer", name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            name,
            model_path,
            material_key,
            generator,
            instance_buffer,
            num_instances: instances.len() as u32,
            needs_rebuild: false,
            last_edit_time: web_time::Instant::now(),
        }
    }

    fn generate_instances(generator: &GeneratorType) -> Vec<InstanceRaw> {
        match generator {
            GeneratorType::Grid(params) => Self::generate_grid_instances(params),
            GeneratorType::Sphere(params) => Self::generate_sphere_instances(params),
        }
    }

    fn generate_grid_instances(params: &GridParams) -> Vec<InstanceRaw> {
        let count = params.rows * params.rows;
        let rows = params.rows;
        let displacement = Vector3::new(rows as f32 * 0.5, 0.0, rows as f32 * 0.5);
        let center = Vector3::new(params.center[0], params.center[1], params.center[2]);

        let mut instances = Vec::with_capacity(count);

        for x in 0..rows {
            for z in 0..rows {
                // Compute grid position
                let grid_position = Vector3::new(x as f32, 0.0, z as f32) - displacement;

                // Apply spacing and center to get world position (fully on CPU now)
                let world_position = grid_position * params.spacing + center;

                let rotation = if grid_position.magnitude2() < 0.001 {
                    Quaternion::new(1.0, 0.0, 0.0, 0.0)
                } else {
                    let axis = grid_position.normalize();
                    Quaternion::from_axis_angle(axis, cgmath::Rad(std::f32::consts::PI / 4.0))
                };

                let model_matrix =
                    Matrix4::from_translation(world_position) * Matrix4::from(rotation);
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

    fn generate_sphere_instances(params: &SphereParams) -> Vec<InstanceRaw> {
        let count = params.count;
        let center = Vector3::new(params.center[0], params.center[1], params.center[2]);
        let mut instances = Vec::with_capacity(count);

        // Golden spiral / Fibonacci sphere distribution
        let golden_ratio = (1.0 + 5.0_f32.sqrt()) / 2.0;
        let angle_increment = std::f32::consts::PI * 2.0 * golden_ratio;

        for i in 0..count {
            let t = i as f32 / count as f32;
            let inclination = (1.0 - 2.0 * t).acos();
            let azimuth = angle_increment * i as f32;

            let x = inclination.sin() * azimuth.cos();
            let y = inclination.sin() * azimuth.sin();
            let z = inclination.cos();

            let unit_position = Vector3::new(x, y, z);

            // Apply radius and center to get world position (fully on CPU now)
            let world_position = unit_position * params.radius + center;

            // Rotation to face outward from center
            let up = Vector3::new(0.0, 1.0, 0.0);
            let rotation = if unit_position.magnitude2() > 0.001 {
                let forward = unit_position.normalize();
                let right = forward.cross(up).normalize();
                let new_up = right.cross(forward);

                Quaternion::from_arc(Vector3::new(0.0, 0.0, 1.0), forward, Some(new_up))
            } else {
                Quaternion::new(1.0, 0.0, 0.0, 0.0)
            };

            let model_matrix = Matrix4::from_translation(world_position) * Matrix4::from(rotation);
            let normal_matrix = Matrix3::from(rotation);

            instances.push(InstanceRaw {
                model: model_matrix.into(),
                normal: normal_matrix.into(),
            });
        }

        instances
    }

    pub fn name(&self) -> &str {
        &self.name
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

    pub fn generator(&self) -> &GeneratorType {
        &self.generator
    }

    pub fn generator_mut(&mut self) -> &mut GeneratorType {
        &mut self.generator
    }

    pub fn set_generator(&mut self, generator: GeneratorType) {
        self.generator = generator;
        self.mark_dirty();
    }

    pub fn num_instances(&self) -> u32 {
        self.num_instances
    }

    pub fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buffer
    }

    pub fn needs_rebuild(&self) -> bool {
        self.needs_rebuild && self.last_edit_time.elapsed().as_millis() >= DEBOUNCE_MS as u128
    }

    pub fn rebuild(&mut self, device: &wgpu::Device) {
        let instances = Self::generate_instances(&self.generator);
        self.num_instances = instances.len() as u32;

        self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Particle System '{}' Instance Buffer", self.name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.needs_rebuild = false;
    }

    pub fn mark_dirty(&mut self) {
        self.needs_rebuild = true;
        self.last_edit_time = web_time::Instant::now();
    }
}

// ============================================================================
// PARTICLE SYSTEM MANAGER
// ============================================================================

pub struct ParticleSystemManager {
    systems: HashMap<String, ParticleSystem>,
}

impl ParticleSystemManager {
    pub fn new() -> Self {
        Self {
            systems: HashMap::new(),
        }
    }

    pub fn add(&mut self, name: String, system: ParticleSystem) {
        self.systems.insert(name, system);
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.systems.remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<&ParticleSystem> {
        self.systems.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut ParticleSystem> {
        self.systems.get_mut(name)
    }

    pub fn systems(&self) -> impl Iterator<Item = (&String, &ParticleSystem)> {
        self.systems.iter()
    }

    pub fn systems_mut(&mut self) -> impl Iterator<Item = (&String, &mut ParticleSystem)> {
        self.systems.iter_mut()
    }

    pub fn count(&self) -> usize {
        self.systems.len()
    }
}
