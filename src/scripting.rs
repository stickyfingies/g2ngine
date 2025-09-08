pub trait ScriptEngine {
    fn new() -> Self
    where
        Self: Sized;

    fn load_javascript_file(&self, path: String);
}

pub fn log_from_js(message: String) {
    log::info!("[Rust Log]: {}", message);
}
