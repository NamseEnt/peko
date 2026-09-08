# Forte Actions

Actions are server-side functions callable from the browser via HTTP POST. They accept a typed JSON input and return a typed JSON output.

## File Location

Place action handlers under `rs/src/actions/`. Two layouts are supported:

```
rs/src/actions/user_login.rs        →  POST /__forte_action/user_login
rs/src/actions/products_list.rs     →  POST /__forte_action/products_list
rs/src/actions/user_login/mod.rs    →  POST /__forte_action/user_login  (directory module)
```

The codegen discovers actions by scanning the top-level `src/actions/` directory (non-recursive) for:
- `.rs` files with `struct Input`, an `Output` type, and `pub async fn handler`
- subdirectories containing a `mod.rs` with the same structure

## Rust Handler

```rust
// rs/src/actions/user_login.rs
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Input {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub enum Output {
    Ok { token: String },
    InvalidCredentials,
    Error { message: String },
}

pub async fn handler(req: forte_sdk::ForteRequest<'_, Input>) -> Output {
    let input = &req.body;
    // validate credentials ...
    Output::Ok {
        token: "...".to_string(),
    }
}
```

Key conventions:
- `Input` — the deserialized request body (JSON); **must be a struct named exactly `Input`** (not an enum)
- `Output` — the serialized response body (JSON); **must be named exactly `Output`** (struct or enum)
- `handler` — must be `pub async fn`, takes `ForteRequest<'_, Input>`; **must be named exactly `handler`**
- Return type: `Output` directly (not `Result<Output>`). The codegen calls `forte_json::to_vec(&output)` on the return value, so it must implement `Serialize`. `anyhow::Error` does not implement `Serialize`, so `anyhow::Result<Output>` will not compile.
- If `Input` deserialization fails (malformed or missing required fields), the runtime returns HTTP 400 "invalid request body: …" before the handler is called.
- Action input is buffered through the SDK's default 1 MiB convenience limit. A larger JSON request returns HTTP 413; this buffering limit is separate from the 100 MB HTTP transport limit.

`forte add action <name>` generates a backend file with the correct names and signatures ready to compile.

## TypeScript Client

`forte add action` creates only the Rust backend file. On the next `forte build` or `forte dev`, `forte-rs-to-ts` generates a fully-typed Zod client at `fe/src/actions/.generated/<name>.ts`. Import it directly:

```ts
import { submit } from "../../actions/.generated/submit";

const result = await submit({ message: "hello" });
```

The generated file exports a named function in camelCase (`userLogin` for `user_login`, `submit` for `submit`) that calls the action and validates the response with Zod. Import it directly — do not hand-write a TypeScript wrapper.

