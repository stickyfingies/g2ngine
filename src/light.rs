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
