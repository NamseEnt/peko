# HTTP Body Streaming

Status: implemented. Tracked by
[GitHub issue #108](https://github.com/NamseEnt/fn0/issues/108).

## Product contract

fn0 Cloud accepts HTTP request bodies up to 100 MB. This is a transport-size
limit, not a promise that the complete body fits in application memory.

Incoming request bodies must be exposed to Forte applications as a
single-consumer, backpressured stream. An application that processes and
discards bounded chunks must be able to handle a request near the 100 MB limit
without retaining the complete body in WASM memory.

The following limits apply independently:

| Limit | Value |
|---|---:|
| Request body | 100 MB |
| WASM memory | 128 MB |
| CPU time | 50 ms |
| Wall time | 15 seconds |

A request within the body-size limit can still fail because it exceeds memory,
CPU time, or wall time. In particular, buffering a large body or materializing a
large JSON value can exhaust the 128 MB WASM memory limit. Slow uploads can
exceed the 15-second wall-time limit, and so can slow downloads: the response
stream shares the same deadline, so an unlimited response size does not mean
unlimited delivery time.

Presigned object-storage URLs are the recommended path for durable file uploads.
They are not required: applications must remain able to stream large HTTP bodies
through compute for use cases such as hashing, incremental parsing,
transformation, and proxying.

## Enforcement

fn0-worker must enforce the 100 MB limit while reading the request body.

- A valid `Content-Length` above 100 MB is rejected with HTTP 413 before the
  application is invoked.
- `Content-Length` is not trusted as the only enforcement mechanism.
- A request without a length, or one using chunked transfer encoding, is stopped
  with HTTP 413 as soon as the received byte count crosses 100 MB.
- A client disconnect, size violation, or invocation timeout cancels body
  delivery and associated application work.

## Streaming requirements

Backpressure must remain intact across the entire path:

```text
Cloudflare
  -> OCI Network Load Balancer
  -> fn0-worker-proxy
  -> Hyper
  -> project worker
  -> WASI HTTP
  -> Forte handler
```

No layer may eagerly collect the complete request. Per-stream queues and
aggregate buffering must be bounded so concurrent large requests cannot turn
streaming into unbounded host memory use.

Forte may provide convenience operations for reading bytes, text, JSON, or form
data. Those operations buffer data and must make their memory cost and any
smaller buffering limit explicit. The 100 MB transport limit does not imply that
these convenience operations are safe for a 100 MB body.

Response bodies follow the same streaming principle. The documented unlimited
response-body size requires fn0-worker to forward response chunks with
backpressure instead of collecting the complete response before sending it.

## Implementation notes

The worker enforces the transport limit while preserving the Hyper request-body
stream through the WASI HTTP boundary. The Forte SDK exposes the request as a
single-consumer `Body` stream. Generated page and API handlers receive it in
`ForteRequest::body`; the legacy `raw_body` slice is empty on those routes.
Typed action and hook deserialization uses an explicit 1 MiB convenience buffer,
so it cannot materialize an arbitrary 100 MB transport body.

The reused WASM instance has a 128 MB linear-memory ceiling and can serve
concurrent requests. The worker limits active body chunks to 64 KiB per stream
and an 8 MiB aggregate request buffer; application-level buffering remains
explicitly separate from this transport budget.

The worker forwards guest response chunks to Hyper without collecting the complete
response body.

## WebSocket relationship

This contract does not set the WebSocket message-size limit. An HTTP body is a
byte stream, while a WebSocket `on_message` callback receives one complete
message. WebSocket messages therefore require an independent atomic-message
limit. Applications should split large real-time data at the application
protocol level or use HTTP and presigned object-storage URLs where appropriate.

## Completion criteria

The implementation is complete when tests demonstrate all of the following:

- A streaming handler consumes a request near 100 MB with bounded guest memory.
- Concurrent large streams apply bounded buffering and backpressure.
- Oversized fixed-length and chunked requests receive HTTP 413.
- Disconnect and timeout cancellation propagate through every layer.
- A large response reaches the client without being fully collected by
  fn0-worker.
- Existing small pages, APIs, actions, hooks, queue tasks, and SSR behavior remain
  compatible through the request API migration.
