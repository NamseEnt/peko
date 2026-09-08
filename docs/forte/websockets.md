# WebSockets

Forte WebSockets use event callbacks while fn0 owns the network connection. Create inbound route
modules under `rs/src/ws_in`; they are published below `/ws`. Create outbound route modules under
`rs/src/ws_out`; they receive messages from connections opened by the application. Create singleton
route modules under `rs/src/ws_singleton`; fn0 keeps exactly one connection per singleton alive
project-wide and routes its messages to the same handler on any worker.

## Local development

`forte dev` accepts WebSocket upgrades for `/ws` and `/ws/...` and runs the same generated
`on_connect`, `on_message`, and `on_disconnect` callbacks as the deployed worker. Text and binary
messages are delivered through the internal callback request body protocol. Selected subprotocols
and application response headers are forwarded when they pass the same platform header checks;
transport and `x-fn0-*` headers remain owned by fn0.

The local server installs a WebSocket hijack, so `forte_sdk::websocket::send` and
`forte_sdk::websocket::disconnect` operate on active local connections. A Rust source rebuild or
server shutdown closes active connections with close code `1012` and invokes `on_disconnect` on a
best-effort basis. Local development has no distributed ownership or cross-worker routing.

Generated `ws_out` `connect(url)` functions support `ws://` and `wss://` targets and route inbound
messages to the corresponding callbacks. The target must be reachable from the development
machine and present a certificate trusted by the bundled WebPKI root certificates for `wss://`
connections.
Persistent `ws_singleton` connections are not opened by `forte dev`.

## Route example

```rust
use forte_sdk::anyhow::Result;
use forte_sdk::websocket::{
    ConnectDecision, ConnectEvent, DisconnectEvent, IncomingMessage, MessageEvent,
    WebSocketMessage,
};

pub async fn on_connect(event: ConnectEvent) -> Result<ConnectDecision> {
    if event.requested_protocols.iter().any(|protocol| protocol == "chat.v1") {
        Ok(ConnectDecision::accept_with_protocol("chat.v1"))
    } else {
        Ok(ConnectDecision::reject(forte_sdk::http::StatusCode::BAD_REQUEST))
    }
}

pub async fn on_message(event: MessageEvent) -> Result<()> {
    let response = match event.message {
        IncomingMessage::Text(text) => WebSocketMessage::text(text),
        IncomingMessage::Binary(bytes) => WebSocketMessage::binary(bytes),
    };
    forte_sdk::websocket::send(&event.connection_id, response).await?;
    Ok(())
}

pub async fn on_disconnect(_event: DisconnectEvent) -> Result<()> {
    Ok(())
}
```

`on_connect` and `on_message` are required. `on_disconnect` is optional and best-effort. An error
returned by `on_message` or `on_disconnect` is logged and metered but does not disconnect the
client. Use `forte_sdk::websocket::disconnect` for an application-requested graceful close.

`DisconnectEvent` carries `connection_id`, `close_code: Option<u16>`, `reason: Option<String>`, and `cause: DisconnectCause`:

| Cause | Meaning |
| --- | --- |
| `Peer` | The client closed the connection |
| `Application` | `websocket::disconnect` was called |
| `Deployment` | The deploy replaced the running instance |
| `HeartbeatTimeout` | The connection missed a heartbeat |
| `ProtocolError` | WebSocket protocol violation |
| `TransportError` | Network-level transport failure |
| `InternalError` | Internal fn0 error |

## Mapping

| Module | URL |
| --- | --- |
| `ws_in/index.rs` | `/ws` |
| `ws_in/chat.rs` | `/ws/chat` |
| `ws_in/rooms/[room_id].rs` | `/ws/rooms/:room_id` |

A dynamic module declares `PathParams` and receives it after the event argument, matching page and
API routes.

## Outbound routes

Outbound routes do not accept inbound client connections. They define `on_message` and may define
`on_disconnect`; `on_connect` is not allowed. Forte generates a route-bound `connect` function for
each outbound route. Dynamic path segments (`[param]`) are not allowed in outbound route modules —
the build panics if any are present.

| Module | Generated path |
| --- | --- |
| `ws_out/slack.rs` | `crate::ws_out::slack::connect(url)` |
| `ws_out/index.rs` | `crate::ws_out::connect(url)` |

The generated callback path for `ws_out/slack.rs` is `/ws_out/slack`. It is an internal callback
route, not a public WebSocket endpoint.

```rust
let connection_id = crate::ws_out::slack::connect("wss://example.com/socket").await?;
forte_sdk::websocket::send(
    &connection_id,
    forte_sdk::websocket::WebSocketMessage::text("hello"),
)
.await?;
```

## Connect decisions

`ConnectEvent` exposes the connection ID, URI, headers, client address, and requested WebSocket
subprotocols. Return `ConnectDecision::Accept` with an optional selected protocol and response
headers, or `ConnectDecision::Reject` with any non-101 status and response headers.

The selected protocol must be one the client requested. Forte controls the WebSocket handshake,
transport, and every `x-fn0-*` header, so those response headers cannot be overridden.

## Sending

`WebSocketMessage::Text(Body)` and `Binary(Body)` accept buffered or streaming HTTP bodies. A
streaming body is read only when its connection reaches the front of the send queue. Text is
validated incrementally as UTF-8.

