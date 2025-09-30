use crate::particle_system::{ParticleSystem, ParticleSystemDesc};
use crate::state::LightManager;
use egui::{Align2, Context};

#[derive(Default)]
pub struct UIState {
    pub checkbox_value: bool,
    pub slider_value: f32,
}

pub fn app_ui(
    ctx: &Context,
    ui_state: &mut UIState,
    clear_color: &mut wgpu::Color,
    particle_system: &mut ParticleSystem,
    light_manager: &mut LightManager,
    light_buffer: &wgpu::Buffer,
    delta_time_ms: f32,
    queue: &wgpu::Queue,
) {
    egui::Window::new("Demo GUI")
        .default_open(true)
        .max_width(400.0)
        .max_height(600.0)
        .default_width(300.0)
        .resizable(true)
        .anchor(Align2::LEFT_TOP, [10.0, 10.0])
        .show(ctx, |ui| {
            ui.heading("Gengine 2");

            ui.separator();

            // Background color picker
            ui.label("Background Color:");
            let mut color = [
                clear_color.r as f32,
                clear_color.g as f32,
                clear_color.b as f32,
                clear_color.a as f32,
            ];
            if ui.color_edit_button_rgba_unmultiplied(&mut color).changed() {
                clear_color.r = color[0].clamp(0.0, 1.0) as f64;
                clear_color.g = color[1].clamp(0.0, 1.0) as f64;
                clear_color.b = color[2].clamp(0.0, 1.0) as f64;
                clear_color.a = color[3].clamp(0.0, 1.0) as f64;
            }

            ui.separator();

            // Particle System info
            ui.collapsing("Particle System", |ui| {
                ui.label(format!("Name: {}", particle_system.config.name));
                ui.label(format!(
                    "Instance Count: {}",
                    particle_system.render.num_instances
                ));

                let mut needs_buffer_rebuild = false;
                let mut needs_uniform_update = false;

                match &mut particle_system.config.desc {
                    ParticleSystemDesc::Grid { count, params } => {
                        ui.label(format!("Type: Grid"));

                        ui.separator();
                        ui.label("Parameters:");

                        // Editable rows slider - triggers buffer rebuild
                        ui.horizontal(|ui| {
                            ui.label("Rows:");
                            if ui
                                .add(egui::Slider::new(&mut params.rows, 5..=50))
                                .changed()
                            {
                                // Update count based on rows
                                *count = params.rows * params.rows;
                                needs_buffer_rebuild = true;
                            }
                        });

                        // Editable spacing slider - updates uniform immediately
                        ui.horizontal(|ui| {
                            ui.label("Spacing:");
                            if ui
                                .add(egui::Slider::new(&mut params.spacing, 0.5..=10.0))
                                .changed()
                            {
                                needs_uniform_update = true;
                            }
                        });

                        // Editable center sliders - update uniform immediately
                        ui.label("Center:");
                        needs_uniform_update |= ui
                            .add(egui::Slider::new(&mut params.center[0], -50.0..=50.0).text("X"))
                            .changed();
                        needs_uniform_update |= ui
                            .add(egui::Slider::new(&mut params.center[1], -50.0..=50.0).text("Y"))
                            .changed();
                        needs_uniform_update |= ui
                            .add(egui::Slider::new(&mut params.center[2], -50.0..=50.0).text("Z"))
                            .changed();

                        ui.separator();
                        ui.label(format!("Target Count: {}", count));

                        // Update GPU uniform immediately for spacing/center changes
                        if needs_uniform_update {
                            particle_system.update_grid_uniform(queue);
                        }
                    }
                }

                if needs_buffer_rebuild {
                    particle_system.mark_dirty();
                }
            });

            ui.separator();

            // Light Manager
            ui.collapsing(
                format!(
                    "Lights ({}/{})",
                    light_manager.num_lights(),
                    light_manager.max_lights()
                ),
                |ui| {
                    let mut needs_gpu_sync = false;

                    // Add light button
                    if ui.button("➕ Add Light").clicked() {
                        if let Some(_idx) =
                            light_manager.add_light([0.0, 3.0, 0.0], [1.0, 1.0, 1.0, 1.0])
                        {
                            needs_gpu_sync = true;
                        }
                    }

                    ui.separator();

                    let mut to_remove = None;

                    // Iterate through all possible light slots
                    for i in 0..light_manager.max_lights() {
                        if let Some(light) = light_manager.get_light(i) {
                            // Copy light data to avoid borrow checker issues
                            let mut pos = [light.position[0], light.position[1], light.position[2]];
                            let mut color = light.color;

                            ui.push_id(i, |ui| {
                                ui.horizontal(|ui| {
                                    let header =
                                        egui::CollapsingHeader::new(format!("Light {}", i))
                                            .default_open(false);

                                    if header
                                        .show(ui, |ui| {
                                            ui.label("Position:");
                                            let pos_changed = ui
                                                .add(
                                                    egui::Slider::new(&mut pos[0], -20.0..=20.0)
                                                        .text("X"),
                                                )
                                                .changed()
                                                | ui.add(
                                                    egui::Slider::new(&mut pos[1], -20.0..=20.0)
                                                        .text("Y"),
                                                )
                                                .changed()
                                                | ui.add(
                                                    egui::Slider::new(&mut pos[2], -20.0..=20.0)
                                                        .text("Z"),
                                                )
                                                .changed();

                                            ui.label("Color:");
                                            let color_changed = ui
                                                .color_edit_button_rgba_unmultiplied(&mut color)
                                                .changed();

                                            if pos_changed || color_changed {
                                                needs_gpu_sync = true;
                                            }
                                        })
                                        .body_returned
                                        .is_some()
                                    {
                                        // Delete button next to the header
                                        if ui.button("🗑").clicked() {
                                            to_remove = Some(i);
                                        }
                                    }
                                });
                            });

                            // Update light after UI interaction
                            light_manager.update_light(i, pos, color);
                        }
                    }

                    // Remove light if delete was clicked
                    if let Some(idx) = to_remove {
                        light_manager.remove_light(idx);
                        needs_gpu_sync = true;
                    }

                    // Sync to GPU if anything changed
                    if needs_gpu_sync {
                        let light_data = light_manager.sync_to_gpu();
                        queue.write_buffer(light_buffer, 0, bytemuck::cast_slice(&[light_data]));
                    }
                },
            );

            ui.separator();

            ui.label(format!("Delta Time: {:.2} ms", delta_time_ms));
            ui.label(format!("FPS: {:.1}", 1000.0 / delta_time_ms));
        });
}
