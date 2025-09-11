use egui::{Align2, Context};

#[derive(Default)]
pub struct UIState {
    pub checkbox_value: bool,
    pub slider_value: f32,
}

pub fn demo_ui(ctx: &Context, ui_state: &mut UIState, delta_time_ms: f32) {
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

            ui.label(format!("Delta Time: {:.2} ms", delta_time_ms));
            ui.label(format!("FPS: {:.1}", 1000.0 / delta_time_ms));

            ui.separator();

            ui.label("Your 3D scene continues to render in the background!");
            ui.label("Camera controls should still work when not over the GUI.");
        });
}
