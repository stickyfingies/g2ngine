# Flexible Material System - Implementation Roadmap

## Overview

This document outlines the critical path for migrating from a single-shader, hardcoded material system to a flexible multi-shader material system that supports:
- Multiple shaders per scene
- Different bind group layouts per shader
- Runtime shader loading
- Material editing and customization
- Saving/loading custom materials

---

## Critical Changes Required

### **Change #1: Material Struct - Data + GPU Resources Split**
**Pain Level: 5/5**  
**Files: `model.rs`, `state.rs`**

**Current (Blocks Everything):**
```rust
pub struct Material {
    pub name: String,
    pub diffuse_texture: GpuTexture,  // ❌ Hardcoded to one texture
    pub bind_group: wgpu::BindGroup,   // ❌ Locked to one layout
}
```

**Must Become:**
```rust
pub struct Material {
    pub name: String,
    pub shader: String,                              // NEW: "pbr.wgsl"
    pub textures: HashMap<String, GpuTexture>,       // NEW: flexible texture slots
    pub bind_group: wgpu::BindGroup,
}
```

**Why Critical:**
- Blocks everything else - can't support different shaders until materials can describe what shader they need
- Breaks all material creation code (model loading, default material, etc.)
- Breaks serialization (world.json)

**Migration Path:**
```rust
// Step 1: Add fields with defaults
pub struct Material {
    pub name: String,
    pub shader: String,                              // "shader.wgsl" for now
    pub diffuse_texture: GpuTexture,                 // Keep for compatibility
    pub textures: HashMap<String, GpuTexture>,       // Empty for now
    pub bind_group: wgpu::BindGroup,
}

// Step 2: Migrate diffuse_texture → textures["diffuse"]
// Step 3: Remove diffuse_texture field
```

---

### **Change #2: One Pipeline → Pipeline per Shader**
**Pain Level: 5/5**  
**Files: `state.rs`**

**Current (Blocks Rendering):**
```rust
pub struct State {
    render_pipeline: wgpu::RenderPipeline,  // ❌ ONE for everything
}

// In render():
render_pass.set_pipeline(&self.render_pipeline);  // ❌ Never changes
for system in systems {
    render_pass.draw(system);
}
```

**Must Become:**
```rust
pub struct State {
    pipeline_cache: HashMap<String, wgpu::RenderPipeline>,  // NEW: shader → pipeline
}

// In render():
let mut current_shader: Option<&str> = None;
for system in systems {
    let material = self.materials.get(system.material_key()).unwrap();
    
    // Switch pipeline when shader changes
    if current_shader != Some(&material.shader) {
        let pipeline = self.pipeline_cache.get(&material.shader).unwrap();
        render_pass.set_pipeline(pipeline);
        current_shader = Some(&material.shader);
    }
    
    render_pass.draw(system);
}
```

**Why Critical:**
- Can't render different shaders without pipeline switching
- Blocks multi-shader support entirely
- Performance critical - must batch by shader

**Migration Path:**
```rust
// Step 1: Convert single pipeline to HashMap
pipeline_cache.insert("shader.wgsl".to_string(), render_pipeline);

// Step 2: Add pipeline switching logic (always switches to same pipeline for now)

// Step 3: Add support for loading multiple shaders
```

---

### **Change #3: load_model() Signature - Remove Hardcoded Layout**
**Pain Level: 4/5**  
**Files: `model.rs`, `state.rs` (all async loading code)**

**Current (Blocks Dynamic Materials):**
```rust
pub async fn load_model(
    file_name: &str,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,  // ❌ Assumes all materials use this
) -> Result<(Model, HashMap<String, Material>)>
```

**Must Become:**
```rust
pub async fn load_model(
    file_name: &str,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout_provider: &dyn BindGroupLayoutProvider,  // NEW: can provide different layouts
) -> Result<(Model, HashMap<String, Material>)>

pub trait BindGroupLayoutProvider {
    fn get_layout(&self, shader: &str) -> &wgpu::BindGroupLayout;
}
```

**Why Critical:**
- Blocks per-material shaders - can't create bind groups for different layouts
- Touches all async loading (desktop threads + web spawn_local)
- Breaks channel signatures (model loading results)

**Migration Path:**
```rust
// Step 1: Create simple provider that always returns texture_bind_group_layout
struct DefaultLayoutProvider {
    layout: wgpu::BindGroupLayout,
}

impl BindGroupLayoutProvider for DefaultLayoutProvider {
    fn get_layout(&self, _shader: &str) -> &wgpu::BindGroupLayout {
        &self.layout  // Always returns same layout for now
    }
}

// Step 2: Replace all load_model() calls with provider

// Step 3: Make provider actually return different layouts per shader
```

---

### **Change #4: Shader Loading Infrastructure**
**Pain Level: 3/5**  
**Files: `state.rs` (new), `resources.rs` (extend)**

**Current (No Dynamic Shader Support):**
```rust
// Shaders loaded once in State::new()
let shader_source = resources::load_string("shader.wgsl").await.unwrap();
let shader = device.create_shader_module(...);
```

**Must Become:**
```rust
pub struct ShaderRegistry {
    shaders: HashMap<String, wgpu::ShaderModule>,
    pending_loads: HashSet<String>,
    in_flight_loads: HashSet<String>,
    // Same pattern as model loading!
}

impl ShaderRegistry {
    pub fn request(&mut self, path: String) { ... }
    pub fn get(&self, path: &str) -> Option<&wgpu::ShaderModule> { ... }
    pub fn poll_loads(&mut self, device: &Device, receiver: &Receiver<...>) { ... }
}
```

