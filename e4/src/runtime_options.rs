use deno_core::{RuntimeOptions, extension, v8::CreateParams};

extension!(
    bootstrap,
    esm_entry_point = "ext:bootstrap/bootstrap.js",
    esm = ["bootstrap.js"],
);

pub fn runtime_options() -> RuntimeOptions {
    RuntimeOptions {
        extensions: vec![
            deno_webidl::deno_webidl::init(),
            deno_web::deno_web::init(Default::default()),
            deno_fetch::deno_fetch::init(Default::default()),
            bootstrap::init(),
        ],
        create_params: Some(CreateParams::default()),
        ..Default::default()
    }
}
