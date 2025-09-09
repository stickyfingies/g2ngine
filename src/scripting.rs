use serde::{Deserialize, Serialize};

pub trait ScriptEngine {
    fn new() -> Self
    where
        Self: Sized;

    async fn load_javascript_file(&mut self, path: String);

    fn call_js<T: Serialize, R: for<'de> Deserialize<'de>>(
        &mut self,
        function_name: String,
        data: &T,
    ) -> Result<R, String>;

    fn call_js_float32array<T: Serialize>(
        &mut self,
        function_name: String,
        data: &T,
    ) -> Result<Vec<f32>, String>;
}

// Exposed to JS
// TODO: bindings are hardcoded in engine_*.rs
// TODO: binding interface in ScriptEngine
pub fn log_from_js(message: String) {
    log::info!("[Rust Log]: {}", message);
}
