use deno_core::{OpState, ResourceId, op2};
use deno_core::{RuntimeOptions, extension, v8::CreateParams};
use deno_error::JsErrorBox;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn runtime_options() -> RuntimeOptions {
    RuntimeOptions {
        extensions: vec![
            deno_webidl::deno_webidl::init(),
            deno_web::deno_web::init(Default::default()),
            deno_fetch::deno_fetch::init(Default::default()),
            bootstrap::init(),
            request_response_extension::init(),
        ],
        create_params: Some(CreateParams::default()),
        ..Default::default()
    }
}

extension!(
    bootstrap,
    esm_entry_point = "ext:bootstrap/bootstrap.js",
    esm = ["bootstrap.js", "run.js"],
);

#[derive(Default)]
pub struct RequestParts {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub rid: Option<ResourceId>,
}

pub struct ResponseParts {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub rid: Option<ResourceId>,
}

type OpGetRequestParts = (String, String, Vec<(String, String)>, Option<ResourceId>);

#[op2]
#[serde]
fn op_get_request_parts(state: &mut OpState) -> Result<OpGetRequestParts, JsErrorBox> {
    let parts = state
        .try_take::<RequestParts>()
        .ok_or_else(|| JsErrorBox::generic("Request parts not found"))?;
    Ok((parts.url, parts.method, parts.headers, parts.rid))
}

#[op2(async)]
async fn op_respond(
    state: Rc<RefCell<OpState>>,
    #[smi] status: u16,
    #[serde] headers: Vec<(String, String)>,
    #[smi] rid: Option<ResourceId>,
) -> Result<(), JsErrorBox> {
    let headers_map = headers.into_iter().collect::<HashMap<String, String>>();
    let parts = ResponseParts {
        status,
        headers: headers_map,
        rid,
    };

    state.borrow_mut().put(parts);
    Ok(())
}

deno_core::extension!(
    request_response_extension,
    ops = [op_get_request_parts, op_respond],
    state = |s| {
        s.put(RequestParts::default());
    },
);
