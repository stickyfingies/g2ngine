use crate::scripting::{ScriptEngine, log_from_js};
use wasm_bindgen::prelude::*;
use web_sys::*;

fn setup_global_functions() -> Result<(), JsValue> {
    let window = web_sys::window().unwrap();

    let say_closure = Closure::wrap(Box::new(move |message: String| {
        log_from_js(message);
    }) as Box<dyn Fn(String)>);

    js_sys::Reflect::set(&window, &"say".into(), say_closure.as_ref().unchecked_ref());
    say_closure.forget();

    Ok(())
}

pub struct ScriptEngineWeb;

impl ScriptEngine for ScriptEngineWeb {
    fn new() -> Self {
        ScriptEngineWeb {}
    }

    async fn load_javascript_file(&self, path: String) {
        use wasm_bindgen_futures::JsFuture;
        use js_sys::Promise;
        use std::rc::Rc;
        use std::cell::RefCell;
        
        setup_global_functions().unwrap();

        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();

        let script = document
            .create_element("script")
            .unwrap()
            .dyn_into::<HtmlScriptElement>()
            .unwrap();

        script.set_src(&format!("/res/{}", path));
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
                    reject.call1(&JsValue::undefined(), &JsValue::from_str("Script failed to load")).unwrap();
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

    fn call_javascript_function<T: serde::Serialize>(
        &self,
        function_name: String,
        data: &T,
    ) -> Result<String, String> {
        let window = web_sys::window().ok_or("No window object available")?;
        
        let function = js_sys::Reflect::get(&window, &function_name.as_str().into())
            .map_err(|_| format!("Failed to get function '{}'", function_name))?;

        if !function.is_function() {
            return Err(format!("'{}' is not a function", function_name));
        }

        let json_data = serde_json::to_string(data)
            .map_err(|e| format!("Failed to serialize data: {}", e))?;

        let js_data = js_sys::JSON::parse(&json_data)
            .map_err(|e| format!("Failed to parse JSON data: {:?}", e))?;

        let result = js_sys::Function::from(function)
            .call1(&window, &js_data)
            .map_err(|e| format!("Function call failed: {:?}", e))?;

        // Try to get string first, then try JSON stringify, finally debug format
        if let Some(string_result) = result.as_string() {
            Ok(string_result)
        } else {
            // Try to JSON stringify the result for arrays/objects
            match js_sys::JSON::stringify(&result) {
                Ok(json_string) => {
                    if let Some(json_str) = json_string.as_string() {
                        Ok(json_str)
                    } else {
                        Ok(format!("{:?}", result))
                    }
                }
                Err(_) => Ok(format!("{:?}", result))
            }
        }
    }
}
