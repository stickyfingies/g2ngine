#[cfg(not(target_arch = "wasm32"))]
mod engine_desktop;
#[cfg(target_arch = "wasm32")]
mod engine_web;
mod resources;
mod scripting;
mod state;
mod texture;

use crate::{scripting::ScriptEngine, state::State};
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::PhysicalKey,
    window::Window,
};

// Decide a script engine type
#[cfg(target_arch = "wasm32")]
type PlatformScriptEngine = engine_web::ScriptEngineWeb;
#[cfg(not(target_arch = "wasm32"))]
type PlatformScriptEngine = engine_desktop::ScriptEngineDesktop;

pub struct App {
    #[cfg(target_arch = "wasm32")]
    proxy: Option<winit::event_loop::EventLoopProxy<State>>,
    state: Option<State>,
    script_engine: PlatformScriptEngine,
}

impl App {
    pub fn new(#[cfg(target_arch = "wasm32")] event_loop: &EventLoop<State>) -> Self {
        #[cfg(target_arch = "wasm32")]
        let proxy = Some(event_loop.create_proxy());

        // Platform-specific script engine types
        #[cfg(target_arch = "wasm32")]
        let script_engine = engine_web::ScriptEngineWeb::new();
        #[cfg(not(target_arch = "wasm32"))]
        let script_engine = engine_desktop::ScriptEngineDesktop::new();

        Self {
            state: None,
            #[cfg(target_arch = "wasm32")]
            proxy,
            script_engine,
        }
    }
}

fn call_demo_functions<T: ScriptEngine>(script_engine: &T) {
    // Demonstrate calling JavaScript functions from Rust
    match script_engine.call_javascript_function("getInfo".into(), vec![]) {
        Ok(result) => log::info!("JS getInfo() returned: {}", result),
        Err(e) => log::error!("Failed to call getInfo: {}", e),
    }

    match script_engine.call_javascript_function("greet".into(), vec!["Rust".into()]) {
        Ok(result) => log::info!("JS greet('Rust') returned: {}", result),
        Err(e) => log::error!("Failed to call greet: {}", e),
    }

    match script_engine.call_javascript_function("add".into(), vec!["5".into(), "3".into()]) {
        Ok(result) => log::info!("JS add('5', '3') returned: {}", result),
        Err(e) => log::error!("Failed to call add: {}", e),
    }
}

impl ApplicationHandler<State> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[allow(unused_mut)]
        let mut window_attributes = Window::default_attributes();

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;

            const CANVAS_ID: &str = "canvas";

            let window = wgpu::web_sys::window().unwrap_throw();
            let document = window.document().unwrap_throw();
            let canvas = document.get_element_by_id(CANVAS_ID).unwrap_throw();
            let html_canvas_element = canvas.unchecked_into();
            window_attributes = window_attributes.with_canvas(Some(html_canvas_element));
        }

        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        // Desktop:
        // Load script, create renderstate, call JS functions
        #[cfg(not(target_arch = "wasm32"))]
        {
            pollster::block_on(self.script_engine.load_javascript_file("demo.js".into()));
            self.state = Some(pollster::block_on(State::new(window)).unwrap());
            call_demo_functions(&self.script_engine);
        }

        // Browser:
        // Load script | create renderstate, see `user_event` below
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async {
                let script_engine = engine_web::ScriptEngineWeb::new();
                script_engine.load_javascript_file("demo.js".into()).await;
            });

            if let Some(proxy) = self.proxy.take() {
                wasm_bindgen_futures::spawn_local(async move {
                    assert!(
                        proxy
                            .send_event(
                                State::new(window)
                                    .await
                                    .expect("Unable to create canvas!!!")
                            )
                            .is_ok()
                    );
                });
            }
        }
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

        // call JS functions once renderstate is ready
        #[cfg(target_arch = "wasm32")]
        call_demo_functions(&self.script_engine);
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
                state.update();
                match state.render() {
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
            WindowEvent::MouseInput { state, button, .. } => match (button, state.is_pressed()) {
                (MouseButton::Left, true) => {}
                (MouseButton::Left, false) => {}
                _ => {}
            },
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: key_state,
                        ..
                    },
                ..
            } => state.handle_key(event_loop, code, key_state.is_pressed()),
            _ => {}
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
