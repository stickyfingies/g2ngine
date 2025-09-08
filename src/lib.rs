#[cfg(not(target_arch = "wasm32"))]
mod engine_desktop;
#[cfg(target_arch = "wasm32")]
mod engine_web;
mod resources;
mod scripting;
mod state;
mod texture;

use crate::{scripting::ScriptEngine, state::State};
use serde::Serialize;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Serialize)]
struct GameData {
    player_name: String,
    score: u32,
    level: u8,
    position: [f32; 2],
}
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
    script_engine: Rc<RefCell<PlatformScriptEngine>>,
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
            // This needs to be reference-counted because web initialization
            // requires access to the script engine from a background process
            // which may not borrow from App, so we clone the reference instead.
            script_engine: Rc::new(RefCell::new(script_engine)),
        }
    }
}

fn call_demo_functions<T: ScriptEngine>(script_engine: &mut T) {
    // Demonstrate calling JavaScript functions from Rust with simple data
    match script_engine.call_javascript_function("getInfo".into(), &()) {
        Ok(result) => log::info!("JS getInfo() returned: {}", result),
        Err(e) => log::error!("Failed to call getInfo: {}", e),
    }

    match script_engine.call_javascript_function("greet".into(), &"Rust".to_string()) {
        Ok(result) => log::info!("JS greet('Rust') returned: {}", result),
        Err(e) => log::error!("Failed to call greet: {}", e),
    }

    match script_engine.call_javascript_function("add".into(), &[5, 3]) {
        Ok(result) => log::info!("JS add([5, 3]) returned: {}", result),
        Err(e) => log::error!("Failed to call add: {}", e),
    }

    // NEW: Demonstrate passing a Rust struct to JavaScript
    let game_data = GameData {
        player_name: "Alice".to_string(),
        score: 1250,
        level: 5,
        position: [100.5, 200.0],
    };

    match script_engine.call_javascript_function("processGameData".into(), &game_data) {
        Ok(result) => log::info!("JS processGameData(struct) returned: {}", result),
        Err(e) => log::error!("Failed to call processGameData: {}", e),
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

        // [Desktop]
        // This is pretty basic - just await (block_on) async initialization.
        #[cfg(not(target_arch = "wasm32"))]
        {
            pollster::block_on(
                self.script_engine
                    .borrow_mut()
                    .load_javascript_file("demo.js".into()),
            );
            self.state = Some(pollster::block_on(State::new(window)).unwrap());
            call_demo_functions(&mut *self.script_engine.borrow_mut());
        }

        // [Browser]
        //     *inhale* Initializing the scripting engine and rendering state
        // are both asynchronous operations, which we cannot block/await in
        // wasm due to runtime limitations.  Instead, we clone the reference
        // to the scripting engine[0], and launch a background process[1] where
        // we can await them.  When they finish, a message is sent[2] to the
        // event loop containing the newly-created renderstate.
        #[cfg(target_arch = "wasm32")]
        {
            use futures::future::join;

            if let Some(proxy) = self.proxy.take() {
                // 0 - see note
                let script_engine = self.script_engine.clone();
                // 1 - see note
                wasm_bindgen_futures::spawn_local(async move {
                    let (state, _) = join(
                        State::new(window),
                        script_engine
                            .borrow_mut()
                            .load_javascript_file("demo.js".into()),
                    )
                    .await;
                    // 2 - see note
                    assert!(
                        proxy
                            .send_event(state.expect("Unable to create canvas!!!"))
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
        call_demo_functions(&mut *self.script_engine.borrow_mut());
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
                {
                    // Call JS update function every frame and capture clear color
                    match self
                        .script_engine
                        .borrow_mut()
                        .call_javascript_function("update".into(), &())
                    {
                        Ok(result) => {
                            // Try to parse the result as a JSON array [r, g, b, a]
                            match serde_json::from_str::<[f32; 4]>(&result) {
                                Ok(color) => {
                                    state.set_clear_color(color);
                                }
                                Err(e) => {
                                    log::warn!(
                                        "JS update() returned invalid color format: {} (error: {})",
                                        result,
                                        e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("JS update() failed: {}", e);
                        }
                    }
                }

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
