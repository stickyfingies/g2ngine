use crate::particle_system::GeneratorType;
use serde::{Deserialize, Serialize};

/// Serializable custom material data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMaterialData {
    pub name: String,
    pub texture_path: String,
    pub color: [f32; 4],
}

/// Serializable representation of the entire game world state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldData {
    pub background_color: [f32; 4],
    pub camera: CameraData,
    pub lights: Vec<LightParams>,
    pub particle_systems: Vec<ParticleSystemData>,
    #[serde(default)]
    pub custom_materials: Vec<CustomMaterialData>,
}

impl Default for WorldData {
    fn default() -> Self {
        Self {
            background_color: [0.1, 0.2, 0.3, 1.0],
            camera: CameraData::default(),
            lights: vec![],
            particle_systems: vec![],
            custom_materials: vec![],
        }
    }
}

/// Camera position and view parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraData {
    pub position: [f32; 3],
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub fovy_deg: f32,
    pub znear: f32,
    pub zfar: f32,
}

impl Default for CameraData {
    fn default() -> Self {
        Self {
            position: [0.0, 5.0, 10.0],
            yaw_deg: -90.0,
            pitch_deg: -20.0,
            fovy_deg: 45.0,
            znear: 0.1,
            zfar: 1000.0,
        }
    }
}

/// Light source parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightParams {
    pub position: [f32; 3],
    pub color: [f32; 4],
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_mesh_index")]
    pub mesh_index: usize,
    pub material_source: crate::model::MaterialSource,
}

/// Particle system configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleSystemData {
    pub name: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_mesh_index")]
    pub mesh_index: usize,
    pub material_source: crate::model::MaterialSource,
    pub generator: GeneratorType,
}

fn default_model() -> String {
    crate::defaults::PARTICLE_SYSTEM_MODEL_PATH.to_string()
}

fn default_mesh_index() -> usize {
    0
}

impl ParticleSystemData {
    pub fn name(&self) -> &str {
        &self.name
    }
}
