# Testing Forte Backends

Backend tests in a Forte project are async Rust functions annotated with `#[forte_sdk::test]`. They do not run under libtest: `forte-test-runner` discovers them through the `fn0:test-harness/harness` export and runs each one in its own component instance.

## `#[forte_sdk::test]`

Use `#[forte_sdk::test]` for all async tests in backend crates. Do not use `#[tokio::test]` — it is not available in `wasm32-wasip2`.

```rust
#[forte_sdk::test]
async fn my_test() {
    // async test code
}
```

The test function must be `async fn` with no parameters. It may return `()` or a `Result`, and a returned `Err` fails the test just as a panic does.

## Wiring a test target

Every target holding these tests turns libtest off and invokes the harness once:

```toml
# Cargo.toml
[[test]]
name = "my_test"
harness = false

[target.'cfg(target_arch = "wasm32")'.dev-dependencies]
forte-sdk = { version = "0.9", features = ["test-harness"] }
```

```rust
// tests/my_test.rs
forte_sdk::test_main!();
```

`test_main!()` exports the harness and supplies the `main` that `harness = false` requires. For unit tests living in `src/`, set `harness = false` under `[lib]` and put `#[cfg(test)] forte_sdk::test_main!();` at the crate root.

### Why not libtest

libtest's `main` reaches wasm through the `wasm32-wasip2` target's **synchronous** `wasi:cli/run` export, and a synchronous task may not block on a host future. Any test that performs real I/O — an HTTP request, a database round trip — therefore traps on its first await with `cannot block a synchronous task before returning`, killing the whole run. Only tests that never yield to the host (in-memory backends) survive, which is why the limitation went unnoticed for so long. The harness export is `async`, which lifts that restriction.

Running each test in a fresh instance also contains failures: the guest is `panic = abort`, so under libtest one failing assertion aborts the process and the remaining tests never run.

## Testing with an In-Memory Database

`doc_db::memory()` returns an in-memory backend with the same `Database` API as the production Turso/libSQL backend. Each call returns a fresh, isolated instance — no setup, no cleanup, no external server.

```rust
use doc_db::DbRequest;

#[forte_sdk::test]
async fn test_user_roundtrip() {
    let db = doc_db::memory();

    UserPut(User {
        id: "alice".into(),
        version: 1,
        name: "Alice".into(),
        email: "alice@example.com".into(),
    })
    .send_with(&db)
    .await
    .unwrap();

    let user: Option<User> = UserGet { id: "alice".into(), version: 1 }
        .send_with(&db)
        .await
        .unwrap();

    assert_eq!(user.unwrap().name, "Alice");
}
```

Two `doc_db::memory()` calls return independent databases that never share state.

### Simulating database errors

The mock API on a `memory()` database lets you force specific return values or errors. Mocks are consumed one-at-a-time (FIFO) per `(op, pk, sk)` key; after the mock fires, subsequent calls hit the real in-memory backend.

```rust
#[forte_sdk::test]
async fn test_get_error_propagation() {
    let db = doc_db::memory();

    // Force the next get on this key to fail
    db.mock_get("User/id=alice", "version=0000000001")
        .returns_err("network timeout");

    let result = db.get("User/id=alice", "version=0000000001").await;
    assert!(result.is_err());

    // Next call hits the real backend (returns None — no data was written)
    let result = db.get("User/id=alice", "version=0000000001").await;
    assert!(result.unwrap().is_none());
}
```

