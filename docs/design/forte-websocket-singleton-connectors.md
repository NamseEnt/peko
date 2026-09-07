# Forte Persistent Outbound WebSocket Design

Status: implemented

This document defines project-scoped outbound WebSockets that Forte keeps connected without an
application invocation calling `connect`. It extends the physical outbound transport in
[Forte WebSocket Design](./forte-websockets.md).

## Application configuration

Persistent WebSockets are declared as Rust modules below `rs/src/ws_singleton`. For example,
`rs/src/ws_singleton/market_feed.rs` declares `singleton_id = "market_feed"` and the internal route
`/ws_singleton/market_feed`. Nested files such as `feeds/us/market.rs` produce
`singleton_id = "feeds/us/market"`. Dynamic path segments are rejected.

```rust
use forte_sdk::anyhow::Result;
use forte_sdk::websocket::{
    DisconnectEvent, MessageEvent, SingletonConnectEvent, SingletonConnectionOptions,
};

pub async fn connect() -> Result<SingletonConnectionOptions> {
    Ok(SingletonConnectionOptions::new(
        "wss://stream.example.com/market",
    ))
}

pub async fn on_connect(_event: SingletonConnectEvent) -> Result<()> {
    Ok(())
}

pub async fn on_message(_event: MessageEvent) -> Result<()> {
    Ok(())
}

pub async fn on_disconnect(_event: DisconnectEvent) -> Result<()> {
    Ok(())
}
```

`connect` and `on_message` are required. `on_connect` and `on_disconnect` are optional. `connect`
returns the URL, handshake headers, and requested subprotocols. Code generation writes the derived
singleton IDs and callback routes to `.forte/ws_singletons.json`; this file is an internal deploy
artifact and is not application configuration. A project may declare at most 1,000 singletons.

## Identity

The logical identity is:

```text
(project_id, singleton_id)
```

One worker normally owns one physical connection for that identity. A physical connection uses the
existing opaque `connection_id`. Control uses an internal, short-lived `claim_token` to fence
assignment attempts. The token is not application-visible and does not change the logical identity.
The same claim is idempotent on a worker; a different singleton ID creates a different connection
even when URL and receive path are equal.

## Deployment state

Control stores every deployment's declarations under its `code_version`. Only declarations for the
project's active code version are eligible for connection. This makes the receive path a deployment
artifact instead of permanent configuration.

After activation, `deploy_artifact_prune` also prunes these declaration documents, including empty
declarations. It queries one project's configurations in pages and retains the active code version,
every newer version that may still be uploading or awaiting activation, and the newest two versions
below active. Older versions are deleted; a failed deployment becomes eligible once later active
versions move it outside this retained history. If no later deployment activates, its configuration
remains retained.

Each deletion re-reads the manifest and the exact `(project_id, code_version)` document in one
transaction. Cleanup stops if the project disappears, the active version changes, or activation is
no longer `active`. Already-deleted documents are harmless on retry. Cleanup logs scanned, retained,
and deleted document counts and does not change singleton runtime leases or ownership.

When a worker adopts a new project code version, it closes every WebSocket for that project with
`1012 Service Restart`. Control does not reconnect a persistent WebSocket from the previous
deployment. It uses the declaration shipped with the active deployment, so a renamed handler either
produces the new path or fails deployment validation rather than reconnecting to a stale path.

Deployments use the platform's rolling consistency model. HTTP requests, WebSocket callbacks, and
queue work may briefly run on old and new code during rollout. Applications that cannot tolerate
that overlap must quiesce traffic and drain asynchronous work. Persistent WebSockets add no stronger
cross-version guarantee; closing them prevents an old connection from surviving indefinitely.

## Control and worker responsibilities

Control is the only caller of the worker's internal operation:

```text
connect_singleton(project_id, singleton_id, claim_token, url, receive_path) -> connection_id
```

Control reads the active deployment declaration from its database. The worker never reads that
record and reuses the existing URL-based outbound WebSocket transport.

Control serializes assignment for one `(project_id, singleton_id)`. A committed, unexpired lease
prevents another worker from receiving the same assignment. Network connection establishment occurs
after the database transaction.

## Connection lease

The worker reports the following internal state to control:

```text
project_id
singleton_id
claim_token
connection_id
status = heartbeat | disconnected
```

The existing `connection_id` lets control ignore a late heartbeat or disconnect from a replaced
physical connection.

The initial lease is 60 seconds. A worker renews substantially earlier and uses a local safety
deadline shorter than the control lease. If it cannot renew by that deadline, it stops admitting
sends and callback dispatch, then closes the socket. Control assigns a replacement only after the
stored lease expires. A crashed worker cannot send `disconnected`; lease expiry is its recovery
path.

## Reconciliation

The previous implementation walked every active project on every control tick, loaded each
singleton runtime with a separate database request until it found one reconnect candidate, and
then enqueued deployment activation for the whole project. Its work grew with the total number of
projects and declarations, produced N+1 runtime reads, and retried healthy declarations together
with the failed declaration. One project-level error also stopped the remaining scan.

The control tick scans at most 64 projects and 256 declarations per invocation. It stores a
`(project_id, singleton_id)` cursor, reads each project's runtime records in one query, and enqueues
only missing, expired, or old-version singletons. For each targeted task it:

1. skips an unexpired current connection or claim;
2. claims the singleton in a short database transaction;
3. calls `connect_singleton` on the worker executing the targeted queue task;
4. records the returned `connection_id` only when the claim token still matches;
5. releases its own failed claim without deleting a replacement claim.

The tick also removes runtime state that no longer exists in the active deployment. A worker that
still owns the removed state can no longer renew it and self-fences at its local safety deadline.

## Delivery behavior

Message callbacks use the declared outbound handler and retain the existing online-only,
at-most-once behavior. The platform does not replay messages received while disconnected and does
not retry ambiguous sends. `send` and `disconnect` continue to address the physical
`connection_id`; a logical singleton send API is outside the initial scope.

## Failure behavior

| Event | Result |
| --- | --- |
| Duplicate control tick | One database claim wins; the other skips the singleton |
| Duplicate worker request | The worker keeps the current singleton connection |
| Upstream dial failure | The short claim expires and a later tick retries |
| Worker crash | The socket disappears and control reassigns after lease expiry |
| Worker cannot renew | The worker self-closes before control can reassign |
| Late disconnect | Control ignores it when `connection_id` is no longer current |
| Project deployment | Workers close project sockets; control uses only the new deployment declaration |
| Declaration removed | Control stops reconciling it and closes the previous physical connection |
