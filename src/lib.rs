mod app_ui;
mod camera;
mod egui;
#[cfg(not(target_arch = "wasm32"))]
mod engine_desktop;
#[cfg(target_arch = "wasm32")]
mod engine_web;
mod model;
mod particle_system;
mod resources;
mod scripting;
mod state;
mod texture;
pub mod world;

use crate::state::State;
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    window::Window,
};

pub struct App {
    #[cfg(target_arch = "wasm32")]
    proxy: Option<winit::event_loop::EventLoopProxy<State>>,
    state: Option<State>,
    last_render_time: web_time::Instant,
}

impl App {
    pub fn new(#[cfg(target_arch = "wasm32")] event_loop: &EventLoop<State>) -> Self {
        #[cfg(target_arch = "wasm32")]
        let proxy = Some(event_loop.create_proxy());

        Self {
            state: None,
            #[cfg(target_arch = "wasm32")]
            proxy,
            last_render_time: web_time::Instant::now(),
        }
    }
}

impl ApplicationHandler<State> for App {
    // [Browser]
    // Initializing the application state is an asynchronous operation,
    // which we cannot block/await in wasm due to runtime limitations.
    // Instead, we launch a background process where we can await it.
    // When it finishes, a message is sent to the event loop containing
    // the newly-created application state.
    #[cfg(target_arch = "wasm32")]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowAttributesExtWebSys;

        const CANVAS_ID: &str = "canvas";

        let window = wgpu::web_sys::window().unwrap_throw();
        let document = window.document().unwrap_throw();
        let canvas = document.get_element_by_id(CANVAS_ID).unwrap_throw();
        let html_canvas_element = canvas.unchecked_into();
        let window_attributes = Window::default_attributes().with_canvas(Some(html_canvas_element));
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        if let Some(proxy) = self.proxy.take() {
            wasm_bindgen_futures::spawn_local(async move {
                let state = State::new(window)
                    .await
                    .expect("Unable to create canvas!!!");
                assert!(proxy.send_event(state).is_ok());
            });
        }
    }

    // [Desktop]
    // This is pretty basic - just await (block_on) async initialization.
    #[cfg(not(target_arch = "wasm32"))]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = Window::default_attributes();
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
        let state = pollster::block_on(State::new(window)).unwrap();
        self.state = Some(state);
    }

    #[allow(unused_mut)]
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, mut event: State) {
        #[cfg(target_arch = "wasm32")]
        {
            event.window().request_redraw();
            event.resize(
                event.window().inner_size().width,
                event.window().inner_size().height,
            );
        }
        self.state = Some(event);
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let state = match &mut self.state {
            Some(canvas) => canvas,
            None => return,
        };

        match event {
            DeviceEvent::MouseMotion { delta } => state.mouse_movement(delta.0, delta.1),
            _ => {}
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let state = match &mut self.state {
            Some(canvas) => canvas,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                let now = web_time::Instant::now();
                let dt = now - self.last_render_time;
                self.last_render_time = now;
                state.update(dt);
                match state.render(dt) {
                    Ok(_) => {}
                    // Reconfigure the surface if it's lost, outdated, or suboptimal
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        let size = state.window().inner_size();
                        state.resize(size.width, size.height);
                    }
                    Err(e) => {
                        log::error!("Unable to render {}", e);
                    }
                }
            }
            _ => {
                state.input(&event_loop, &event);
            }
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    // Set up logging
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::init();
    #[cfg(target_arch = "wasm32")]
    console_log::init_with_level(log::Level::Info).unwrap_throw();

    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new(
        #[cfg(target_arch = "wasm32")]
        &event_loop,
    );
    event_loop.run_app(&mut app)?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn run_web() -> Result<(), wasm_bindgen::JsValue> {
    console_error_panic_hook::set_once();
    run().unwrap_throw();

    Ok(())
}
