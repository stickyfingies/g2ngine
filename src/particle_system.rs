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

/// This is used by demo.js to return a description of the original starting particle system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParticleSystemDesc {
    #[serde(rename = "grid")]
    Grid { count: usize, params: GridParams },
}

/// Common interface for all particle system types
pub trait ParticleSystemType {
    /// Get the name of this system
    fn name(&self) -> &str;

    /// Get the number of instances to render
    fn num_instances(&self) -> u32;

    /// Get the instance buffer
    fn instance_buffer(&self) -> &wgpu::Buffer;

    /// Get the bind group for type-specific uniforms
    fn uniform_bind_group(&self) -> &wgpu::BindGroup;

    /// Update GPU uniform buffer if parameters changed
    fn update_uniform(&self, queue: &wgpu::Queue);

    /// Check if this system needs instance buffer rebuild
    fn needs_rebuild(&self) -> bool;

    /// Rebuild instance buffer
    fn rebuild(&mut self, device: &wgpu::Device);

    /// Mark as needing rebuild
    fn mark_dirty(&mut self);
}

// ============================================================================
// GRID PARTICLE SYSTEM
// ============================================================================

pub struct GridParticleSystem {
    name: String,
    params: GridParams,
    model_path: String,
    material_key: String,
    instance_buffer: wgpu::Buffer,
    num_instances: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    needs_rebuild: bool,
    last_edit_time: web_time::Instant,
}

impl GridParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        name: String,
        params: GridParams,
        model_path: String,
        material_key: String,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let count = params.rows * params.rows;
        let instances = Self::generate_grid_instances(count, &params);

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Grid System '{}' Instance Buffer", name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform = GridTransformUniform {
            center: params.center,
            spacing: params.spacing,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Grid System '{}' Uniform Buffer", name)),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("Grid System '{}' Bind Group", name)),
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            name,
            params,
            model_path,
            material_key,
            instance_buffer,
            num_instances: instances.len() as u32,
            uniform_buffer,
            bind_group,
            needs_rebuild: false,
            last_edit_time: web_time::Instant::now(),
        }
    }

    fn generate_grid_instances(count: usize, params: &GridParams) -> Vec<InstanceRaw> {
        let rows = params.rows;
        let displacement = Vector3::new(rows as f32 * 0.5, 0.0, rows as f32 * 0.5);

        let mut instances = Vec::with_capacity(count);

        for x in 0..rows {
            for z in 0..rows {
                let position = Vector3::new(x as f32, 0.0, z as f32) - displacement;

                let rotation = if position.magnitude2() < 0.001 {
                    Quaternion::new(1.0, 0.0, 0.0, 0.0)
                } else {
                    let axis = position.normalize();
                    Quaternion::from_axis_angle(axis, cgmath::Rad(std::f32::consts::PI / 4.0))
                };

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

    pub fn params(&self) -> &GridParams {
        &self.params
    }

    pub fn model_path(&self) -> &str {
        &self.model_path
    }

    pub fn material_key(&self) -> &str {
        &self.material_key
    }

    pub fn update_params(&mut self, params: GridParams) {
        let old_rows = self.params.rows;
        self.params = params;

        if old_rows != self.params.rows {
            self.mark_dirty();
        }
    }
}

impl ParticleSystemType for GridParticleSystem {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_instances(&self) -> u32 {
        self.num_instances
    }

    fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buffer
    }

    fn uniform_bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    fn update_uniform(&self, queue: &wgpu::Queue) {
        let uniform = GridTransformUniform {
            center: self.params.center,
            spacing: self.params.spacing,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    fn needs_rebuild(&self) -> bool {
        self.needs_rebuild && self.last_edit_time.elapsed().as_millis() >= DEBOUNCE_MS as u128
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        let count = self.params.rows * self.params.rows;
        let instances = Self::generate_grid_instances(count, &self.params);
        self.num_instances = instances.len() as u32;

        self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Grid System '{}' Instance Buffer", self.name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.needs_rebuild = false;
    }

    fn mark_dirty(&mut self) {
        self.needs_rebuild = true;
        self.last_edit_time = web_time::Instant::now();
    }
}

// ============================================================================
// SPHERE PARTICLE SYSTEM
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SphereParams {
    pub count: usize,
    pub radius: f32,
    pub center: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SphereTransformUniform {
    pub center: [f32; 3],
    pub radius: f32,
}

pub struct SphereParticleSystem {
    name: String,
    params: SphereParams,
    model_path: String,
    material_key: String,
    instance_buffer: wgpu::Buffer,
    num_instances: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    needs_rebuild: bool,
    last_edit_time: web_time::Instant,
}

impl SphereParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        name: String,
        params: SphereParams,
        model_path: String,
        material_key: String,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let instances = Self::generate_sphere_instances(&params);

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Sphere System '{}' Instance Buffer", name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform = SphereTransformUniform {
            center: params.center,
            radius: params.radius,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Sphere System '{}' Uniform Buffer", name)),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("Sphere System '{}' Bind Group", name)),
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            name,
            params,
            model_path,
            material_key,
            instance_buffer,
            num_instances: instances.len() as u32,
            uniform_buffer,
            bind_group,
            needs_rebuild: false,
            last_edit_time: web_time::Instant::now(),
        }
    }

    fn generate_sphere_instances(params: &SphereParams) -> Vec<InstanceRaw> {
        let count = params.count;
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

            let position = Vector3::new(x, y, z); // Unit sphere, shader will scale

            // Rotation to face outward from center
            let up = Vector3::new(0.0, 1.0, 0.0);
            let rotation = if position.magnitude2() > 0.001 {
                let forward = position.normalize();
                let right = forward.cross(up).normalize();
                let new_up = right.cross(forward);

                Quaternion::from_arc(Vector3::new(0.0, 0.0, 1.0), forward, Some(new_up))
            } else {
                Quaternion::new(1.0, 0.0, 0.0, 0.0)
            };

            let model_matrix = Matrix4::from_translation(position) * Matrix4::from(rotation);
            let normal_matrix = Matrix3::from(rotation);

            instances.push(InstanceRaw {
                model: model_matrix.into(),
                normal: normal_matrix.into(),
            });
        }

        instances
    }

    pub fn params(&self) -> &SphereParams {
        &self.params
    }

    pub fn model_path(&self) -> &str {
        &self.model_path
    }

    pub fn material_key(&self) -> &str {
        &self.material_key
    }

    pub fn update_params(&mut self, params: SphereParams) {
        let old_count = self.params.count;
        self.params = params;

        if old_count != self.params.count {
            self.mark_dirty();
        }
    }
}

