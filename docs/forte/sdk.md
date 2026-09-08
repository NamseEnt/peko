# forte-sdk Reference

`forte-sdk` is the runtime library that backend WASM components use. It provides HTTP types, request/response handling, cookie utilities, an outbound HTTP client, and re-exports commonly used crates.

## `ForteRequest<'a, Body>`

The request context passed to page and action handlers.

```rust
pub struct ForteRequest<'a, Body = forte_sdk::http::Body> {
    pub uri_authority: &'a str,       // host:port, e.g. "example.com"
    pub method: &'a http::Method,
    pub headers: &'a http::HeaderMap,
    pub jar: &'a mut CookieJar,
    pub raw_body: &'a [u8],            // populated only by buffered generated routes
    pub body: Body,                    // streaming body for pages/APIs, Input for actions
}
```

## HTTP Types

Re-exported from the `http` crate:

```rust
pub use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode};
pub use http::uri::{Authority, PathAndQuery, Scheme, Uri};
pub use http::request::Builder as RequestBuilder;
```

Also available via `forte_sdk::http_header::*` (all `http::header::*` constants).

## `Body`

An enum representing an HTTP body:

```rust
pub enum Body {
    Empty,
    Bytes(Vec<u8>),
    Stream(StreamReader<u8>),
}

impl Body {
    pub fn empty() -> Self;
    pub fn channel() -> (StreamWriter<u8>, Body);
    pub async fn read_chunk(&mut self) -> Result<Option<Bytes>, BodyError>;
    pub async fn bytes(self) -> Bytes;
    pub async fn bytes_limited(self, limit: usize) -> Result<Bytes, BodyError>;
    pub async fn text(self) -> Result<String>;
    pub async fn text_limited(self, limit: usize) -> Result<String>;
    pub async fn json<T: DeserializeOwned>(self) -> Result<T>;
    pub async fn json_limited<T: DeserializeOwned>(self, limit: usize) -> Result<T>;
    pub async fn form(self) -> Result<Vec<(String, String)>>;
    pub async fn form_limited(self, limit: usize) -> Result<Vec<(String, String)>>;
}
```

The runtime also uses an incoming-stream form for request and client-response
bodies; its transport fields are managed by the runtime. `BodyError` reports
buffer-limit overflow, WASI transport errors, cancellation, and invalid UTF-8.

`read_chunk` is single-consumer and backpressured. Each request chunk is at most 64 KiB;
dropped or unread bodies cancel delivery. `bytes_limited`, `text_limited`, `json_limited`, and
`form_limited` stop with an error when their explicit buffer limit is exceeded. The default
buffered limit used by `json`, `text`, and `form` is 1 MiB. `bytes` is retained for compatibility
and can collect an unbounded body, so use `bytes_limited` for request data.

Generated page and API handlers receive the streaming body in `req.body`. `raw_body` remains a
legacy slice and is empty for those streaming handlers. Generated actions and hooks buffer their
input through the 1 MiB convenience limit before deserializing it; this is independent of the
100 MB transport limit.

Converts from `Vec<u8>`, `&[u8]`, `String`, `&str`, `Bytes`, and `()`.

`Body::channel()` returns a writer/body pair for producing a streaming body incrementally. Bytes written to the `StreamWriter<u8>` are consumed by the paired body. Spawn a task to write bytes while the body is already in the response:

```rust
use forte_sdk::http::{Body, Response};
use forte_sdk::runtime::spawn;

let (mut writer, body) = Body::channel();
spawn(async move {
    for chunk in chunks {
        let _ = writer.write_all(chunk).await;
    }
    // writer drop closes the stream
});
let response = Response::builder()
    .status(200)
    .header("content-type", "text/plain")
    .body(body)?;
```

Use `Body::channel()` when you need to stream a response body without buffering the entire payload — for example, proxying an upstream streaming response or generating a long response incrementally. A response body from `http::Client::send` is already streaming and can be forwarded directly without `channel()`.

## Outbound HTTP Client

Make outbound requests using `http::Client`:

