# Forte Code Generation

Forte uses two code generation tools:

1. **`forte-codegen`** — a build-script library that generates `route_generated.rs` and TypeScript helpers from Rust source files
2. **`forte-rs-to-ts`** — a standalone binary that extracts Rust types and converts them to TypeScript

Both run automatically during `forte build` and `forte dev`.

## `forte-codegen` (build.rs)

Every Forte project's `build.rs` calls:

```rust
fn main() {
    forte_codegen::generate_routes();
}
```

This scans the backend source tree and writes:

| Output | Description |
|---|---|
| `rs/src/route_generated.rs` | Complete HTTP dispatch, WASM export, and route matching |
| `fe/src/paths.generated.ts` | Type-safe path builder object |
| `rs/src/actions/mod.rs` | Module declarations for discovered actions |
| `rs/src/admin/mod.rs` | Module declarations for discovered admin tasks |
| `rs/src/queue_task/mod.rs` | Module declarations for discovered queue tasks |

It also updates the `FORTE-MANAGED` block in `rs/src/lib.rs`.

Cargo reruns the build script when any of these directories change:
- `src/pages`
- `src/apis`
- `src/hooks`
- `src/actions`
- `src/queue_task`
- `src/admin`
- `src/ws_in`
- `src/ws_out`
- `src/ws_singleton`
- `public`
- `Cargo.lock` (ensures the build script re-runs after dependency version changes, since declaring any `rerun-if-changed` disables Cargo's default file-based detection)

### Discovery Rules

Each handler type is discovered by statically parsing the Rust source:

| Directory | Discovery rule | Scan depth |
|---|---|---|
| `src/pages/` | File contains `pub async fn handler` returning `Result<Props>` or `Result<Redirect>` | Recursive |
| `src/apis/` | Same as pages; route is prefixed with `/api/` | Recursive |
| `src/hooks/` | File has `struct Input`, `Output` type (struct or enum), and `pub async fn handler` | Top-level only (`.rs` files) |
| `src/actions/` | `.rs` file OR `<name>/mod.rs` directory module; both must have `struct Input`, `Output`, and `pub async fn handler` | Top-level only |
| `src/queue_task/` | File has `struct Input` or `type Input = ()` and `pub async fn handle` | Top-level only (`.rs` files) |
| `src/admin/` | Same as queue tasks | Top-level only (`.rs` files) |
| `src/ws_in/` | File defines `pub async fn on_connect` and `pub async fn on_message`; `on_disconnect` optional | Recursive |
| `src/ws_out/` | File defines `pub async fn on_message` (`on_connect` must be absent); `on_disconnect` optional; no dynamic path segments | Recursive |
| `src/ws_singleton/` | File defines `pub async fn connect` returning `SingletonConnectionOptions` and `pub async fn on_message`; `on_connect` and `on_disconnect` optional; no dynamic path segments; also writes `.forte/ws_singletons.json` for `forte deploy` | Recursive |
| `public/` | Every file (recursively); served verbatim at the file's relative path | Recursive (all files) |

**Important:** `src/actions/` supports two layouts: flat `.rs` files (`user_login.rs`) or directory modules (`user_login/mod.rs`). Only the top level of `src/actions/` is scanned; nested paths like `user/login.rs` are not discovered. All other handler directories (`hooks/`, `queue_task/`, `admin/`) support only flat `.rs` files.

`src/pages/` and `src/apis/` are scanned recursively, so nested directory structures work for URL routing.

**`mod.rs` behavior differs by directory type:**

- `src/pages/` and `src/apis/`: `mod.rs` at any subdirectory level is the handler for that directory (e.g., `pages/about/mod.rs` handles `/about`). It is **not** skipped.
- `src/hooks/`, `src/queue_task/`, and `src/admin/`: the auto-generated `mod.rs` at the directory root is excluded from discovery.
- `src/actions/`: the top-level `mod.rs` is excluded, but `<name>/mod.rs` inside a named subdirectory is a supported layout.

Flat `.rs` files are also supported for pages and APIs. `pages/about.rs` is equivalent to `pages/about/mod.rs` and maps to `/about`.

The return type check for pages and APIs matches on the string representation of the return type: the type must contain both `"Result"` and `"Props"`, or both `"Result"` and `"Redirect"`. Name your return type `Props` (or any type alias that includes the word `Props`) to be discovered.

### Route Mapping

| File path | Route |
|---|---|
| `pages/index/mod.rs` or `pages/index.rs` | `/` |
| `pages/about/mod.rs` | `/about` |
| `pages/product/[id]/mod.rs` | `/product/:id` |
| `apis/users.rs` | `/api/users` |

Dynamic segments are `[param]` in the directory name, mapped to `:param` in the route.

### Static Files (`public/`)

Files in `rs/public/` are discovered recursively and embedded in the generated code via `include_bytes!`. Each file gets a match arm in the router that returns its bytes with the appropriate `Content-Type`. Static file routes are checked before page/API routes, so they always win on path conflicts.

See [project-structure.md](project-structure.md#rspublic--embedded-static-files-optional) for the content-type mapping and usage notes.

### Generated Router Internals (`route_generated.rs`)

The generated file:
- Declares all discovered modules with `#[path = "..."] mod ...`
- Defines the WASM component export via `wit_bindgen::generate!`
- Implements `wasi::http::handler::Guest` which calls `forte_sdk::serve::serve`
- Defines `dispatch_inner` which routes to pages, API endpoints, actions, hooks, queue tasks, and admin tasks
- Handles path parameter extraction (`PathParams`) and search parameter parsing (`SearchParams`)
- Generates the `Redirect` enum with variants for every discovered page

**Special routes handled by the generated dispatcher:**

| Path prefix | Handler |
|---|---|
| `/__self_invoke/<name>` | Hook handler |
| `/__forte_action/<name>` | Action handler |
| `/__fn0_queue_task/execute` | Queue task executor |
| `/__forte_admin/<name>` | Admin task handler |

### `paths.generated.ts`

Emitted alongside `route_generated.rs`. Exports a `paths` const with one entry per page:

```ts
export const paths = {
  "/": () => "/",
  "/product/:id": ({ id }: { id: string }) => `/product/${id}`,
} as const;
```

Rust types are mapped to TypeScript: `String`/`&str` → `string`, integer types → `number`, `bool` → `boolean`.

## `generate_env` (build.rs)

`forte-codegen` also exports a second build-script function, called by default in the `forte init` template:

```rust
fn main() {
    forte_codegen::generate_routes();
    forte_codegen::generate_env();
}
```

`generate_env` reads the project's env files — `<project>/env.yaml` and `<project>/env.local.yaml` — and writes `rs/src/env_generated.rs`. For the union of their keys (excluding the internal `__dek` key, which is auto-managed by the vault) it generates a zero-cost accessor each:

```rust
pub fn cookie_secret() -> &'static str { ... }  // for COOKIE_SECRET
pub fn turso_url() -> &'static str { ... }       // for TURSO_URL
```

Values are loaded from the real environment at first use via `std::sync::LazyLock`. The env files only determine which accessor functions exist; the returned value is always the runtime value of the variable. Encrypted (`secret:`) entries generate an accessor exactly like plain ones — the ciphertext is never read at build time.

Because the accessors are derived from `env.yaml` itself, an accessor exists for everything you deploy. Each one panics if the variable is unset at runtime, which is what you get in `forte dev` for a `secret:` entry with no `env.local.yaml` override — `forte dev` lists those keys on startup.

`cargo:rerun-if-changed` is emitted for both files, so the module is regenerated when either changes.

To use the generated module, add `mod env_generated;` to your `lib.rs` (outside the FORTE-MANAGED block) and call `env_generated::cookie_secret()` etc.

## `forte-rs-to-ts`

A standalone binary (`forte-rs-to-ts`) that reads the Rust source tree and generates TypeScript type files. Run automatically during `forte build`.

> **Implementation note:** `forte-rs-to-ts` uses private Rust compiler APIs (`rustc_driver`, `rustc_hir`, `rustc_middle`, etc.) to resolve and analyze types accurately. Because these APIs are unstable and tied to a specific nightly compiler build, the binary is distributed as a pre-built release artifact bundled with a matching sysroot — it cannot be compiled from source with an ordinary `cargo install`. The CLI downloads the correct version automatically on first use; the binary is cached in `~/.forte/bin/forte-rs-to-ts-<version>/forte-rs-to-ts` (shared across all projects on the machine). Supported platforms: `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.

For each page handler (`rs/src/pages/<path>/mod.rs`), it generates:
- `fe/src/pages/<path>/.props.ts` — the `Props` type as a TypeScript discriminated union

For each action handler (`rs/src/actions/<name>.rs`), it generates:
- `fe/src/actions/.generated/<name>.ts` — Zod schema and TypeScript types for `Input` and `Output`

For each hook handler (`rs/src/hooks/<name>.rs`), it generates:
- `fe/src/hooks/.generated/use<Name>.ts` — Zod schema and TypeScript types for `Input` and `Output`, plus a React Suspense hook `use<Name>(input: Input): Output` backed by `useForteHook` from `@forte/react`. The file name is `use` followed by the hook name with its first character uppercased and the rest unchanged (e.g., `user_session.rs` → `useUser_session.ts`, `session.rs` → `useSession.ts`).

Each generated file imports `zod` and exports:
```ts
export const InputSchema = z.object({ ... });
export type Input = z.infer<typeof InputSchema>;

export const OutputSchema = z.discriminatedUnion("t", [ ... ]);
export type Output = z.infer<typeof OutputSchema>;
```

The JSON shape for enum variants follows the same rules as `forte-json` serialization:

| Variant kind | JSON | TypeScript |
|---|---|---|
| Unit: `Ok` | `{"t":"Ok"}` | `{ t: "Ok" }` |
| Tuple/newtype: `Ok(String)` | `{"t":"Ok","v":"..."}` | `{ t: "Ok", v: string }` |
| Struct: `Ok { message: String }` | `{"t":"Ok","message":"..."}` | `{ t: "Ok", message: string }` |

Struct variant fields are spread flat alongside `t` — there is no `v` wrapper. Rust field names become camelCase.

## The `FORTE-MANAGED` Block

`rs/src/lib.rs` must contain this block for codegen to work:

```rust
// === FORTE-MANAGED START ===
// Auto-managed by `forte build`. Do not edit between the START/END markers.
pub mod actions;      // present only when src/actions/ has handlers
pub mod admin;        // present only when src/admin/ has handlers
pub mod queue_task;   // present only when src/queue_task/ has handlers
mod route_generated;
pub use route_generated::enqueue; // present only when queue tasks exist
pub use route_generated::ws_out;  // present only when src/ws_out/ has routes
// === FORTE-MANAGED END ===
```

`forte-codegen` replaces the content between the markers with the appropriate subset of declarations — only the lines for features actually present in the project are emitted. If the markers are missing, the build panics with instructions to add them.
