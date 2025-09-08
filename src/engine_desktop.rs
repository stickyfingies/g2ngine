use boa_engine::{Context, JsResult, JsValue, NativeFunction, Source};

use crate::resources::load_string;
use crate::scripting::log_from_js;

pub async fn do_js_stuff() {
    let js_code = load_string("demo.js").await.unwrap();
    let js_source = Source::from_bytes(js_code.as_str());

    let mut context = Context::default();
    // JS -> Rust: Register the `log_from_js` function.
    let log_fn = NativeFunction::from_fn_ptr(|_this, args, _context| {
        let msg = args.get(0).cloned().unwrap_or_default();
        let msg_string = msg.to_string(_context).unwrap().to_std_string_lossy();
        log_from_js(msg_string.to_string());
        Ok(JsValue::undefined())
    });
    context
        .register_global_callable("say".into(), 0, log_fn)
        .expect("Failed to register function");

    let result = context.eval(js_source).expect("Failed to evaluate script");

    log::info!("{}", result.display());
}
