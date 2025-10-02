use crate::{
    resources::{load_binary, load_string},
    texture::GpuTexture,
};
use std::{
    cell::RefCell,
    io::{BufReader, Cursor},
    ops::Range,
    sync::Arc,
};
use wgpu::util::DeviceExt;

pub trait Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static>;
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub normal: [f32; 3],
}

impl Vertex for ModelVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use wgpu::{
            BufferAddress, VertexAttribute, VertexBufferLayout, VertexFormat, VertexStepMode,
        };
        VertexBufferLayout {
            array_stride: std::mem::size_of::<ModelVertex>() as BufferAddress,
            step_mode: VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as BufferAddress,
                    shader_location: 1,
                    format: VertexFormat::Float32x2,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 5]>() as BufferAddress,
                    shader_location: 2,
                    format: VertexFormat::Float32x3,
                },
            ],
        }
    }
}

pub struct Model {
    pub name: String,
    pub meshes: Vec<Mesh>,
    pub material_keys: Vec<String>,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialProperties {
    pub color: [f32; 4],
}

impl Default for MaterialProperties {
    fn default() -> Self {
        Self {
            color: [1.0, 1.0, 1.0, 1.0], // Default to white (no tint)
        }
    }
}

/// CPU-side material description (serializable, GPU-agnostic)
#[derive(Debug, Clone)]
pub struct MaterialDesc {
    pub name: String,
    pub texture_path: String,
    pub properties: RefCell<MaterialProperties>,
}

/// GPU realization of a material
#[allow(dead_code)]
pub struct GpuMaterial {
    pub desc: MaterialDesc,
    pub diffuse_texture: Arc<GpuTexture>,
    pub properties_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}

pub struct Mesh {
    pub name: String,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_elements: u32,
    pub vertex_count: u32,
    pub material_key: String,
}

pub async fn load_model(
    file_name: &str,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    texture_registry: &Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<GpuTexture>>>>,
) -> anyhow::Result<(Model, std::collections::HashMap<String, GpuMaterial>)> {
    let obj_text = load_string(file_name).await?;
    let obj_cursor = Cursor::new(obj_text);
    let mut obj_reader = BufReader::new(obj_cursor);

    let (models, obj_materials) = tobj::load_obj_buf_async(
        &mut obj_reader,
        &tobj::LoadOptions {
            triangulate: true,
            single_index: true,
            ..Default::default()
        },
        |path| async move {
            let mat_text = load_string(&path).await.unwrap();
            tobj::load_mtl_buf(&mut BufReader::new(Cursor::new(mat_text)))
        },
    )
    .await?;

    // Extract model name from file path (e.g., "teapot.obj" -> "teapot")
    let model_name = file_name
        .split('/')
        .last()
        .unwrap_or(file_name)
        .trim_end_matches(".obj");

    let mut materials_map = std::collections::HashMap::new();
    let mut material_keys = Vec::new();

    for mat in obj_materials? {
        let material_key = format!("{}/{}", model_name, mat.name);
        let diffuse_texture_filename = &mat.diffuse_texture;

        // Check if texture already exists in registry, otherwise load it
        let diffuse_texture = {
            let mut registry = texture_registry.lock().unwrap();
            if let Some(existing_texture) = registry.get(diffuse_texture_filename) {
                Arc::clone(existing_texture)
            } else {
                let diffuse_texture_bytes = load_binary(&diffuse_texture_filename).await?;
                let texture = Arc::new(GpuTexture::from_bytes(
                    device,
                    queue,
                    &diffuse_texture_bytes,
                    diffuse_texture_filename,
                )?);
                registry.insert(diffuse_texture_filename.clone(), Arc::clone(&texture));
                texture
            }
        };

        let desc = MaterialDesc {
            name: mat.name.clone(),
            texture_path: diffuse_texture_filename.clone(),
            properties: RefCell::new(MaterialProperties::default()),
        };

        let properties_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}_properties", mat.name)),
            contents: bytemuck::cast_slice(&[*desc.properties.borrow()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(mat.name.as_str()),
            layout,
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

        materials_map.insert(
            material_key.clone(),
            GpuMaterial {
                desc,
                diffuse_texture,
                properties_buffer,
                bind_group,
            },
        );
        material_keys.push(material_key);
    }

    // If no materials were loaded, use the default material
    if materials_map.is_empty() {
        material_keys.push("default".to_string());
    }

    let meshes = models
        .into_iter()
        .map(|model| {
            let vertices = (0..model.mesh.positions.len() / 3)
                .map(|i| {
                    let normal = if model.mesh.normals.is_empty() {
                        [0.0, 0.0, 0.0]
                    } else {
                        [
                            model.mesh.normals[i * 3],
                            model.mesh.normals[i * 3 + 1],
                            model.mesh.normals[i * 3 + 2],
                        ]
                    };
                    let tex_coords = if model.mesh.texcoords.is_empty() {
                        [0.0, 0.0]
                    } else {
                        [
                            model.mesh.texcoords[i * 2],
                            1.0 - model.mesh.texcoords[i * 2 + 1],
                        ]
                    };
                    ModelVertex {
                        position: [
                            model.mesh.positions[i * 3],
                            model.mesh.positions[i * 3 + 1],
                            model.mesh.positions[i * 3 + 2],
                        ],
                        tex_coords,
                        normal,
                    }
                })
                .collect::<Vec<_>>();

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{:?} Vertex Buffer", file_name)),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{:?} Index Buffer", file_name)),
                contents: bytemuck::cast_slice(&model.mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });

            let material_key = match model.mesh.material_id {
                Some(material_index) => material_keys
                    .get(material_index)
                    .cloned()
                    .unwrap_or_else(|| "default".to_string()),
                None => "default".to_string(),
            };

            Mesh {
                name: file_name.to_string(),
                vertex_buffer,
                index_buffer,
                num_elements: model.mesh.indices.len() as u32,
                vertex_count: vertices.len() as u32,
                material_key,
            }
        })
        .collect::<Vec<_>>();

    Ok((
        Model {
            name: model_name.to_string(),
            meshes,
            material_keys,
        },
        materials_map,
    ))
}

