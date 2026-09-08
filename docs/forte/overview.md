# Forte Framework Overview

Forte is a full-stack web framework built on top of [fn0](../fn0/overview.md). It compiles a Rust backend to WebAssembly (WASI Component Model) and pairs it with a React/TypeScript frontend built by Vite.

## Architecture

```
┌────────────────────────────────────────────┐
│  Request                                   │
└────────────┬───────────────────────────────┘
             │
             ▼
┌────────────────────────────────────────────┐
│  fn0 runtime (Wasmtime)                    │
│  Executes backend.wasm                     │
└────────────┬───────────────────────────────┘
             │
             ▼
┌────────────────────────────────────────────┐
│  route_generated.rs  (auto-generated)      │
│  Routes request to page / action handler   │
└────────────┬───────────────────────────────┘
             │ page request
             ▼
┌────────────────────────────────────────────┐
│  Rust page handler                         │
│  Returns Props (serialized to JSON)        │
│  Sets x-fn0-next: js header                │
└────────────┬───────────────────────────────┘
             │
             ▼
┌────────────────────────────────────────────┐
│  JS runtime (server.js SSR bundle)         │
│  React SSR renders Props → HTML            │
└────────────────────────────────────────────┘
```

For API endpoints (`src/apis/`), the Rust handler returns JSON directly — no SSR step.

### SSR pipeline detail

When a page handler sets `x-fn0-next: js`, fn0 delegates to **fn0-ski** — a WinterCG-compatible JavaScript runtime built on V8 (via `deno_core`). fn0-ski executes `dist/server.js`, the Vite SSR bundle produced by `forte build`. That bundle re-imports React, the page component, and the `@forte/react` helpers, then calls `renderToString` with the `Props` JSON returned by the Rust handler. The resulting HTML is sent to the browser.

Key properties of the SSR step:

- **Hook fetches happen here.** During SSR, React components that call `useForteHook` from `@forte/react` trigger a `POST /__self_invoke/<name>` back into the same WASM component. Those results are embedded in `__FORTE_HOOK_CACHE__` in the HTML and reused by the client without a second network request.
- **N isolates per thread, M threads model.** fn0-ski runs N V8 isolates per OS thread. A Forte request is pinned to one thread (by `project_id % thread_count`), so a given project always lands on the same thread's isolate pool.
- **deno_core is vendored and patched.** `vendor/deno/core/` contains a patched copy of `deno_core` that serializes module maps in deterministic order, making the fn0-ski build reproducible. Do not replace it with the upstream crate.

## Key Packages

| Package | Version | Crate | Purpose |
|---|---|---|---|
| `forte-sdk` | 0.9.0 | `forte/sdk` | Runtime library for wasm components (HTTP types, `ForteRequest`, cookie utilities, metrics, etc.) |
| `forte-cli` | 0.6.5 | `forte/cli` | Developer CLI (`forte dev`, `forte build`, `forte deploy`, etc.) |
| `forte-macros` | 0.6.0 | `forte/macros` | Procedural macros: `#[forte_sdk::test]`, `#[forte_doc]` |
| `forte-json` | 0.1.3 | `forte/json` | Streaming JSON serializer used for Props serialization |
| `forte-codegen` | 0.5.0 | `forte/codegen` | Build-script library that generates `route_generated.rs` |
| `forte-wit` | 0.2.0 | `forte/wit` | Embeds the WASI WIT definitions (wasi:http p3 world); provides `extract_wit()` for build scripts |
| `forte-test-runner` | 0.1.0 | `forte/test-runner` | Binary used as the `[target.wasm32-wasip2] runner`; drives `fn0:test-harness/harness` exports test by test |
| `forte-rs-to-ts` | 0.1.10 | `forte/rs-to-ts` | Standalone binary: Rust → TypeScript type generator (uses private rustc APIs; downloaded automatically) |

## Project Structure

See [project-structure.md](project-structure.md) for the full layout.

## Runtime Constraints

Forte backends run inside a single WASM instance that is reused across many concurrent requests (Cloudflare Workers model):

- **No multi-threading.** Use `async`/`await` for concurrency. `std::thread::spawn` and `tokio` are not available in `wasm32-wasip2`.
- **Stateless handlers.** Module-level mutable state must not carry information between requests — another request may be interleaved at any `await` point. Module-level initialization runs once; per-request state belongs inside the handler.
- **No `std::thread::sleep`.** Use `forte_sdk::time_wasi::sleep` instead.

fn0 does not enforce statefulness — violating the stateless constraint causes request-level data leakage.

## Handler Types Quick Reference

| Type | Location | Route | When to use |
|---|---|---|---|
| Page | `rs/src/pages/` | `GET /<path>` | Server-renders a React component; returns `Props` to the SSR step |
| API | `rs/src/apis/` | `/api/<path>` | Returns JSON directly; no React rendering; any HTTP method |
| Action | `rs/src/actions/` | `POST /__forte_action/<name>` | Mutation or query called from the browser via the generated typed client |
| Hook | `rs/src/hooks/` | `POST /__self_invoke/<name>` | Data fetch called during SSR; result is embedded in the HTML and rehydrated on the client |
| WebSocket (inbound) | `rs/src/ws_in/` | `/ws/<path>` | Accept browser WebSocket connections |
| WebSocket (outbound) | `rs/src/ws_out/` | internal | Receive messages from sockets the app opens with `ws_out::connect(url)` |
| WebSocket (singleton) | `rs/src/ws_singleton/` | internal | One shared connection per project (market feed, upstream push channel) |
| Queue task | `rs/src/queue_task/` | internal | Background job enqueued with `enqueue::<name>(input)` |
| Admin task | `rs/src/admin/` | internal | One-off operation run via `forte admin run <name>` |

See the full handler docs: [Pages](pages.md), [API Endpoints](apis.md), [Actions & Tasks](actions.md), [WebSockets](websockets.md).


## Workflow Overview

1. `forte init <name>` — scaffold a new project
2. `forte dev` — run dev server with hot reload
3. `forte add page <path>` / `forte add action <path>` — add handlers
4. `forte build` — compile backend (WASM) + frontend (Vite)
5. `forte login` — authenticate with fn0 Cloud (required before first deploy)
6. `forte cloud init --zone <zone> --project-name <name>` — connect your Cloudflare account (required before first deploy; asks for a setup token once per Cloudflare account, then reuses it for every other project on that account)
7. `forte deploy` — upload to fn0 Cloud

See [cli.md](cli.md) for all commands and [fn0/cloudflare.md](../fn0/cloudflare.md) for the Cloudflare token setup.
