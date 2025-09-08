use boa_engine::{Context, JsValue, NativeFunction, Source};

use crate::resources::load_string;
use crate::scripting::{ScriptEngine, log_from_js};

fn setup_global_functions(context: &mut Context) {
    let log_fn = NativeFunction::from_fn_ptr(|_this, args, _context| {
        let msg = args.get(0).cloned().unwrap_or_default();
        let msg_string = msg.to_string(_context).unwrap().to_std_string_lossy();
        log_from_js(msg_string.to_string());
        Ok(JsValue::undefined())
    });
    context
        .register_global_callable("say".into(), 0, log_fn)
        .expect("Failed to register function");
}

pub struct ScriptEngineDesktop {
    context: Context,
}

impl ScriptEngine for ScriptEngineDesktop {
    fn new() -> Self {
        let context = Context::default();
        ScriptEngineDesktop { context }
    }

    async fn load_javascript_file(&mut self, path: String) {
        let js_code = load_string(&path).await.unwrap();
        let js_source = Source::from_bytes(js_code.as_str());

        setup_global_functions(&mut self.context);

        let result = self
            .context
            .eval(js_source)
            .expect("Failed to evaluate script");

        log::info!("{}", result.display());
    }

    fn call_javascript_function<T: serde::Serialize>(
        &mut self,
        function_name: String,
        data: &T,
    ) -> Result<String, String> {
        let json_data =
            serde_json::to_string(data).map_err(|e| format!("Failed to serialize data: {}", e))?;

        let function_call = format!("{}({})", function_name, json_data);

        let source = Source::from_bytes(&function_call);
        let result = self
            .context
            .eval(source)
            .map_err(|e| format!("Function call failed: {}", e))?;

        Ok(result.display().to_string())
    }
}