pub trait DrawModel<'a> {
    fn draw_mesh_instanced(
        &mut self,
        mesh: &'a Mesh,
        material: &'a GpuMaterial,
        instances: Range<u32>,
        per_frame_bind_group: &'a wgpu::BindGroup,
    );
}

impl<'a, 'b> DrawModel<'b> for wgpu::RenderPass<'a>
where
    'b: 'a,
{
    fn draw_mesh_instanced(
        &mut self,
        mesh: &'b Mesh,
        material: &'b GpuMaterial,
        instances: Range<u32>,
        per_frame_bind_group: &'b wgpu::BindGroup,
    ) {
        self.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        self.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.set_bind_group(0, per_frame_bind_group, &[]);
        self.set_bind_group(1, &material.bind_group, &[]);
        self.draw_indexed(0..mesh.num_elements, 0, instances);
    }
}

pub trait DrawLight<'a> {
    fn draw_light_mesh_instanced(
        &mut self,
        mesh: &'a Mesh,
        instances: Range<u32>,
        per_frame_bind_group: &'a wgpu::BindGroup,
    );
}

impl<'a, 'b> DrawLight<'b> for wgpu::RenderPass<'a>
where
    'b: 'a,
{
    fn draw_light_mesh_instanced(
        &mut self,
        mesh: &'b Mesh,
        instances: Range<u32>,
        per_frame_bind_group: &'b wgpu::BindGroup,
    ) {
        self.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        self.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.set_bind_group(0, per_frame_bind_group, &[]);
        self.draw_indexed(0..mesh.num_elements, 0, instances);
    }
}
