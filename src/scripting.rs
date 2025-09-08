pub trait ScriptEngine {
    fn new() -> Self
    where
        Self: Sized;

    async fn load_javascript_file(&self, path: String);
    fn call_javascript_function(
        &self,
        function_name: String,
        args: Vec<String>,
    ) -> Result<String, String>;
}

pub fn log_from_js(message: String) {
    log::info!("[Rust Log]: {}", message);
}
