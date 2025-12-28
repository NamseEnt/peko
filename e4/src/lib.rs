mod http_body_resource;
mod runtime_options;

use bytes::Bytes;
use deno_core::anyhow::{Result, anyhow};
use deno_core::*;
use futures::StreamExt;
use http::*;
use http_body_resource::*;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::Body;
use runtime_options::*;

static RUNTIME_SNAPSHOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/RUNJS_SNAPSHOT.bin"));
static RUN_JS: FastStaticString = ascii_str_include!("../run.js");

pub async fn run<B>(
    code: &str,
    request: hyper::Request<B>,
) -> Result<hyper::Response<BoxBody<Bytes, std::io::Error>>>
where
    B: Body<Data = Bytes, Error = std::io::Error> + Send + Sync + 'static,
{
    let mut runtime_options = runtime_options();
    runtime_options.startup_snapshot = Some(RUNTIME_SNAPSHOT);

    let mut runtime = JsRuntime::new(runtime_options);
    runtime.execute_script("[user code]", code.to_string())?;

    {
        let op_state = runtime.op_state();
        let mut state = op_state.borrow_mut();
        let (url, method, headers, rid) = register_hyper_request(&mut state, request);
        state.put(RequestParts {
            url,
            method,
            headers,
            rid,
        });
    }

    {
        let script_result = runtime.execute_script("[run]", RUN_JS)?;
        let run_future = runtime.resolve(script_result);
        runtime.run_event_loop(Default::default()).await?;
        run_future.await?;
    }

    let op_state = runtime.op_state();
    let mut state = op_state.borrow_mut();

    let response_parts = state
        .try_take::<ResponseParts>()
        .ok_or_else(|| anyhow!("Did not get a response from JavaScript"))?;

    let mut builder =
        hyper::Response::builder().status(StatusCode::from_u16(response_parts.status)?);

    for (key, value) in response_parts.headers {
        if let Ok(name) = HeaderName::from_bytes(key.as_bytes()) {
            builder = builder.header(name, value);
        }
    }

    let body = if let Some(rid) = response_parts.rid {
        let resource = state
            .resource_table
            .take::<HttpBodyResource>(rid)
            .map_err(|_| anyhow!("Resource not found"))?;

        let mut stream_opt = resource.stream.borrow_mut().await;
        let stream = stream_opt
            .take()
            .ok_or_else(|| anyhow!("Stream already used"))?;

        let http_stream = stream.map(|res| res.map(hyper::body::Frame::data));

        BodyExt::boxed(StreamBody::new(http_stream))
    } else {
        BodyExt::boxed(Empty::<Bytes>::new().map_err(|never| match never {}))
    };

    Ok(builder.body(body)?)
}

fn register_hyper_request<B>(
    state: &mut OpState,
    req: hyper::Request<B>,
) -> (String, String, Vec<(String, String)>, Option<ResourceId>)
where
    B: Body<Data = Bytes> + Send + Sync + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    let (parts, body) = req.into_parts();

    let url = parts.uri.to_string();
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

    (url, method, headers, rid)
}
