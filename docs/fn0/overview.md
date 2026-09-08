# fn0 Platform Overview

fn0 (pronounced "f-n-zero") is a FaaS (Function-as-a-Service) platform powered by [Wasmtime](https://wasmtime.dev/). It executes WebAssembly components compiled to WASI 0.3.0 (Component Model).

## Core Concepts

### Execution Model

fn0 uses the same model as Cloudflare Workers:

- A single WASM instance (and, for JS deployments, a single V8 isolate) is reused to serve many concurrent requests on the same worker thread.
- Handlers must be **effectively stateless across requests**. Module-level mutable state must not carry information between requests because another request may be interleaved at any `await` point.
- Module-level initialization runs once. Per-request setup belongs inside the handler.

fn0 does not enforce this contract. Violating it causes request-level data leakage or inconsistent behavior.

### Response Specification

The WASM component returns an HTTP response. fn0 inspects it:

- If the response has status 200 and the header `x-fn0-next: js`, fn0 delegates to the named runtime (currently only `js` — the Ski JavaScript runtime).
- Otherwise fn0 forwards the response as-is.
- All `x-fn0-*` headers are stripped from the response before it is sent to the client.

Forte uses `x-fn0-next: js` for page handlers (SSR) and skips it for API endpoints and actions.

### fn0 Cloud Limits

These limits apply to fn0 Cloud. Self-hosted deployments can remove them.

| Limit | Value |
|---|---|
| Request headers | 128 KB |
| Request body | 100 MB |
| Response headers | 128 KB |
| Response body | Unlimited |
| Memory | 128 MB |
| CPU time | 50 ms |
| Max duration | 15 seconds |
| Subrequests (external HTTP) | 50 per request |

The request-body value is a transport limit. Applications can process bodies up
to that limit with bounded memory only by streaming; buffering remains subject
to the 128 MB memory limit. Durable file uploads should normally use presigned
object-storage URLs, while streaming through compute remains supported for cases
that require it. Request and response streaming is implemented end to end; see
[HTTP Body Streaming](../design/http-body-streaming.md).

### Cluster Architecture (Internal)

- **Monolith architecture** — no microservices.
- **Public ingress (OCI)**: Cloudflare (orange proxy) → OCI L4 Network Load Balancer (always-free) → worker pool. Workers run in an OCI InstancePool with AutoScaling; the NLB is the single entry.
- **Intra-node dispatch**: Each worker node runs a pool of N OS threads (default = CPU count). Incoming requests are dispatched to a specific thread using a hash of the `project_id` modulo N, so a given project always lands on the same thread within a node. Thread queue capacity is 256 per thread; a full queue returns HTTP 503.
- **Blue-green deploys**: `fn0-worker-agent` polls the control DB for the target worker image, starts the new container, waits for it to pass health checks, and then drains and stops the old one. `fn0-worker-proxy` reroutes new TCP connections to the active container. Established WebSockets receive `1012` and the old container remains alive through their graceful close timeout.
- **WebSocket routing**: The owning `fn0-worker` keeps the socket. The fn0-control Turso database stores connection ownership, and direct TLS-protected QUIC streams carry remote `send` and `disconnect` commands. Turso never carries message payloads; bounded control reconciliation removes confirmed stale records without leases or heartbeats.
- WASM modules are cached in memory after the first download. On subsequent requests, the compiled bundle version is checked against the manifest; re-download only if the version changed.

## Packages

| Package | Version | Description |
|---|---|---|
| `fn0` | 0.6.7 | Core FaaS runtime (`ExecutionContext`, `Bundle`, `build_engine`) |
| `fn0-cli` | 0.1.18 | Local development CLI |
| `fn0-worker` | 0.4.11 | Worker binary (distributed execution node) |
| `fn0-worker-agent` | 0.1.6 | Per-instance container supervisor (blue-green deploys, in-host TCP proxy) |
| `fn0-worker-proxy` | 0.1.0 | Tiny TCP forwarder fronting fn0-worker containers; polls a target file written by worker-agent |
| `fn0-deploy` | 0.3.2 | fn0 Cloud deployment client |
| `fn0-wasmtime` | 0.2.0 | Wasmtime wrapper with fn0-specific config |
| `fn0-ski` | 0.1.11 | WinterCG-compatible JS runtime (Deno-based, no Node.js) |
| `fn0-compiler` | 0.1.0 | CLI tool: compiles `.wasm` → `.cwasm` (Wasmtime pre-compiled native format); used internally by the platform |

## fn0-cli Commands

The fn0 CLI (`fn0/cli`) provides tooling for projects deployed directly to fn0 without Forte:

| Command | Description |
|---|---|
| `fn0 init [--name <name>]` | Scaffold a new fn0 project (prompts for framework and language) |
| `fn0 build` | Compile the project to WASM |
| `fn0 local [--port <port>]` | Run locally on the given port (default: auto) |
| `fn0 deploy` | Build and deploy to fn0 Cloud |
| `fn0 destroy` | Delete the deployed project |
| `fn0 rename <new-name>` | Rename the deployed project |
| `fn0 login [token]` | Authenticate with fn0 Cloud |
| `fn0 admin run <task>` | Run an admin task against the deployed app |
| `fn0 env set <key> <value> [--secret]` | Set an env entry (plain by default; `--secret` encrypts via vault and injects through vault_hijack at runtime) |
| `fn0 env list` | List env entries with their kind (plain / secret) |
| `fn0 env unset <key>` | Remove an env entry |
| `fn0 purge <key>...` | Invalidate the CDN edge copy of one or more public objects (needed after presigned PUT uploads; `public::put` already purges automatically) |
| `fn0 purge-page <path>...` | Invalidate the CDN edge copy of one or more `cache_static` pages (use when runtime data changes; see [pages.md](../forte/pages.md#invalidating-a-page-without-deploying)) |

> **Note:** Most Forte developers use `forte` CLI instead of `fn0` CLI. `fn0` CLI is for projects that use fn0 as a raw FaaS platform (e.g., Hono-based TypeScript apps).

## Supported Languages

- **Rust** — primary target; compiles to `wasm32-wasip2`
- **JavaScript / TypeScript** — via the Ski runtime (WinterCG subset, no Node.js APIs)

## Supported Cloud Providers

- Oracle Cloud Infrastructure (OCI) — instance pool, NLB, vault

## Supported CDN Providers

- Cloudflare Workers (integration)

## Supported Code Storage Providers

- File system (including NFS, e.g. AWS EFS)
- S3 and compatible object storage (via `opendal`)

## Observability

fn0 has built-in OpenTelemetry support:

- OTLP span exporter via `fn0/fn0/src/otlp_hijack.rs` (worker-side) and `forte/sdk/src/otel.rs` (WASM-side)
- Structured logging via `tracing`
- Distributed tracing with per-request spans

For Forte apps running on fn0 Cloud, traces are exported to `http://fn0-otel.fn0.dev/v1/traces`. The service name is controlled via the `OTEL_SERVICE_NAME` environment variable (defaults to `"forte-app"`).

For self-hosted fn0 worker deployments, configure the OTLP endpoint via environment variables on the worker binary:

| Variable | Required | Default | Description |
|---|---|---|---|
| `FN0_OTLP_TARGET_HOST` | Yes | — | OTLP collector hostname (the worker's local Alloy) |
| `FN0_OTLP_TARGET_SCHEME` | Yes | — | URL scheme for the OTLP collector: `http` or `https` |
| `FN0_OTLP_AUTH` | Yes | — | Base64-encoded Basic auth credentials (`user:token`); use empty string for unauthenticated |
| `FN0_OTLP_TARGET_PATH_PREFIX` | No | `""` | Path prefix prepended to every OTLP request path |
| `FN0_OTLP_PLACEHOLDER_HOST` | No | `fn0-otel.fn0.dev` | Placeholder hostname used inside WASM apps |

The worker intercepts outgoing OTLP requests that target `FN0_OTLP_PLACEHOLDER_HOST` and rewrites them to `FN0_OTLP_TARGET_HOST` with the configured auth header.

## Worker Environment Variables (Self-Hosting)

When running `fn0-worker` yourself, set these environment variables on the binary:

### Storage

| Variable | Required | Default | Description |
|---|---|---|---|
| `CWASM_BUCKET` | Yes | — | S3 bucket name for pre-compiled `.cwasm` bundles |
| `S3_ENDPOINT` | Yes | — | S3-compatible endpoint URL |
| `S3_REGION` | No | `us-east-1` | S3 region |
| `AWS_ACCESS_KEY_ID` | Yes | — | S3 access key |
| `AWS_SECRET_ACCESS_KEY` | Yes | — | S3 secret key |
| `FN0_BUNDLE_CACHE_SIZE_BYTES` | No | `536870912` (512 MB) | In-memory bundle cache size |

### Networking / TLS

| Variable | Required | Default | Description |
|---|---|---|---|
| `HTTP_PORT` | No | `443` | Port for user HTTPS traffic |
| `FN0_WORKER_OPS_PORT` | No | `9090` | Port for ops endpoints (`/ready`, `/drain`, `/status`, `/health`) |
| `ORIGIN_CERT_PEM` | Yes | — | PEM-encoded TLS origin certificate (or use `ORIGIN_CERT_PEM_BASE64`) |
| `ORIGIN_KEY_PEM` | Yes | — | PEM-encoded TLS private key (or use `ORIGIN_KEY_PEM_BASE64`) |
| `FN0_APEX_DOMAIN` | No | — | Apex domain to route when no project hostname matches |
| `FN0_APEX_PROJECT_ID` | No | — | Project ID to serve for `FN0_APEX_DOMAIN` requests; both must be set together |

The first byte on `HTTP_PORT`: `0x16` (TLS ClientHello) routes to user traffic; anything else routes to health checks. `FN0_WORKER_OPS_PORT` serves ops endpoints unconditionally.

### Telemetry

| Variable | Required | Default | Description |
|---|---|---|---|
| `OTLP_ENDPOINT` | Yes | — | Worker's own OTLP telemetry endpoint |
| `FN0_OTLP_TARGET_HOST` | Yes | — | OTLP collector hostname for guest (WASM) traces |
| `FN0_OTLP_TARGET_SCHEME` | Yes | — | URL scheme for guest OTLP collector: `http` or `https` |
| `FN0_OTLP_AUTH` | Yes | — | Base64-encoded Basic auth (`user:token`) for guest OTLP; empty string for unauthenticated |
| `FN0_OTLP_TARGET_PATH_PREFIX` | No | `""` | Path prefix prepended to every guest OTLP request path |
| `FN0_OTLP_PLACEHOLDER_HOST` | No | `fn0-otel.fn0.dev` | Placeholder hostname intercepted inside WASM apps |

### Platform Services

| Variable | Required | Default | Description |
|---|---|---|---|
| `TURSO_GROUP_TOKEN` | Yes | — | Auth token for the Turso group (used by `turso_hijack`) |
| `TURSO_DB_HOST_SUFFIX` | Yes | — | Host suffix for Turso database URLs (e.g. `.turso.io`) |
| `TURSO_PLACEHOLDER_HOST` | No | `fn0-db.fn0.dev` | Placeholder hostname intercepted for Turso requests |
| `FN0_CONTROL_PROJECT_ID` | Yes | — | Project ID of the fn0 control service; used by public storage and static page cache hijacks |
| `FN0_OBJECT_STORAGE_PLACEHOLDER_HOST` | No | `fn0-object-storage.fn0.dev` | Placeholder hostname for private object storage requests |
| `FN0_PUBLIC_STORAGE_PLACEHOLDER_HOST` | No | `fn0-public-storage.fn0.dev` | Placeholder hostname for public object storage requests |
| `FN0_STATIC_PAGE_CACHE_PLACEHOLDER_HOST` | No | `fn0-static-page-cache.fn0.dev` | Placeholder hostname for static page cache purge requests |

### OCI Vault

| Variable | Required | Description |
|---|---|---|
| `FN0_VAULT_CRYPTO_ENDPOINT` | Yes | OCI Vault cryptographic endpoint URL |
| `FN0_VAULT_KEY_OCID` | Yes | OCID of the encryption key |
| `FN0_VAULT_OCI_TENANCY_ID` | Yes | OCI tenancy OCID |
| `FN0_VAULT_OCI_USER_ID` | Yes | OCI user OCID |
| `FN0_VAULT_OCI_FINGERPRINT` | Yes | API key fingerprint |
| `FN0_VAULT_OCI_PRIVATE_KEY_BASE64` | Yes | Base64-encoded PEM private key for the OCI API signing key |

## Hijack Architecture

fn0 uses "hijack" components to inject platform services into the WASM execution environment without modifying the user's code:

| Hijack | Purpose |
|---|---|
| `turso_hijack` | Injects Turso/libSQL database connection |
| `object_storage_hijack` | Routes & SigV4-signs per-project private object storage requests |
| `public_storage_hijack` | Routes public object storage requests (no SigV4 signing required) |
| `otlp_hijack` | Injects OpenTelemetry OTLP endpoint |
| `queue_hijack` | Intercepts outgoing queue requests |
| `vault_hijack` | Injects secrets (Vault integration) |
| `websocket_hijack` | Dispatches WebSocket `send` and `disconnect` commands via QUIC |
| `static_page_cache_hijack` | Routes `static_page_cache::purge` calls; shares the hourly purge budget with public storage purges |
| `cross_project_enqueue_hijack` | Routes cross-project queue enqueue calls |
| `cross_project_invoke_hijack` | Routes cross-project direct invocations |

These are configured on `ExecutionContext` via builder methods:

```rust
let ctx = ExecutionContext::new(engine, linker, bundle_cache)
    .with_turso_hijack(turso_hijack)
    .with_otlp_hijack(otlp_hijack)
    .with_queue_hijack(queue_hijack)
    .with_cross_project_enqueue_hijack(enqueue_hijack)
    .with_cross_project_invoke_hijack(invoke_hijack)
    .with_vault_hijack(vault_hijack)
    .with_object_storage_hijack(object_storage_hijack)
    .with_public_storage_hijack(public_storage_hijack)
    .with_websocket_hijack(websocket_hijack)
    .with_static_page_cache_hijack(static_page_cache_hijack);
```
