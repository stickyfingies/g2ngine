use crate::scripting::{ScriptResult, ScriptingHost, log_from_js};
use wasm_bindgen::prelude::*;
use web_sys::*;

// JS -> Rust: Use `#[wasm_bindgen]` to expose the function.
#[wasm_bindgen]
pub fn say(message: String) {
    log_from_js(message);
}

fn setup_global_functions() -> Result<(), JsValue> {
    let window = web_sys::window().unwrap();

    let say_closure = Closure::wrap(Box::new(move |message: String| {
        log_from_js(message);
    }) as Box<dyn Fn(String)>);

    js_sys::Reflect::set(&window, &"say".into(), say_closure.as_ref().unchecked_ref());
    say_closure.forget();

    Ok(())
}

pub async fn do_js_stuff() {
    setup_global_functions().unwrap();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();

    let script = document
        .create_element("script")
        .unwrap()
        .dyn_into::<HtmlScriptElement>()
        .unwrap();

    script.set_src("/res/demo.js");
    script.set_type("text/javascript");

    let head = document.head().unwrap();
    head.append_child(&script).unwrap();
}