```rust
let (mut writer, body) = forte_sdk::http::Body::channel();
forte_sdk::runtime::spawn(async move {
    let _ = writer.write_all(first_chunk).await;
    let _ = writer.write_all(second_chunk).await;
});
forte_sdk::websocket::send(
    &connection_id,
    forte_sdk::websocket::WebSocketMessage::Binary(body),
)
.await?;
```

A successful send means the owning worker wrote and flushed the message, not that the browser
processed it. Inspect `WebSocketSendError::delivery_state()` before deciding whether an
application-level retry is safe. Forte does not retry automatically.

## Recovery

WebSocket delivery is at-most-once and not durable. Deploys close affected project connections
with `1012`. Clients reconnect, fetch authoritative state over HTTP, and only then resume applying
live messages.

## Singleton connections

Singleton routes model **one shared outbound connection per project** — a market-data feed, a
third-party push channel, a chat firehose. Every worker that handles a message for the project
routes it into the same handler; fn0 opens the underlying socket at most once and re-opens it if
it drops.

Create modules under `rs/src/ws_singleton`. Dynamic path segments (`[param]`) are rejected at
build time. `codegen` scans this directory recursively for `.rs` files, and derives the singleton
id from the file path relative to `ws_singleton/`:

| Module | Singleton id | Route |
| --- | --- | --- |
| `ws_singleton/market_feed.rs` | `market_feed` | `/ws_singleton/market_feed` |
| `ws_singleton/feeds/us_market.rs` | `feeds/us_market` | `/ws_singleton/feeds/us_market` |

Every discovered singleton is written to `.forte/ws_singletons.json` by the build (each entry has
`singleton_id` and `route_path`). `forte deploy` reads the manifest and posts the declarations to
control, which validates them and rejects the deploy if any `singleton_id` is empty, duplicated,
or does not match `/ws_singleton/<singleton_id>`.

### Handler shape

Each singleton module defines `connect` (required), `on_message` (required), and optionally
`on_connect` and `on_disconnect`. All are `pub async` and return `Result<...>`; `codegen` panics
at build time otherwise.

```rust
use forte_sdk::anyhow::Result;
use forte_sdk::websocket::{
    DisconnectEvent, IncomingMessage, MessageEvent, SingletonConnectEvent,
    SingletonConnectionOptions, WebSocketMessage,
};

pub async fn connect() -> Result<SingletonConnectionOptions> {
    Ok(SingletonConnectionOptions::new("wss://feed.example.com/market"))
}

pub async fn on_connect(event: SingletonConnectEvent) -> Result<()> {
    tracing::info!(?event.connection_id, ?event.protocol, "singleton up");
    Ok(())
}

pub async fn on_message(event: MessageEvent) -> Result<()> {
    match event.message {
        IncomingMessage::Text(text) => tracing::info!(%text, "tick"),
        IncomingMessage::Binary(bytes) => tracing::info!(len = bytes.len(), "tick"),
    }
    Ok(())
}

pub async fn on_disconnect(_event: DisconnectEvent) -> Result<()> {
    Ok(())
}
```

`connect` is called by fn0 whenever the singleton needs to be (re-)established. It returns a
`SingletonConnectionOptions` describing the target URL and optional per-connection headers or
requested subprotocols. Add extras with the builder-style helpers:

```rust
let mut options = SingletonConnectionOptions::new("wss://feed.example.com/market");
options.headers.insert("authorization", "Bearer ...".parse()?);
options.protocols.push("market.v1".to_string());
Ok(options)
```

The URL must be `ws://` or `wss://`. Reserved headers (`host`, `content-length`, `upgrade`,
`connection`, `sec-websocket-*`, and anything starting with `x-fn0-`) are stripped; fn0 owns
those. Protocols must be non-empty, must not contain whitespace, and must not contain commas.

`on_connect` fires once each time the connection completes the WebSocket handshake and receives
the selected `Sec-WebSocket-Protocol`, if any. `on_message` fires for every inbound frame.
`on_disconnect` is best-effort, with the same `DisconnectCause` mapping as inbound routes; use
`Deployment` and `TransportError` to distinguish an intentional restart from a wire drop.

### Sending, disconnecting, and status

Singletons are addressed by the `ConnectionId` fn0 assigns them. Use
`forte_sdk::websocket::send` and `forte_sdk::websocket::disconnect` the same way as inbound routes;
call them from anywhere in the backend that reaches the id (typically an action, a hook, or a
queue task):

```rust
forte_sdk::websocket::send(
    &connection_id,
    WebSocketMessage::text("{\"op\":\"subscribe\",\"symbol\":\"AAPL\"}"),
)
.await?;
```

There is no public `connect_singleton` in the SDK — fn0 owns opening and re-opening the socket
in response to the manifest declaration, so user code never calls it. The current status of a
singleton (which worker owns it, when it last handshook, the last error, if any) is tracked by
the fn0 control plane; a UI can pull it through the `websocket_singleton_status` admin action on
control.

### Lifecycle

- Registered at deploy time from `.forte/ws_singletons.json`; a rename or removal takes effect on
  the next `forte deploy`.
- fn0 keeps at most one active connection per `(project, singleton_id)` at any time, even across
  many worker instances. A new deploy closes the old connection before starting the new one.
- Delivery is still at-most-once. A dropped message is not replayed.
- Owner leases are renewed periodically; when a worker vanishes, control hands the singleton to
  another worker and calls `connect` again there.

See [Limits & Quotas](../fn0/limits.md) and the internal
[WebSocket design](../design/forte-websockets.md) for queue, size, and lifecycle details.
