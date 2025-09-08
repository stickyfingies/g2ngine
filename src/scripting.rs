use serde::Serialize;

pub trait ScriptEngine {
    fn new() -> Self
    where
        Self: Sized;

    async fn load_javascript_file(&self, path: String);

    fn call_javascript_function<T: Serialize>(
        &self,
        function_name: String,
        data: &T,
    ) -> Result<String, String>;
}

// Exposed to JS
// TODO: bindings are hardcoded in engine_*.rs
// TODO: binding interface in ScriptEngine
pub fn log_from_js(message: String) {
    log::info!("[Rust Log]: {}", message);
}
