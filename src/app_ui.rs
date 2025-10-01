use crate::particle_system::{
    GeneratorType, GridParams, ParticleSystem, ParticleSystemManager, SphereParams,
};
use crate::state::LightManager;
use egui::{Align2, Context};

pub struct UiState {
    pub model_path_input: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            model_path_input: String::new(),
        }
    }
}

pub struct UiActions {
    pub save_requested: bool,
    pub load_requested: bool,
    pub model_to_load: Option<String>,
}

impl Default for UiActions {
    fn default() -> Self {
        Self {
            save_requested: false,
            load_requested: false,
            model_to_load: None,
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
    models: &std::collections::HashMap<String, std::sync::Arc<crate::model::Model>>,
    materials: &std::collections::HashMap<String, std::sync::Arc<crate::model::Material>>,
    ui_state: &mut UiState,
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
                                            ui.label("Model & Material:");

                                            egui::ComboBox::from_id_source(format!(
                                                "light_{}_model",
                                                i
                                            ))
                                            .selected_text(light_manager.model_path())
                                            .show_ui(
                                                ui,
                                                |ui| {
                                                    for model_path in models.keys() {
                                                        if ui
                                                            .selectable_label(
                                                                light_manager.model_path()
                                                                    == model_path,
                                                                model_path,
                                                            )
                                                            .clicked()
                                                        {
                                                            light_manager
                                                                .set_model_path(model_path.clone());
                                                        }
                                                    }
                                                },
                                            );

                                            egui::ComboBox::from_id_source(format!(
                                                "light_{}_material",
                                                i
                                            ))
                                            .selected_text(light_manager.material_key())
                                            .show_ui(
                                                ui,
                                                |ui| {
                                                    for material_key in materials.keys() {
                                                        if ui
                                                            .selectable_label(
                                                                light_manager.material_key()
                                                                    == material_key,
                                                                material_key,
                                                            )
                                                            .clicked()
                                                        {
                                                            light_manager.set_material_key(
                                                                material_key.clone(),
                                                            );
                                                        }
                                                    }
                                                },
                                            );

                                            ui.separator();
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
                            let params = GridParams {
                                rows: 10,
                                spacing: 1.0,
                                center: [0.0, 0.0, 0.0],
                            };
                            let system = ParticleSystem::new(
                                device,
                                name.clone(),
                                "teapot.obj".to_string(),
                                "teapot/default".to_string(),
                                GeneratorType::Grid(params),
                            );
                            particle_system_manager.add(name, system);
                        }

                        if ui.button("➕ Add Sphere").clicked() {
                            let name = format!("Sphere_{}", particle_system_manager.count());
                            let params = SphereParams {
                                count: 1000,
                                radius: 5.0,
                                center: [0.0, 0.0, 0.0],
                            };
                            let system = ParticleSystem::new(
                                device,
                                name.clone(),
                                "teapot.obj".to_string(),
                                "teapot/default".to_string(),
                                GeneratorType::Sphere(params),
                            );
                            particle_system_manager.add(name, system);
                        }
                    });

                    ui.separator();

                    // --- Particle Systems ---
                    ui.label("Particle Systems:");

                    let mut system_to_remove = None;
                    for (name, system) in particle_system_manager.systems_mut() {
                        ui.push_id(name, |ui| {
                            ui.horizontal(|ui| {
                                let header = egui::CollapsingHeader::new(name).default_open(false);

                                if header
                                    .show(ui, |ui| {
                                        ui.label(format!("Instances: {}", system.num_instances()));

                                        ui.separator();

                                        // Model and Material selection
                                        ui.label("Model & Material:");

                                        egui::ComboBox::from_id_source(format!("{}_model", name))
                                            .selected_text(system.model_path())
                                            .show_ui(ui, |ui| {
                                                for model_path in models.keys() {
                                                    if ui
                                                        .selectable_label(
                                                            system.model_path() == model_path,
                                                            model_path,
                                                        )
                                                        .clicked()
                                                    {
                                                        system.set_model_path(model_path.clone());
                                                    }
                                                }
                                            });

                                        egui::ComboBox::from_id_source(format!(
                                            "{}_material",
                                            name
                                        ))
                                        .selected_text(system.material_key())
                                        .show_ui(
                                            ui,
                                            |ui| {
                                                for material_key in materials.keys() {
                                                    if ui
                                                        .selectable_label(
                                                            system.material_key() == material_key,
                                                            material_key,
                                                        )
                                                        .clicked()
                                                    {
                                                        system
                                                            .set_material_key(material_key.clone());
                                                    }
                                                }
                                            },
                                        );

                                        ui.separator();
                                        ui.label("Generator:");

                                        let mut params_changed = false;

                                        match system.generator_mut() {
                                            crate::particle_system::GeneratorType::Grid(params) => {
                                                ui.label("Type: Grid");
                                                ui.separator();

                                                ui.horizontal(|ui| {
                                                    ui.label("Rows:");
                                                    if ui
                                                        .add(egui::Slider::new(
                                                            &mut params.rows,
                                                            5..=50,
                                                        ))
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
                                                }
                                            }
                                            crate::particle_system::GeneratorType::Sphere(
                                                params,
                                            ) => {
                                                ui.label("Type: Sphere");
                                                ui.separator();

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
                                                }
                                            }
                                        }

                                        if params_changed {
                                            system.mark_dirty();
                                        }
                                    })
                                    .body_returned
                                    .is_some()
                                {
                                    if ui.button("🗑").clicked() {
                                        system_to_remove = Some(name.clone());
                                    }
                                }
                            });
                        });
                    }

                    if let Some(name) = system_to_remove {
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

            // Materials Inspection
            ui.collapsing(format!("🎨 Materials ({})", materials.len()), |ui| {
                for (key, material) in materials.iter() {
                    ui.label(format!("• {} (texture: {})", material.name, key));
                }
            });

            ui.separator();

            // Geometries Inspection
            ui.collapsing(format!("🔷 Geometries ({})", models.len()), |ui| {
                for (_path, model) in models.iter() {
                    let total_vertices: u32 = model.meshes.iter().map(|m| m.vertex_count).sum();
                    ui.label(format!(
                        "• {} ({} mesh{}, {} vertices)",
                        model.name,
                        model.meshes.len(),
                        if model.meshes.len() == 1 { "" } else { "es" },
                        total_vertices
                    ));
                }
            });

            ui.separator();

            // Load Model
            ui.collapsing("📦 Load Model", |ui| {
                ui.label("Enter model path (e.g., 'teapot.obj'):");

                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut ui_state.model_path_input);

                    if ui.button("Load").clicked() && !ui_state.model_path_input.is_empty() {
                        actions.model_to_load = Some(ui_state.model_path_input.clone());
                        ui_state.model_path_input.clear();
                    }
                });

                ui.label("Common models in res/:");
                ui.label("• teapot.obj");
            });

            ui.separator();

            ui.label(format!("Delta Time: {:.2} ms", delta_time_ms));
            ui.label(format!("FPS: {:.1}", 1000.0 / delta_time_ms));
        });

    actions
}
