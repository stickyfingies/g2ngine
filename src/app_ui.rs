use crate::particle_system::{ParticleSystemManager, ParticleSystemType};
use crate::state::LightManager;
use egui::{Align2, Context};

pub struct UiActions {
    pub save_requested: bool,
    pub load_requested: bool,
}

impl Default for UiActions {
    fn default() -> Self {
        Self {
            save_requested: false,
            load_requested: false,
        }
    }
}

pub fn app_ui(
    ctx: &Context,
    clear_color: &mut wgpu::Color,
    particle_system_manager: &mut ParticleSystemManager,
    light_manager: &mut LightManager,
    light_buffer: &wgpu::Buffer,
    delta_time_ms: f32,
    queue: &wgpu::Queue,
    device: &wgpu::Device,
    particle_uniform_bind_group_layout: &wgpu::BindGroupLayout,
) -> UiActions {
    let mut actions = UiActions::default();
    egui::Window::new("Scene Editor")
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

            // NEW Particle System Manager
            ui.collapsing(
                format!("Particle Systems ({})", particle_system_manager.count()),
                |ui| {
                    // --- Add Particle System Buttons ---
                    ui.horizontal(|ui| {
                        if ui.button("➕ Add Grid").clicked() {
                            let name = format!("Grid_{}", particle_system_manager.count());
                            let params = crate::particle_system::GridParams {
                                rows: 10,
                                spacing: 1.0,
                                center: [0.0, 0.0, 0.0],
                            };
                            let grid = crate::particle_system::GridParticleSystem::new(
                                device,
                                name.clone(),
                                params,
                                "teapot.obj".to_string(),
                                particle_uniform_bind_group_layout,
                            );
                            particle_system_manager.add_grid(name, grid);
                        }

                        if ui.button("➕ Add Sphere").clicked() {
                            let name = format!("Sphere_{}", particle_system_manager.count());
                            let params = crate::particle_system::SphereParams {
                                count: 1000,
                                radius: 5.0,
                                center: [0.0, 0.0, 0.0],
                            };
                            let sphere = crate::particle_system::SphereParticleSystem::new(
                                device,
                                name.clone(),
                                params,
                                "teapot.obj".to_string(),
                                particle_uniform_bind_group_layout,
                            );
                            particle_system_manager.add_sphere(name, sphere);
                        }
                    });

                    ui.separator();

                    // --- Grid Systems ---
                    ui.label("Grid Systems:");

                    let mut grid_to_remove = None;
                    for (name, grid) in particle_system_manager.grids_mut() {
                        ui.push_id(name, |ui| {
                            ui.horizontal(|ui| {
                                let header = egui::CollapsingHeader::new(name).default_open(false);

                                if header
                                    .show(ui, |ui| {
                                        ui.label(format!("Instances: {}", grid.num_instances()));

                                        let mut params = grid.params().clone();
                                        let mut params_changed = false;
                                        let mut uniform_changed = false;

                                        ui.separator();
                                        ui.label("Parameters:");

                                        ui.horizontal(|ui| {
                                            ui.label("Rows:");
                                            if ui
                                                .add(egui::Slider::new(&mut params.rows, 5..=50))
                                                .changed()
                                            {
                                                params_changed = true;
                                            }
                                        });

                                        ui.horizontal(|ui| {
                                            ui.label("Spacing:");
                                            if ui
                                                .add(egui::Slider::new(
                                                    &mut params.spacing,
                                                    0.5..=10.0,
                                                ))
                                                .changed()
                                            {
                                                params_changed = true;
                                                uniform_changed = true;
                                            }
                                        });

                                        ui.label("Center:");
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut params.center[0],
                                                    -50.0..=50.0,
                                                )
                                                .text("X"),
                                            )
                                            .changed()
                                        {
                                            params_changed = true;
                                            uniform_changed = true;
                                        }
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut params.center[1],
                                                    -50.0..=50.0,
                                                )
                                                .text("Y"),
                                            )
                                            .changed()
                                        {
                                            params_changed = true;
                                            uniform_changed = true;
                                        }
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut params.center[2],
                                                    -50.0..=50.0,
                                                )
                                                .text("Z"),
                                            )
                                            .changed()
                                        {
                                            params_changed = true;
                                            uniform_changed = true;
                                        }

                                        if params_changed {
                                            grid.update_params(params);
                                            if uniform_changed {
                                                grid.update_uniform(queue);
                                            }
                                        }
                                    })
                                    .body_returned
                                    .is_some()
                                {
                                    if ui.button("🗑").clicked() {
                                        grid_to_remove = Some(name.clone());
                                    }
                                }
                            });
                        });
                    }

                    if let Some(name) = grid_to_remove {
                        particle_system_manager.remove(&name);
                    }

                    ui.separator();

                    // --- Sphere Systems ---
                    ui.label("Sphere Systems:");

                    let mut sphere_to_remove = None;
                    for (name, sphere) in particle_system_manager.spheres_mut() {
                        ui.push_id(name, |ui| {
                            ui.horizontal(|ui| {
                                let header = egui::CollapsingHeader::new(name).default_open(false);

                                if header
                                    .show(ui, |ui| {
                                        ui.label(format!("Instances: {}", sphere.num_instances()));

                                        let mut params = sphere.params().clone();
                                        let mut params_changed = false;
                                        let mut uniform_changed = false;

                                        ui.separator();
                                        ui.label("Parameters:");

                                        ui.horizontal(|ui| {
                                            ui.label("Count:");
                                            if ui
                                                .add(egui::Slider::new(
                                                    &mut params.count,
                                                    100..=5000,
                                                ))
                                                .changed()
                                            {
                                                params_changed = true;
                                            }
                                        });

                                        ui.horizontal(|ui| {
                                            ui.label("Radius:");
                                            if ui
                                                .add(egui::Slider::new(
                                                    &mut params.radius,
                                                    1.0..=20.0,
                                                ))
                                                .changed()
                                            {
                                                params_changed = true;
                                                uniform_changed = true;
                                            }
                                        });

                                        ui.label("Center:");
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut params.center[0],
                                                    -50.0..=50.0,
                                                )
                                                .text("X"),
                                            )
                                            .changed()
                                        {
                                            params_changed = true;
                                            uniform_changed = true;
                                        }
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut params.center[1],
                                                    -50.0..=50.0,
                                                )
                                                .text("Y"),
                                            )
                                            .changed()
                                        {
                                            params_changed = true;
                                            uniform_changed = true;
                                        }
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut params.center[2],
                                                    -50.0..=50.0,
                                                )
                                                .text("Z"),
                                            )
                                            .changed()
                                        {
                                            params_changed = true;
                                            uniform_changed = true;
                                        }

                                        if params_changed {
                                            sphere.update_params(params);
                                            if uniform_changed {
                                                sphere.update_uniform(queue);
                                            }
                                        }
                                    })
                                    .body_returned
                                    .is_some()
                                {
                                    if ui.button("🗑").clicked() {
                                        sphere_to_remove = Some(name.clone());
                                    }
                                }
                            });
                        });
                    }

                    if let Some(name) = sphere_to_remove {
                        particle_system_manager.remove(&name);
                    }
                },
            );

            ui.separator();

            // Save/Load World
            ui.collapsing("💾 Save/Load World", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("💾 Save World").clicked() {
                        actions.save_requested = true;
                    }

                    if ui.button("📂 Load World").clicked() {
                        actions.load_requested = true;
                    }
                });

                ui.label("Saves to: world.json");
            });

            ui.separator();

            ui.label(format!("Delta Time: {:.2} ms", delta_time_ms));
            ui.label(format!("FPS: {:.1}", 1000.0 / delta_time_ms));
        });

    actions
}
