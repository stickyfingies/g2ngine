use crate::particle_system::{GridTransformUniform, ParticleSystem, ParticleSystemDesc};
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
            ui.heading("wgpu + egui Integration");

            ui.separator();

            ui.label("This is a demo of egui running on top of your existing wgpu 3D scene!");

            if ui.button("Click me!").clicked() {
                log::info!("Button clicked!");
            }

            ui.separator();

            ui.label("You can add various controls here:");

            // Add some demo controls
            ui.checkbox(&mut ui_state.checkbox_value, "Example checkbox");

            ui.horizontal(|ui| {
                ui.label("Slider:");
                ui.add(egui::Slider::new(&mut ui_state.slider_value, 0.0..=1.0).text("value"));
            });

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
                            let uniform = GridTransformUniform {
                                center: params.center,
                                spacing: params.spacing,
                            };
                            queue.write_buffer(
                                &particle_system.render.grid_transform_buffer,
                                0,
                                bytemuck::cast_slice(&[uniform]),
                            );
                        }
                    }
                }

                if needs_buffer_rebuild {
                    particle_system.mark_dirty();
                }
            });

            ui.separator();

            ui.label(format!("Delta Time: {:.2} ms", delta_time_ms));
            ui.label(format!("FPS: {:.1}", 1000.0 / delta_time_ms));

            ui.separator();

            ui.label("Your 3D scene continues to render in the background!");
            ui.label("Camera controls should still work when not over the GUI.");
        });
}
