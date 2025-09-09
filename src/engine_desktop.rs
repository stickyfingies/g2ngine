use boa_engine::builtins::array_buffer::ArrayBuffer;
use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::resources::load_string;
use crate::scripting::{ScriptEngine, log_from_js};

/** JavaScript moves a Float32Array into Rust */
fn take_buffer(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> JsResult<JsValue> {
    // Argument(any)
    let js_typed_array = args.get(0).and_then(|val| val.as_object()).ok_or_else(|| {
        JsError::from(JsNativeError::typ().with_message("Argument must be a TypedArray"))
    })?;

    // Argument(any) -> Sub-property(ArrayBuffer)
    let js_buffer_obj = js_typed_array
        .get(JsString::from("buffer"), _context)?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            JsError::from(
                JsNativeError::typ().with_message("Could not get ArrayBuffer from object"),
            )
        })?;

    // ArrayBuffer -> Vec<u8>
    if let Some(mut array_buffer) = js_buffer_obj.downcast_mut::<ArrayBuffer>() {
        if let Some(byte_data) = array_buffer.detach(&JsValue::undefined())? {
            // Vec<u8> -> Vec<f32>
            let floats: Vec<f32> = byte_data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            // Meaningful work!
            println!("Updated data from JS: {:?}", floats);
        } else {
            return Err(JsError::from(
                JsNativeError::typ().with_message("Failed to detach ArrayBuffer"),
            ));
        }
    } else {
        return Err(JsError::from(
            JsNativeError::typ().with_message("Argument is not a valid ArrayBuffer"),
        ));
    }

    Ok(JsValue::undefined())
}

fn setup_global_functions(context: &mut Context, data: Rc<RefCell<Vec<f32>>>) {
    let log_fn = NativeFunction::from_fn_ptr(|_this, args, _context| {
        let msg = args.get(0).cloned().unwrap_or_default();
        let msg_string = msg.to_string(_context).unwrap().to_std_string_lossy();
        log_from_js(msg_string.to_string());
        Ok(JsValue::undefined())
    });
    context
        .register_global_callable("say".into(), 0, log_fn)
        .expect("Failed to register function");
    let data_fn = NativeFunction::from_fn_ptr(take_buffer);
    context
        .register_global_callable("data_fn".into(), 1, data_fn)
        .expect("Failed to register data_fn");
}

pub struct ScriptEngineDesktop {
    context: Context,
    data: Rc<RefCell<Vec<f32>>>,
}

impl ScriptEngine for ScriptEngineDesktop {
    fn new() -> Self {
        let context = Context::default();
        let data = Rc::new(RefCell::new(vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 20.0, 30.0, 1.0,
        ]));
        ScriptEngineDesktop { context, data }
    }

    async fn load_javascript_file(&mut self, path: String) {
        let js_code = load_string(&path)
            .await
            .expect("Failed to load javascript file");
        let js_source = Source::from_bytes(js_code.as_str());

        setup_global_functions(&mut self.context, self.data.clone());

        let result = self
            .context
            .eval(js_source)
            .expect("Failed to evaluate script (syntax error?)");

        log::info!("{}", result.display());
    }

    fn call_js<T: serde::Serialize, R: for<'de> serde::Deserialize<'de>>(
        &mut self,
        function_name: String,
        data: &T,
    ) -> Result<R, String> {
        let json_data =
            serde_json::to_string(data).map_err(|e| format!("Failed to serialize data: {}", e))?;

        let function_call = format!("{}({})", function_name, json_data);

        let source = Source::from_bytes(&function_call);
        let result = self
            .context
            .eval(source)
            .map_err(|e| format!("Function call failed: {}", e))?;

        let (json_value, json_string) = if result.is_undefined() || result.is_null() {
            (serde_json::Value::Null, "null".to_string())
        } else {
            let json_string = result
                .to_json(&mut self.context)
                .map_err(|e| format!("Failed to convert result to JSON: {}", e))?
                .to_string();

            let json_value = serde_json::from_str(&json_string)
                .map_err(|e| format!("Failed to parse result as JSON '{}': {}", json_string, e))?;

            (json_value, json_string)
        };

        // Then convert from Value to target type (this handles number->i32, string->String, etc.)
        serde_json::from_value(json_value).map_err(|e| {
            format!(
                "Failed to convert result '{}' to target type: {}",
                json_string, e
            )
        })
    }
}
