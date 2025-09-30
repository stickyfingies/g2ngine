use crate::particle_system::{GridParams, SphereParams};
use serde::{Deserialize, Serialize};

/// Serializable representation of the entire game world state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldData {
    pub background_color: [f32; 4],
    pub camera: CameraData,
    pub lights: Vec<LightData>,
    pub particle_systems: Vec<ParticleSystemData>,
}

impl Default for WorldData {
    fn default() -> Self {
        Self {
            background_color: [0.1, 0.2, 0.3, 1.0],
            camera: CameraData::default(),
            lights: vec![],
            particle_systems: vec![],
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

/// Light source data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightData {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Particle system configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParticleSystemData {
    #[serde(rename = "grid")]
    Grid { name: String, params: GridParams },
    #[serde(rename = "sphere")]
    Sphere { name: String, params: SphereParams },
}

impl ParticleSystemData {
    pub fn name(&self) -> &str {
        match self {
            ParticleSystemData::Grid { name, .. } => name,
            ParticleSystemData::Sphere { name, .. } => name,
        }
    }
}
