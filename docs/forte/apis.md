# Forte API Endpoints

API endpoints live under `rs/src/apis/` and return JSON directly to the client. Unlike page handlers, they do not trigger the SSR step — there is no React rendering or `x-fn0-next: js` header.

## File Location and Route Mapping

Files in `rs/src/apis/` map to routes prefixed with `/api/`. Discovery is recursive, so subdirectories work:

| File path | Route |
|---|---|
| `rs/src/apis/users.rs` | `/api/users` |
| `rs/src/apis/products.rs` | `/api/products` |
| `rs/src/apis/orders/[id]/mod.rs` | `/api/orders/:id` |

The generated router does not constrain HTTP methods for API routes — the handler receives the request for any verb. Check `req.method` inside the handler if you need to distinguish `GET` vs `POST`, etc.

There is no `forte add api` command. Create files manually.

## Handler Signature

An API handler looks identical to a page handler, but the return type **must be named `Props`** (or contain the string `"Props"`) for codegen to discover it. (The exception is raw-response handlers — see [Raw Responses](#raw-responses-forteresponse) below.)

```rust
// rs/src/apis/users.rs
use anyhow::Result;
use forte_sdk::ForteRequest;
use serde::Serialize;

#[derive(Serialize)]
pub enum Props {
    Ok { users: Vec<User> },
    Empty,
}

#[derive(Serialize)]
pub struct User {
    pub id: String,
    pub name: String,
}

pub async fn handler(_req: ForteRequest<'_>) -> Result<Props> {
    // ... fetch users ...
    Ok(Props::Ok { users: vec![] })
}
```

The response is serialized with `forte_json::to_vec`, so:
- Struct field names become camelCase (`user_id` → `"userId"`)
- Enum variants use the `t` discriminant (same as page Props)

The response is sent with `Content-Type: application/json` and HTTP 200.

## Path and Search Parameters

API endpoints support the same `PathParams` and `SearchParams` conventions as pages:

```rust
// rs/src/apis/orders/[id]/mod.rs
use anyhow::Result;
use forte_sdk::ForteRequest;
use serde::Serialize;

pub struct PathParams {
    pub id: String,
}

pub struct SearchParams {
    pub include_items: Option<bool>,
}

#[derive(Serialize)]
pub enum Props {
    Ok { id: String, total: f64 },
    NotFound,
}

pub async fn handler(
    _req: ForteRequest<'_>,
    path: PathParams,
    search: SearchParams,
) -> Result<Props> {
    // ...
    Ok(Props::Ok { id: path.id, total: 99.99 })
}
```

## Accessing Request Data

API handlers receive the full `ForteRequest` context, so you can read headers, cookies, and the
request stream:

```rust
use serde::Deserialize;

#[derive(Deserialize)]
struct CreateUserRequest {
    pub name: String,
}

pub async fn handler(req: ForteRequest<'_>) -> Result<Props> {
    let auth = req.headers.get("authorization");
    let mut body = req.body;
    while let Some(chunk) = body.read_chunk().await? {
        process_chunk(chunk).await?;
    }
    // ...
}
```

`req.body` is a single-consumer, backpressured stream. `req.raw_body` is a legacy slice and is
empty for streaming page and API handlers. Buffer only when the endpoint needs a complete value,
and make the bound explicit:

```rust
pub async fn handler(req: ForteRequest<'_>) -> Result<Props> {
    let body = req
        .body
        .json_limited::<CreateUserRequest>(1024 * 1024)
        .await?;
    // body.name is parsed with camelCase→snake_case key conversion
    // ...
}
```

The transport accepts up to 100 MB, but convenience parsing is intentionally bounded and a
buffer limit error is returned before application logic continues. Durable file uploads should
normally use a presigned object-storage URL; presigned uploads are recommended, not required for
large streaming HTTP processing.

## Raw Responses (`ForteResponse`)

Props handlers delegate the HTTP shape to the router: the status is always 200 and the body always goes through `forte_json`. Third-party webhook protocols (Discord interactions, Stripe, GitHub) treat the HTTP status and the exact JSON bytes as part of the contract — Discord, for example, requires 401 on a bad request signature and rejects the endpoint registration otherwise. For those endpoints, alias `Props` to `ForteResponse` and build the full response yourself:

```rust
// rs/src/apis/discord/interactions.rs
use anyhow::Result;
use forte_sdk::{ForteRequest, ForteResponse};
use forte_sdk::http::{Body, Response};

pub type Props = ForteResponse;

pub async fn handler(req: ForteRequest<'_>) -> Result<Props> {
    if !signature_is_valid(&req) {
        return Ok(Response::builder()
            .status(401)
            .body(Body::empty())?);
    }
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&serde_json::json!({ "type": 1 }))?.into())?)
}
```

`ForteResponse` is `http::Response<Body>`; build one with `forte_sdk::http::Response::builder()` (the alias itself cannot host `builder()` — that associated function only exists on `Response<()>`). The router passes it through untouched except for two things: cookie changes in `req.jar` are still written as `Set-Cookie` headers, and any `x-fn0-*` headers are stripped (a raw handler must never trigger the platform's SSR delegation).

Key differences from Props handlers:

- **Status and headers are yours.** Nothing is added or defaulted — set `content-type` yourself.
- **The body bypasses `forte_json`.** Serialize with `serde_json` (re-exported by the SDK) when a protocol requires exact field names without camelCase or `t`-discriminant conversion.
- **The body can stream.** `Body` supports empty, buffered, and streaming values. `Body::channel()` returns a writer/body pair for producing bytes incrementally, and a response from `forte_sdk::http::Client::send` already carries a streaming body, so an upstream response can be proxied straight through.
- **Error handling is unchanged.** `Err` still becomes 302 for `Redirect` and 500 otherwise.

`ForteResponse` handlers are only supported under `rs/src/apis/`. A page handler returning `ForteResponse` fails the build — pages must return `Props` for SSR. The declaration also works directly in the signature (`-> Result<ForteResponse>`) without the alias.

## Discovery Rules

Codegen discovers API endpoints by scanning `rs/src/apis/` recursively (including subdirectories). A file is included if it contains `pub async fn handler` with a return type containing `"Result"` and either `"Props"` or `"ForteResponse"`. When the return type names `Props`, a `pub type Props = ...` alias decides the handler kind: aliasing `ForteResponse` makes it a raw-response handler, aliasing `Redirect` makes it redirect-only.

`mod.rs` at any subdirectory level maps to the handler for that directory (e.g., `apis/orders/mod.rs` → `/api/orders`). There is no generated `mod.rs` in the `apis/` directory.

## Differences from Page Handlers

| Feature | Page (`src/pages/`) | API (`src/apis/`) |
|---|---|---|
| Route prefix | none | `/api/` |
| Response | `x-fn0-next: js` → SSR | `Content-Type: application/json` |
| `Redirect` | supported (via `Err(Redirect::...)`) | supported (same mechanism) |
| React component | required (`fe/src/pages/`) | not applicable |
| `.props.ts` generated | yes | no |

## Error Responses

When the handler returns `Err(e)`:
- If `e` downcasts to `Redirect`, the response is HTTP 302 with a `Location` header.
- Otherwise, the error is printed to stderr via `eprintln!` and the client receives HTTP 500 with body `"Internal Server Error"` (the error detail is not sent to the client).

## `paths.generated.ts`

API routes are included in the `paths.generated.ts` object alongside page routes:

```ts
import { paths } from "./paths.generated";
paths["/api/users"]();                      // "/api/users"
paths["/api/orders/:id"]({ id: "42" });     // "/api/orders/42"
```