impl ParticleSystemType for SphereParticleSystem {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_instances(&self) -> u32 {
        self.num_instances
    }

    fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buffer
    }

    fn uniform_bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    fn update_uniform(&self, queue: &wgpu::Queue) {
        let uniform = SphereTransformUniform {
            center: self.params.center,
            radius: self.params.radius,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    fn needs_rebuild(&self) -> bool {
        self.needs_rebuild && self.last_edit_time.elapsed().as_millis() >= DEBOUNCE_MS as u128
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        let instances = Self::generate_sphere_instances(&self.params);
        self.num_instances = instances.len() as u32;

        self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Sphere System '{}' Instance Buffer", self.name)),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.needs_rebuild = false;
    }

    fn mark_dirty(&mut self) {
        self.needs_rebuild = true;
        self.last_edit_time = web_time::Instant::now();
    }
}

// ============================================================================
// PARTICLE SYSTEM MANAGER
// ============================================================================

#[derive(Copy, Clone, Debug)]
pub enum ParticleSystemKind {
    Grid,
    Sphere,
}

pub struct ParticleSystemManager {
    grids: HashMap<String, GridParticleSystem>,
    spheres: HashMap<String, SphereParticleSystem>,
    name_to_kind: HashMap<String, ParticleSystemKind>,
}

impl ParticleSystemManager {
    pub fn new() -> Self {
        Self {
            grids: HashMap::new(),
            spheres: HashMap::new(),
            name_to_kind: HashMap::new(),
        }
    }

    pub fn add_grid(&mut self, name: String, system: GridParticleSystem) {
        self.name_to_kind
            .insert(name.clone(), ParticleSystemKind::Grid);
        self.grids.insert(name, system);
    }

    pub fn add_sphere(&mut self, name: String, system: SphereParticleSystem) {
        self.name_to_kind
            .insert(name.clone(), ParticleSystemKind::Sphere);
        self.spheres.insert(name, system);
    }

    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(kind) = self.name_to_kind.remove(name) {
            match kind {
                ParticleSystemKind::Grid => self.grids.remove(name).is_some(),
                ParticleSystemKind::Sphere => self.spheres.remove(name).is_some(),
            }
        } else {
            false
        }
    }

    pub fn get_kind(&self, name: &str) -> Option<ParticleSystemKind> {
        self.name_to_kind.get(name).copied()
    }

    pub fn get_grid(&self, name: &str) -> Option<&GridParticleSystem> {
        self.grids.get(name)
    }

    pub fn get_grid_mut(&mut self, name: &str) -> Option<&mut GridParticleSystem> {
        self.grids.get_mut(name)
    }

    pub fn get_sphere(&self, name: &str) -> Option<&SphereParticleSystem> {
        self.spheres.get(name)
    }

    pub fn get_sphere_mut(&mut self, name: &str) -> Option<&mut SphereParticleSystem> {
        self.spheres.get_mut(name)
    }

    pub fn grids(&self) -> impl Iterator<Item = (&String, &GridParticleSystem)> {
        self.grids.iter()
    }

    pub fn grids_mut(&mut self) -> impl Iterator<Item = (&String, &mut GridParticleSystem)> {
        self.grids.iter_mut()
    }

    pub fn spheres(&self) -> impl Iterator<Item = (&String, &SphereParticleSystem)> {
        self.spheres.iter()
    }

    pub fn spheres_mut(&mut self) -> impl Iterator<Item = (&String, &mut SphereParticleSystem)> {
        self.spheres.iter_mut()
    }

    pub fn all_names(&self) -> impl Iterator<Item = &String> {
        self.name_to_kind.keys()
    }

    pub fn count(&self) -> usize {
        self.grids.len() + self.spheres.len()
    }
}