**Why Critical:**
- Blocks loading shaders at runtime - materials can't reference shaders that aren't loaded
- Blocks fallback shader - need "error.wgsl" preloaded
- Blocks material editing - changing shader requires loading new shader

**Migration Path:**
```rust
// Step 1: Create ShaderRegistry, populate with "shader.wgsl" and "light.wgsl"

// Step 2: Add async loading (same pattern as models - pending/in-flight/channel)

// Step 3: Create fallback "error.wgsl" shader (magenta)
```

---

### **Change #5: Bind Group Layout per Shader**
**Pain Level: 4/5**  
**Files: `state.rs`, `model.rs`**

**Current (Blocks Different Shader Layouts):**
```rust
// ONE layout for all materials
let texture_bind_group_layout = device.create_bind_group_layout(&...);
```

**Must Become:**
```rust
pub struct BindGroupLayoutCache {
    layouts: HashMap<String, wgpu::BindGroupLayout>,  // shader → layout
}

impl BindGroupLayoutCache {
    pub fn get_or_create(&mut self, shader: &str, device: &Device) -> &wgpu::BindGroupLayout {
        self.layouts.entry(shader.to_string()).or_insert_with(|| {
            // Parse shader or use metadata to determine layout
            create_layout_for_shader(shader, device)
        })
    }
}
```

**Why Critical:**
- Different shaders need different bind group layouts (PBR has 3 textures, unlit has 1)
- Pipelines must match layouts - pipeline creation needs the right layout
- Materials must create bind groups with correct layout for their shader

**Migration Path:**
```rust
// Step 1: Cache the current layout under "shader.wgsl"
layouts.insert("shader.wgsl", texture_bind_group_layout);

// Step 2: Look up layout by material.shader when creating bind groups

// Step 3: Add layout definitions for new shaders
```

---

### **Change #6: World Serialization - Add Shader Field**
**Pain Level: 2/5**  
**Files: `world.rs`, `state.rs`**

**Current (Can't Save Material Type):**
```rust
pub struct ParticleSystemData {
    pub model: String,
    pub material_key: String,  // ❌ Just a key, no shader info
}
```

**Must Become:**
```rust
pub struct ParticleSystemData {
    pub model: String,
    pub material_key: String,
    pub material_shader: Option<String>,  // NEW: "pbr.wgsl" (for instances)
}
```

Or better:
```rust
pub enum MaterialReference {
    Canonical { key: String },                    // "teapot/default"
    Instance { shader: String, textures: ... },   // Full material definition
}

pub struct ParticleSystemData {
    pub model: String,
    pub material: MaterialReference,  // NEW
}
```

**Why Critical:**
- Can't save/load custom materials without shader info
- Blocks copy-on-write instances - no way to persist modified materials
- Blocks material editing - changes aren't saved

**Migration Path:**
```rust
// Step 1: Add material_shader field with #[serde(default)]
pub material_shader: Option<String>,  // None = use model's material shader

// Step 2: Save shader when exporting, load when importing

// Step 3: Migrate to MaterialReference enum
```

---

## Implementation Order (Critical Path)

### **Week 1: Foundation**
- **Change #4:** ShaderRegistry (load shaders dynamically)
- **Change #5:** BindGroupLayoutCache (layouts per shader)
- **Change #1:** Material struct (add shader field + textures map)
- **Bonus:** Create fallback "error.wgsl" shader (magenta)

### **Week 2: Rendering**
- **Change #2:** Pipeline cache (one per shader)
- **Change #3:** load_model() signature (layout provider)

### **Week 3: Persistence**
- **Change #6:** World serialization (material references)

---

## Success Criteria

After implementing these 6 changes, you should be able to:

```rust
// Create a material with custom shader
let material = Material {
    name: "my_pbr".to_string(),
    shader: "pbr.wgsl".to_string(),
    textures: HashMap::from([
        ("albedo", albedo_texture),
        ("normal", normal_texture),
    ]),
    bind_group: /* created with pbr's layout */,
};

// Render it
render_pass.set_pipeline(pipeline_cache.get("pbr.wgsl"));
render_pass.set_bind_group(0, &material.bind_group, &[]);
render_pass.draw(...);

// Save it
world.materials.insert("my_pbr", material.serialize());
```

**Capabilities Unlocked:**
- ✅ Multiple shaders
- ✅ Different bind group layouts per shader
- ✅ Runtime shader loading
- ✅ Material editing (shader swapping)
- ✅ Saving custom materials
- ✅ Fallback rendering when shaders fail

---

## Future Enhancements (Not Critical)

These features build on top of the core system:

| Feature | Required? | When? |
|---------|-----------|-------|
| Copy-on-write instances | No | After core system works |
| Material editing UI | No | After serialization works |
| Pipeline batching/sorting | No | Performance optimization later |
| Material properties/uniforms | No | Future enhancement |
| Shader hot-reload | No | Nice-to-have |
| Instance diffing | No | Optimization |

---

## Key Architectural Decisions

### **Material Classes (Option B)**
Materials using the same shader MUST have the same bind group layout. This simplifies pipeline management and reduces pipeline count.

### **Fallback Shader (Option A)**
When shader compilation fails or shader is missing, use a fallback "error.wgsl" shader (bright magenta) for visibility.

### **Copy-on-Write Materials**
When a user edits a canonical material (from a model), create an instance:
- Canonical materials track the source model
- Instances are independent copies saved in world.json
- Reloading a model updates canonical materials but preserves instances

---

## Notes

This roadmap focuses on the **minimum viable changes** to enable flexible materials. Everything else (copy-on-write, material editing UI, advanced batching) can be added incrementally after the foundation is solid.

The design avoids premature modularization - we'll see where natural boundaries emerge after the core system is working.