If you need to call an action without the generated client (for example, before running `forte build`), use `callAction` from `@forte/react` directly. See [frontend.md](frontend.md#callaction) for details.

## Accessing Cookies and Headers

Actions receive the full `ForteRequest` context, so you can read and write cookies:

```rust
use forte_sdk::cookie_sign::{sign_cookie, unsign_cookie};

pub async fn handler(req: forte_sdk::ForteRequest<'_, Input>) -> Output {
    // read
    let session: Option<Session> = unsign_cookie(req.jar, "session");

    // write
    sign_cookie(req.jar, "session", &session, None);

    // read headers
    let auth = req.headers.get("authorization");

    Output::Ok { ... }
}
```

Cookie changes made in `req.jar` are written back to `Set-Cookie` response headers automatically.

## Hooks

Hooks are server-side data-fetching handlers. They live under `rs/src/hooks/` and follow the same `Input` / `Output` / `pub async fn handler` signature as actions. Unlike actions (which are called by user-triggered browser fetches), hooks are called during SSR: the JS SSR runtime posts to `/__self_invoke/<name>` to fetch data needed to render a page, and the results are embedded in the HTML and rehydrated on the client.

### Rust handler

```rust
// rs/src/hooks/user_session.rs
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Input {
    pub session_token: String,
}

#[derive(Serialize)]
pub enum Output {
    Ok { user_id: String, name: String },
    Unauthorized,
}

pub async fn handler(req: forte_sdk::ForteRequest<'_, Input>) -> Output {
    // validate session_token ...
    Output::Ok {
        user_id: "42".to_string(),
        name: "Alice".to_string(),
    }
}
```

Key rules — same as actions:
- `Input` must be a struct named exactly `Input`
- `Output` must be a struct or enum named exactly `Output`
- `handler` must be `pub async fn`, takes `ForteRequest<'_, Input>`
- Codegen scans `src/hooks/` non-recursively; only flat `.rs` files (no subdirectory modules)

> **There is no `forte add hook` command.** Create hook files manually. There is also no need to add a `mod` declaration in `lib.rs` — codegen adds it automatically via the generated `route_generated.rs`.

### TypeScript client

On the next `forte build` or `forte dev`, `forte-rs-to-ts` generates `fe/src/hooks/.generated/use<Name>.ts` with a React Suspense hook. The file name is `use` followed by the hook name with its first character uppercased — the rest of the name is kept as-is (underscores are not converted). For `user_session.rs` the generated file is `useUser_session.ts`:

```ts
// fe/src/hooks/.generated/useUser_session.ts (auto-generated)
export function useUser_session(input: Input): Output {
  return useForteHook("user_session", input, OutputSchema);
}
```

Use it in a React component (requires a `<Suspense>` boundary):

```tsx
import { useUser_session } from "../hooks/.generated/useUser_session";

function ProfileCard() {
  const session = useUser_session({ sessionToken: getCookie("session") });
  if (session.t === "Unauthorized") return null;
  return <div>{session.name}</div>;
}
```

During SSR the hook calls the WASM component server-side and embeds the result in `__FORTE_HOOK_CACHE__` in the HTML. On the client the cached value is read synchronously — no additional network request unless there is a cache miss.

Hook inputs follow the same camelCase→snake_case conversion as actions (`sessionToken` in TypeScript → `session_token` in Rust).

### Invalidating hook cache

After a mutation (e.g., a successful action), call `clearHookCache` from `@forte/react` to evict cached results so the next render re-fetches from the server:

```tsx
import { clearHookCache } from "@forte/react";

async function handleSubmit() {
  await submitAction({ message: "hello" });
  clearHookCache("user_session"); // evict one hook by name
  // clearHookCache();            // evict all hooks
  // re-render to trigger Suspense re-fetch
}
```

`clearHookCache(hookName?)` removes entries whose cache key starts with `hookName`. Call it with no argument to evict all cached hooks.

## Deserialization: `forte_json` vs `serde_json`

Action and hook inputs are deserialized with `forte_json`, which converts camelCase keys to snake_case Rust field names. A TypeScript caller sending `{"userName": "alice"}` maps to a Rust field `pub user_name: String`.

`forte_json` **serialization** (for action and hook outputs) also differs from `serde_json`: `Option::None` struct fields are omitted from the JSON output entirely rather than serialized as `null`. The generated TypeScript types reflect this — optional Rust fields become optional TypeScript properties (`fieldName?: T`) rather than nullable ones (`fieldName: T | null`).

Queue task and admin task inputs are deserialized with standard `serde_json` (no key conversion). Their struct field names must match the JSON keys exactly as sent by the caller — typically the generated `enqueue::*` functions (for queue tasks) or the `--input` JSON provided to `forte admin run` (for admin tasks). Use snake_case field names to match the default serde naming.

## Queue Tasks

Background tasks live under `rs/src/queue_task/`. They have an `Input` type and a `pub async fn handle` (not `handler`) function. `Input` can be a struct (when the task takes arguments) or a unit type (for tasks that take no input, such as cron jobs). For cron tasks the validator accepts `pub struct Input;`, `pub struct Input {}`, `pub struct Input()`, or `pub type Input = ();`:

```rust
// rs/src/queue_task/send_email.rs
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]  // both required: Deserialize for execution, Serialize for enqueue
pub struct Input {
    pub to: String,
    pub subject: String,
    pub body: String,
}

pub async fn handle(input: Input) -> Result<()> {
    // send email ...
    Ok(())
}
```

To enqueue a task from anywhere in the backend:

```rust
use crate::enqueue;
use crate::queue_task::send_email;

enqueue::send_email(send_email::Input {
    to: "user@example.com".to_string(),
    subject: "Welcome".to_string(),
    body: "...".to_string(),
}).await?;
```

The `enqueue` module is generated automatically when queue tasks exist. It requires the `FN0_QUEUE_URL` environment variable to be set.

## Admin Tasks

Admin tasks live under `rs/src/admin/`. Same `Input` / `handle` convention as queue tasks. They are called via `forte admin run <name>` and require the `x-fn0-admin: true` header (added automatically by the CLI). Unauthenticated requests get HTTP 401.

```rust
// rs/src/admin/seed_database.rs
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Input {
    pub count: u32,
}

#[derive(Serialize)]
pub struct Output {
    pub created: u32,
}

pub async fn handle(input: Input) -> Result<Output> {
    // seed logic ...
    Ok(Output { created: input.count })
}
```

Run it against the deployed app:

```sh
forte admin run seed_database --input '{"count": 10}'
# or read input from a file:
forte admin run seed_database --input-file seed.json
```

Run it against a locally running `forte dev` server:

```sh
forte admin run-local seed_database --input '{"count": 10}'
forte admin run-local seed_database -P 8080 --input '{"count": 10}'  # custom port
```

`forte admin run-local` sends the request to `http://localhost:<port>` (default 3000) instead of the deployed app. Use it to test admin tasks without a deploy.

See [`forte admin run`](cli.md#forte-admin-run-task-options) and [`forte admin run-local`](cli.md#forte-admin-run-local-task-options) in the CLI reference for all flags (`--input`, `--input-file`, `--timeout-seconds`).

**Serialization note:** Admin task input is deserialized with standard `serde_json` (no camelCase→snake_case conversion). The `Output` is also serialized with standard `serde_json` — field names appear as snake_case in the JSON output printed to the terminal. This differs from actions and hooks, whose output goes through `forte_json` (camelCase field names).