```rust
use forte_sdk::http::{Client, Request};

let client = Client::new();

let req = Request::builder()
    .method("POST")
    .uri("https://api.example.com/data")
    .header("content-type", "application/json")
    .body(Body::from(r#"{"key":"value"}"#))?;

let resp = client.send(req).await?;
let status = resp.status();
let body = resp.into_body().bytes().await;

// Or deserialize the JSON response body directly:
let data: MyType = resp.into_body().json::<MyType>().await?;
```

Outbound requests are subject to the fn0 Cloud subrequest limit (50 per request). Streaming request bodies use `Body::channel()` — see the [Body section](#body) above.

## WebSockets

`forte_sdk::websocket` exposes the connection event types and server-side command API:

```rust
pub async fn send(
    connection_id: &ConnectionId,
    message: WebSocketMessage,
) -> Result<(), WebSocketSendError>;

pub async fn disconnect(
    connection_id: &ConnectionId,
) -> Result<(), WebSocketDisconnectError>;
```

`WebSocketMessage` is `Text(Body)` or `Binary(Body)`, so both buffered and streaming bodies are
accepted. `IncomingMessage` is `Text(String)` or `Binary(Vec<u8>)`.

`WebSocketSendError` variants:

| Variant | `delivery_state()` | Meaning |
| --- | --- | --- |
| `ConnectionNotFound` | `NotSent` | Connection ID does not exist or already closed |
| `Backpressure` | `NotSent` | Per-connection send queue is full |
| `DeadlineExceeded { delivery }` | inherited | Timeout before flush completed |
| `Transport { delivery }` | inherited | Network failure during send |
| `InvalidText { delivery }` | inherited | UTF-8 violation in a `Text` message |
| `Internal { delivery }` | inherited | Internal fn0 error |

`delivery_state()` returns `NotSent` when the message was definitely not delivered, `Unknown` when
delivery is uncertain. Forte does not retry automatically.

See [WebSockets](websockets.md) for route callbacks, acceptance headers, limits, reconnect
recovery, and `DisconnectCause` variants.

## Static Page Cache

`forte_sdk::static_page_cache` invalidates individual `#[cache_static]` pages at the edge:

```rust
use forte_sdk::static_page_cache;

static_page_cache::purge(&["/episode/1", "/episodes"]).await?;
```

Paths are route paths as visitors request them (leading `/`, no query string or fragment). Returns once the invalidation is queued, not once the edge is consistent. Errors: `UnusablePath`, `RateLimited` (hourly budget shared with `object_storage::public` purges), `Transport`, `UnexpectedStatus`.

See [pages.md](pages.md#invalidating-a-page-without-deploying) for constraints, percent-encoding rules, and the CLI equivalent (`forte purge-page`).

## Cookie Signing

Signed, HMAC-SHA256 cookies. Requires `COOKIE_SECRET` env var.

```rust
use forte_sdk::cookie_sign::{sign_cookie, unsign_cookie};
use forte_sdk::time;

// Write a signed cookie
sign_cookie(
    req.jar,
    "session",
    &my_value,               // any T: Serialize
    Some(time::Duration::days(30)),
);

// Read and verify a signed cookie
let value: Option<MyType> = unsign_cookie(req.jar, "session");
```

Cookies are set `HttpOnly`, `Secure`, `Path=/`. The value is serialized with `serde_json` and then HMAC-signed; the signature is appended as a hex suffix separated by `.`.

## `serve` function

Used internally by the generated `route_generated.rs`. Bridges between the WASI HTTP types and the `http::Request` / `http::Response` types:

```rust
pub async fn serve<F, Fut, E>(
    req: wasi::http::types::Request,
    dispatch: F,
) -> Result<wasi::http::types::Response, ErrorCode>
where
    F: FnOnce(http::Request<forte_sdk::http::Body>) -> Fut,
    Fut: Future<Output = Result<http::Response<Body>, E>>,
    E: fmt::Debug,
```

Also initializes OpenTelemetry and creates a tracing span for each request.

## Time

```rust
use forte_sdk::{DateTime, now};

pub type DateTime = chrono::DateTime<chrono::Utc>;

let t: DateTime = now(); // current UTC time
```

Also re-exports the `time` crate for use with cookie max-age.

## Async Sleep and Monotonic Time (`time_wasi`)

`forte_sdk::time_wasi` provides WASI-backed async sleep and monotonic time measurement. Use these instead of `std::thread::sleep` (which blocks) or `tokio::time::sleep` (which is not available in WASM components).

```rust
use forte_sdk::time_wasi;

// Async sleep
time_wasi::sleep(time_wasi::Duration::from_secs(1)).await;

// Measure elapsed time
let start = time_wasi::Instant::now();
// ... do work ...
let elapsed: time_wasi::Duration = start.elapsed();
```

API:
- `time_wasi::sleep(duration: Duration)` — suspend the current task for `duration`
- `time_wasi::Instant::now() -> Instant` — current monotonic time
- `time_wasi::Instant::duration_since(&self, earlier: Instant) -> Duration` — time between two instants
- `time_wasi::Instant::elapsed(&self) -> Duration` — time since this instant was recorded
- `time_wasi::Duration` — re-exported `std::time::Duration`

## UUID

forte-sdk enables the uuid `v7` feature. Use `now_v7()` to generate a time-ordered UUID:

```rust
use forte_sdk::Uuid;
let id = Uuid::now_v7();
```

## Randomness

Backed by WASI random.

```rust
use forte_sdk::rand;

// Fill a buffer with cryptographically secure random bytes
rand::fill_bytes(&mut buf);

// Get a Vec<u8> of secure random bytes
let bytes: Vec<u8> = rand::get_random_bytes(32);

// Get a single u64
let n: u64 = rand::get_random_u64();

// Insecure (fast) variants — not suitable for secrets
rand::get_insecure_random_bytes(&mut buf);
let n: u64 = rand::get_insecure_random_u64();
```

## Tracing / Logging

```rust
use forte_sdk::tracing;

tracing::info!("processing request");
tracing::error!("something failed: {}", err);
```

OpenTelemetry is initialized once per instance on the first request via `otel::init_once()`, which is called automatically by the `serve` function. Spans are exported to `http://fn0-otel.fn0.dev/v1/traces` (the fn0 Cloud collector) using the OTLP protobuf format.

The service name defaults to `"forte-app"` and can be overridden with the `OTEL_SERVICE_NAME` environment variable.

## Metrics

`forte_sdk::metrics` provides an OTLP metrics pipeline. Instruments created from the shared `Meter` are aggregated with **delta temporality** and flushed at the end of each request — the same path as traces. No background exporter is needed because a forte component only holds CPU during a request.

```rust
use forte_sdk::metrics::{meter, Counter, KeyValue};

// Create a counter once (e.g. at module level with LazyLock)
let requests: Counter<u64> = meter().u64_counter("requests").build();

// Record a measurement inside a handler
requests.add(1, &[KeyValue::new("route", "/api/users")]);
```

Instrument types re-exported from `forte_sdk::metrics`:

| Type | Description |
|---|---|
| `Counter<T>` | Monotonically increasing sum |
| `UpDownCounter<T>` | Sum that can increase and decrease |
| `Gauge<T>` | Last-value point-in-time measurement |
| `Histogram<T>` | Bucketed distribution |
| `KeyValue` | Attribute key/value pair for labels |

Get the shared `Meter` with `forte_sdk::metrics::meter()`. Instruments survive across the process lifetime (module-level `LazyLock` is fine).

Metrics are exported to `http://fn0-otel.fn0.dev/v1/metrics` (the fn0 Cloud collector). The service name defaults to `"forte-app"` and can be overridden with `OTEL_SERVICE_NAME`.

If no instrument has been created during a request the flush is a no-op; there is no overhead for apps that don't use metrics.

## `forte_json` — Serialization Format

`forte-json` is a custom JSON serializer/deserializer used for all handler I/O. It differs from `serde_json` in two ways:

**Serialization (Rust → JSON):**
- Struct field names are converted to camelCase (`user_name` → `"userName"`)
- **`Option::None` struct fields are omitted entirely** — they do not appear in the JSON output at all (not serialized as `null`). This differs from `serde_json` default behavior.
- Enum variants use a `t` discriminant field:

| Variant kind | Rust | JSON |
|---|---|---|
| Unit | `Ok` | `{"t":"Ok"}` |
| Tuple/newtype (1 field) | `Ok(String)` | `{"t":"Ok","v":"..."}` |
| Struct | `Ok { message: String }` | `{"t":"Ok","message":"..."}` |

For struct variants the fields are spread flat alongside `t`; there is no `v` wrapper.

`Option::None` omission applies to struct and struct-variant fields only — a top-level `None` serializes as `null`. Generated TypeScript types use `fieldName?: T` (optional property) rather than `fieldName: T | null`.

**Deserialization (JSON → Rust):**
- All object keys are converted to snake_case before deserializing (`"userName"` → `"user_name"`)
- This means TypeScript action input shapes use camelCase while Rust struct fields use snake_case

```rust
// Rust
#[derive(Deserialize)]
pub struct Input {
    pub user_name: String,   // receives "userName" from the browser
}
```

The API:
```rust
use forte_sdk::forte_json;
use futures::StreamExt;

// Serialize
let bytes: Vec<u8> = forte_json::to_vec(&my_value);
let stream = forte_json::to_stream(&my_value); // Stream<Item = Bytes>; yields chunks

// Deserialize
let value: MyType = forte_json::from_slice(&bytes)?;
let value: MyType = forte_json::from_str(json_str)?;
```

`to_stream` returns a lazy `Stream<Item = Bytes>` that emits up to 8 KiB chunks. Use it when building a streaming HTTP response body rather than buffering the whole payload in memory first (`to_vec` always buffers).

## Re-exported Crates

All re-exported at the crate root and usable via `forte_sdk::`:

| Symbol | Source crate |
|---|---|
| `anyhow` | `anyhow` |
| `chrono` | `chrono` |
| `cookie`, `Cookie`, `CookieBuilder`, `CookieJar` | `cookie` |
| `form_urlencoded` | `form_urlencoded` |
| `forte_json` | `forte-json` |
| `forte_macros::{cache_static, forte_doc, test}` | `forte-macros` |
| `futures` | `futures` |
| `hex` | `hex` |
| `serde` | `serde` |
| `serde_json` | `serde_json` |
| `sha2` | `sha2` |
| `time` | `time` |
| `tracing` | `tracing` |
| `Uuid` | `uuid` |
| `wit_bindgen` | `wit-bindgen` |
| `metrics::{meter, Counter, Gauge, Histogram, UpDownCounter, KeyValue}` | `forte-sdk` (OTLP metrics) |

## Runtime Utilities

All re-exported from `wit_bindgen`:

```rust
use forte_sdk::runtime::{spawn, block_on, yield_async, yield_blocking, backpressure_inc, backpressure_dec};
```

- `spawn(future)` — spawns an async task on the current WASI task pool; used internally by `serve` to write response bodies.
- `block_on(future)` — runs an async future to completion; used by the `#[forte_sdk::test]` macro.
- `yield_async()` — suspends the current task, allowing other pending tasks to make progress (cooperative multitasking).
- `yield_blocking()` — similar to `yield_async` but intended for wrapping operations that block the thread.
- `backpressure_inc()` / `backpressure_dec()` — signal to the WASI host that the component is under backpressure (increments/decrements the backpressure counter).

`block_on` is used by the `#[forte_sdk::test]` macro. Most application code only needs `spawn` to fire-and-forget a future (e.g., send a metric without awaiting the response).

## Macros

### `#[forte_sdk::test]`

Wraps an `async fn` as a synchronous test using `runtime::block_on`:

```rust
#[forte_sdk::test]
async fn my_test() {
    // async test code
}
```

Must be `async fn` with no arguments.

### `#[forte_doc]`

Derives database CRUD operations for a struct. See [doc-db/overview.md](../doc-db/overview.md) for full documentation.
