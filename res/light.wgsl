struct Camera {
    view_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;

struct Light {
    position: vec4<f32>,
    color: vec4<f32>,
}

const MAX_LIGHTS: u32 = 10u;

struct LightData {
    lights: array<Light, MAX_LIGHTS>,
    num_lights: u32,
}

@group(0) @binding(1)
var<uniform> light_data: LightData;

//

struct VertexInput {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(model: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let scale = 0.25;
    let light = light_data.lights[instance_index];
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(model.position * scale + light.position.xyz, 1.0);
    out.color = light.color.xyz;
    return out;
}

//

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
