mod http_body_resource;
mod runtime_options;

use bytes::Bytes;
use deno_core::anyhow::{Result, anyhow};
use deno_core::*;
use http::*;
use http_body_resource::*;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty};
use runtime_options::*;

type Body = UnsyncBoxBody<Bytes, anyhow::Error>;
type Request = hyper::Request<Body>;
type Response = hyper::Response<Body>;

static RUNTIME_SNAPSHOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/RUNJS_SNAPSHOT.bin"));

pub async fn run(code: &str, request: Request) -> Result<Response> {
    let code = code.to_string();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let mut runtime_options = runtime_options();
            runtime_options.startup_snapshot = Some(RUNTIME_SNAPSHOT);

            let mut runtime = JsRuntime::new(runtime_options);
            runtime.execute_script("[user code]", code.to_string())?;

            register_hyper_request(&mut runtime, request);

            eprintln!("[ski/lib.rs] Executing __ski_runHandler()...");
            let script_result = runtime.execute_script(
                "[run]",
                ascii_str!("globalThis.__ski_runHandler();"),
            )?;
            eprintln!("[ski/lib.rs] Script executed, resolving future...");
            let run_future = runtime.resolve(script_result);
            eprintln!("[ski/lib.rs] Awaiting run_future with event loop...");
            runtime.with_event_loop_future(run_future, Default::default()).await?;
            eprintln!("[ski/lib.rs] Handler completed");

            eprintln!("[ski/lib.rs] Getting op_state...");
            let op_state = runtime.op_state();

            eprintln!("[ski/lib.rs] Extracting ResponseParts...");
            let response_parts = op_state
                .borrow_mut()
                .try_take::<ResponseParts>()
                .ok_or_else(|| anyhow!("Did not get a response from JavaScript"))?;

            eprintln!("[ski/lib.rs] Got ResponseParts: status={}, rid={:?}", response_parts.status, response_parts.rid);

            let mut builder =
                hyper::Response::builder().status(StatusCode::from_u16(response_parts.status)?);

            for (key, value) in response_parts.headers {
                if let Ok(name) = HeaderName::from_bytes(key.as_bytes()) {
                    builder = builder.header(name, value);
                }
            }

            let Some(rid) = response_parts.rid else {
                eprintln!("[ski/lib.rs] No RID, returning empty body");
                let body =
                    BodyExt::boxed_unsync(Empty::<Bytes>::new().map_err(|never| match never {}));
                return Ok(builder.body(body)?);
            };

            eprintln!("[ski/lib.rs] Getting resource from table, RID: {}", rid);

            // Get the resource that was created by resourceForReadableStream() or is Deno-backed
            let resource = op_state
                .borrow_mut()
                .resource_table
                .get_any(rid)
                .map_err(|_| anyhow!("Resource not found"))?;

            eprintln!("[ski/lib.rs] Creating body from resource using ResourceToBodyAdapter...");

            // Use Deno's ResourceToBodyAdapter to convert Resource to Hyper Body
            let body_adapter = deno_fetch::ResourceToBodyAdapter::new(resource);
            let body = BodyExt::boxed_unsync(body_adapter.map_err(|e| anyhow::anyhow!(e)));

            eprintln!("[ski/lib.rs] Body created, building response...");
            Ok(builder.body(body)?)
        })
    })
    .await?
}

fn register_hyper_request(runtime: &mut JsRuntime, req: Request) {
    let op_state = runtime.op_state();
    let mut state = op_state.borrow_mut();

    let (parts, body) = req.into_parts();

    // Convert URI to full URL
    let url = if parts.uri.scheme().is_some() {
        parts.uri.to_string()
    } else {
        format!("http://localhost{}", parts.uri)
    };
    let method = parts.method.to_string();
    let headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let rid = if method == "GET" || method == "HEAD" {
        None
    } else {
        let resource = HttpBodyResource::new(body);
        Some(state.resource_table.add(resource))
    };
    state.put(RequestParts {
        url,
        method,
        headers,
        rid,
    });
}

#[tokio::test]
async fn test() {
    run(
        "new MessageChannel();",
        Request::new(UnsyncBoxBody::new(
            http_body_util::Empty::new().map_err(|never| match never {}),
        )),
    )
    .await
    .unwrap();
}
