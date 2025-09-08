use boa_engine::{Context, JsValue, NativeFunction, Source};
use std::cell::RefCell;

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
    context: RefCell<Option<Context>>,
}

impl ScriptEngine for ScriptEngineDesktop {
    fn new() -> Self {
        ScriptEngineDesktop {
            context: RefCell::new(None),
        }
    }

    async fn load_javascript_file(&self, path: String) {
        let js_code = load_string(&path).await.unwrap();
        let js_source = Source::from_bytes(js_code.as_str());

        let mut context = Context::default();
        setup_global_functions(&mut context);

        let result = context.eval(js_source).expect("Failed to evaluate script");

        log::info!("{}", result.display());

        *self.context.borrow_mut() = Some(context);
    }

    fn call_javascript_function(
        &self,
        function_name: String,
        args: Vec<String>,
    ) -> Result<String, String> {
        let mut context_opt = self.context.borrow_mut();
        let context = context_opt
            .as_mut()
            .ok_or("No JavaScript context available. Load a script first.")?;

        let args_str = args
            .iter()
            .map(|arg| format!("\"{}\"", arg.replace("\"", "\\\"")))
            .collect::<Vec<_>>()
            .join(", ");

        let function_call = format!("{}({})", function_name, args_str);

        let source = Source::from_bytes(&function_call);
        let result = context
            .eval(source)
            .map_err(|e| format!("Function call failed: {}", e))?;

        Ok(result.display().to_string())
    }
}