See [doc-db/overview.md](../doc-db/overview.md#mocking-tests) for the full mock API (`mock_put`, `mock_delete`, `clear_mocks`, etc.).

> **Note on raw pk/sk keys:** The mock API takes raw string keys. When using the `#[forte_doc]` macro, keys follow the format `TypeName/pk_field=value` for the pk and `sk_field=value` for the sk. Integer fields are zero-padded (e.g. `version=0000000001` for a `u32` of 1). Prefer testing through the generated typed helpers (`UserGet`, `UserPut`, etc.) and only reach for the raw mock API when you need to simulate errors.

## Testing with In-Memory Object Storage

`object_storage::private::memory()` returns an in-process `PrivateBucket` backed by a `BTreeMap`. The API is identical to production; each call returns a fresh, isolated instance.

```rust
#[forte_sdk::test]
async fn test_file_roundtrip() {
    let bucket = object_storage::private::memory();

    bucket.put("avatars/alice.png", Some("image/png"), b"fake png data" as &[u8]).await.unwrap();

    let object = bucket.get("avatars/alice.png").await.unwrap().unwrap();
    assert_eq!(object.body.bytes().await.unwrap().as_ref(), b"fake png data");

    bucket.delete("avatars/alice.png").await.unwrap();
    assert!(bucket.get("avatars/alice.png").await.unwrap().is_none());
}
```

## Testing Action Handlers Directly

Action and hook handlers are plain `async fn`s. You can call them directly in tests by constructing a `ForteRequest` manually — there is no constructor, so initialize all fields directly.

```rust
use forte_sdk::{ForteRequest, CookieJar};
use forte_sdk::http::{Method, HeaderMap};

#[forte_sdk::test]
async fn test_login_ok() {
    let mut jar = CookieJar::new();
    let method = Method::POST;
    let headers = HeaderMap::new();

    let req = ForteRequest {
        uri_authority: "localhost",
        method: &method,
        headers: &headers,
        jar: &mut jar,
        raw_body: &[],
        body: crate::actions::user_login::Input {
            email: "alice@example.com".to_string(),
            password: "correct".to_string(),
        },
    };

    let output = crate::actions::user_login::handler(req).await;
    assert!(matches!(output, crate::actions::user_login::Output::Ok { .. }));
}
```

Combine with `doc_db::memory()` and `object_storage::private::memory()` to test handlers without any external services.

## Running Tests

### Inside a Forte project (`rs/`)

A Forte project's `rs/` compiles to `wasm32-wasip2` (set by `.cargo/config.toml`). To run tests with `cargo test`, add the WASM runner to `rs/.cargo/config.toml`:

```toml
[build]
target = "wasm32-wasip2"

[target.wasm32-wasip2]
runner = "forte-test-runner"
```

Then install the runner and run:

```sh
cargo install --path <monorepo>/forte/test-runner
cargo test
```

> **Note:** `forte init` does not add the runner automatically. Without it, `cargo test` will build the WASM binary but fail to execute it.

> **Note:** `forte-test-runner` is not published to crates.io and has no pre-built binaries. You must clone the fn0 monorepo and install from source with `cargo install --path forte/test-runner`. Reinstall whenever you pull monorepo changes that touch WIT bindings — an out-of-date runner fails at link time with an "unimplemented WASI import" error, not a version message.

### From the monorepo workspace root

```sh
cargo test                  # all workspace crates (native targets only)
cargo test -p fn0-doc-db    # doc-db only (requires runner + libSQL; see development.md)
cargo test -p forte-sdk     # forte-sdk only (native, no runner needed)
```

### doc-db integration tests

`doc-db/tests/integration_test.rs` connects to a live libSQL server — the `libsql-test` service in `docker-compose.yml`, on `127.0.0.1:18123`. Set `DOC_DB_TEST_URL` to point it elsewhere. See [development.md](../development.md#doc-db-integration-tests) for prerequisites (runner installation and starting the local DB).

### Supported test filters

The runner accepts the parts of libtest's command line that carry over, and ignores the rest rather than failing the run:

| Argument | Behaviour |
| --- | --- |
| `<filter>` | substring match on the test name |
| `--exact` | makes the filters exact matches |
| `--list` | prints the test names and exits |
| `--ignored` | runs nothing; there is no `#[ignore]` equivalent |
| `--nocapture` | accepted; output is never captured to begin with |

Tests run sequentially, and the guest's own panic message is printed above the failure line.
