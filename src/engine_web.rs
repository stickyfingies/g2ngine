use crate::scripting::{ScriptEngine, log_from_js};
use wasm_bindgen::prelude::*;
use web_sys::*;

fn setup_global_functions() -> Result<(), JsValue> {
    let window = web_sys::window().unwrap();

    let say_closure = Closure::wrap(Box::new(move |message: String| {
        log_from_js(message);
    }) as Box<dyn Fn(String)>);

    js_sys::Reflect::set(&window, &"say".into(), say_closure.as_ref().unchecked_ref())
        .expect("Failed to set global function");
    say_closure.forget();

    let data_fn_closure = Closure::wrap(Box::new(move |float32_array: js_sys::Float32Array| {
        // Work directly with the Float32Array
        let mut floats = vec![0f32; float32_array.length() as usize];
        float32_array.copy_to(&mut floats);

        // Meaningful work!
        web_sys::console::log_1(&format!("Updated data from JS: {:?}", floats).into());
    }) as Box<dyn Fn(js_sys::Float32Array)>);

    js_sys::Reflect::set(
        &window,
        &"data_fn".into(),
        data_fn_closure.as_ref().unchecked_ref(),
    )
    .expect("Failed to set data_fn function");
    data_fn_closure.forget();

    Ok(())
}

pub struct ScriptEngineWeb;

impl ScriptEngine for ScriptEngineWeb {
    fn new() -> Self {
        ScriptEngineWeb {}
    }

    async fn load_javascript_file(&mut self, path: String) {
        use js_sys::Promise;
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen_futures::JsFuture;

        setup_global_functions().unwrap();

        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();

        let script = document
            .create_element("script")
            .unwrap()
            .dyn_into::<HtmlScriptElement>()
            .unwrap();

        // Build URL using window.location.origin + pathname
        let location = window.location();
        let origin = location.origin().unwrap();
        let pathname = location.pathname().unwrap();
        let script_url = format!("{}{}/res/{}", origin, pathname, path);

        script.set_src(&script_url);
        script.set_type("text/javascript");

        // Create a promise that resolves when the script loads
        let promise = Promise::new(&mut |resolve, reject| {
            let resolve = Rc::new(RefCell::new(Some(resolve)));
            let reject = Rc::new(RefCell::new(Some(reject)));

            let resolve_clone = resolve.clone();
            let onload_closure = Closure::wrap(Box::new(move || {
                if let Some(resolve) = resolve_clone.borrow_mut().take() {
                    resolve.call0(&JsValue::undefined()).unwrap();
                }
            }) as Box<dyn Fn()>);

            let reject_clone = reject.clone();
            let onerror_closure = Closure::wrap(Box::new(move |_: web_sys::Event| {
                if let Some(reject) = reject_clone.borrow_mut().take() {
                    reject
                        .call1(
                            &JsValue::undefined(),
                            &JsValue::from_str("Script failed to load"),
                        )
                        .unwrap();
                }
            }) as Box<dyn Fn(web_sys::Event)>);

            script.set_onload(Some(onload_closure.as_ref().unchecked_ref()));
            script.set_onerror(Some(onerror_closure.as_ref().unchecked_ref()));

            onload_closure.forget();
            onerror_closure.forget();
        });

        let head = document.head().unwrap();
        head.append_child(&script).unwrap();

        // Wait for the script to load
        JsFuture::from(promise).await.unwrap();
    }

    fn call_js<T: serde::Serialize, R: for<'de> serde::Deserialize<'de>>(
        &mut self,
        function_name: String,
        data: &T,
    ) -> Result<R, String> {
        let window = web_sys::window().ok_or("No window object available")?;

        let function = js_sys::Reflect::get(&window, &function_name.as_str().into())
            .map_err(|_| format!("Failed to get function '{}'", function_name))?;

        if !function.is_function() {
            return Err(format!("'{}' is not a function", function_name));
        }

        let json_data =
            serde_json::to_string(data).map_err(|e| format!("Failed to serialize data: {}", e))?;

        let js_data = js_sys::JSON::parse(&json_data)
            .map_err(|e| format!("Failed to parse JSON data: {:?}", e))?;

        let result = js_sys::Function::from(function)
            .call1(&window, &js_data)
            .map_err(|e| format!("Function call failed: {:?}", e))?;

        // Handle JavaScript undefined/null directly
        let json_value: serde_json::Value = if result.is_undefined() || result.is_null() {
            serde_json::Value::Null
        } else if let Some(string_result) = result.as_string() {
            // If it's already a string, convert to JSON Value
            serde_json::Value::String(string_result)
        } else {
            // Try to JSON stringify the result for arrays/objects/numbers
            match js_sys::JSON::stringify(&result) {
                Ok(json_string) => {
                    if let Some(json_str) = json_string.as_string() {
                        serde_json::from_str(&json_str).map_err(|e| {
                            format!("Failed to parse stringified result '{}': {}", json_str, e)
                        })?
                    } else {
                        return Err("Failed to stringify result".to_string());
                    }
                }
                Err(_) => return Err("Failed to stringify result".to_string()),
            }
        };

        // Then convert from Value to target type (this handles number->i32, string->String, etc.)
        serde_json::from_value(json_value)
            .map_err(|e| format!("Failed to convert result to target type: {}", e))
    }

    fn call_js_float32array<T: serde::Serialize>(
        &mut self,
        function_name: String,
        data: &T,
    ) -> Result<Vec<f32>, String> {
        let window = web_sys::window().ok_or("No window object available")?;

        let function = js_sys::Reflect::get(&window, &function_name.as_str().into())
            .map_err(|_| format!("Failed to get function '{}'", function_name))?;

        if !function.is_function() {
            return Err(format!("'{}' is not a function", function_name));
        }

        let json_data =
            serde_json::to_string(data).map_err(|e| format!("Failed to serialize data: {}", e))?;

        let js_data = js_sys::JSON::parse(&json_data)
            .map_err(|e| format!("Failed to parse JSON data: {:?}", e))?;

        let result = js_sys::Function::from(function)
            .call1(&window, &js_data)
            .map_err(|e| format!("Function call failed: {:?}", e))?;

        // Convert result to Float32Array
        let float32_array = js_sys::Float32Array::new(&result);

        // Extract data efficiently
        let mut floats = vec![0f32; float32_array.length() as usize];
        float32_array.copy_to(&mut floats);

        Ok(floats)
    }
}
