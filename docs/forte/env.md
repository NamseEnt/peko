# Environment Variables

Forte projects expose environment variables through two YAML files at the project root and a generated Rust accessor module. This document covers the file formats, CLI commands, local development workflow, and how to consume variables at runtime.

## Files at a glance

| File | Committed | Purpose |
|---|---|---|
| `env.yaml` | Yes | Shared variables bundled into every deploy; secrets are encrypted |
| `env.local.yaml` | No (gitignored) | Local plaintext overrides for dev; never deployed |

`forte dev` resolves the environment as plain `env.yaml` entries overlaid with `env.local.yaml`. Only keys in `env.yaml` get a generated Rust accessor; `env.local.yaml` only supplies values, never new keys.

## env.yaml

`forte env set` manages this file. The format is a YAML mapping of key → value, where secrets are a nested `secret:` mapping:

```yaml
# env.yaml
PUBLIC_API_URL: https://api.example.com
STRIPE_SECRET_KEY:
  secret: <ciphertext written by forte env set --secret>
```

Do not edit the `__dek` key — it is an auto-managed data-encryption key used by the vault.

Plain values are bundled as-is and exposed as environment variables in the deployed app. Secret values are encrypted via fn0 Cloud's vault and decrypted in-worker at runtime; the secret key is never stored in plaintext anywhere in the file.

## env.local.yaml

Same YAML shape as `env.yaml`, but plaintext only — `secret:` entries are rejected. `forte init` gitignores it automatically.

Use it to supply plaintext values locally for variables that are encrypted in `env.yaml`:

```yaml
# env.local.yaml
STRIPE_SECRET_KEY: sk_test_...
COOKIE_SECRET: any-32-byte-local-secret
```

Encrypted `env.yaml` entries cannot be decrypted offline (the vault requires credentials and network), so `forte dev` leaves them unset and prints which ones are missing on startup. Give any secret you need locally a value here.

## Managing entries

```sh
# Add or overwrite a plain entry
forte env set PUBLIC_API_URL https://api.example.com

# Add an encrypted secret (requires forte login)
forte env set STRIPE_SECRET_KEY sk_live_... --secret

# List all entries (shows kind: plain or secret, not the values)
fn0 env list

# Remove an entry
fn0 env unset STRIPE_SECRET_KEY
```

`forte env` implements `set` and `migrate`. List and unset are provided by the `fn0` CLI (`cargo binstall fn0-cli`). Both CLIs operate on the same `env.yaml` file.

### Migrating a legacy `.env` file

Projects that previously used a `.env` file can convert it to `env.local.yaml`:

```sh
forte env migrate
```

Reads `.env` from the project root, writes each `KEY=value` entry into `env.local.yaml`, and prints the count of migrated entries. Refuses to run if `env.local.yaml` already exists — merge manually in that case. Leaves `.env` on disk for you to delete after verifying the output.

## Generated Rust accessors

`forte_codegen::generate_env()` (called from `build.rs` by default) reads `env.yaml` and `env.local.yaml` and writes `rs/src/env_generated.rs` with a zero-cost typed accessor per key:

```rust
// rs/src/env_generated.rs (do not edit)
pub fn cookie_secret() -> &'static str { ... }
pub fn stripe_secret_key() -> &'static str { ... }
pub fn public_api_url() -> &'static str { ... }
```

Key → function name conversion: `COOKIE_SECRET` → `cookie_secret`. Values are loaded from the real environment at first use via `std::sync::LazyLock`. Each accessor panics if the variable is unset at runtime.

To use the module, add `mod env_generated;` to `rs/src/lib.rs` **outside** the FORTE-MANAGED block:

```rust
// rs/src/lib.rs
mod env_generated;  // add this outside the managed block

// === FORTE-MANAGED START ===
// ...
// === FORTE-MANAGED END ===
```

Then call the accessor anywhere:

```rust
use crate::env_generated;

let secret = env_generated::cookie_secret();
```

Adding a new variable to `env.yaml` requires a `cargo build` (so the new accessor is generated). Changing the value of an existing variable does not — `forte dev` hot-reloads `env.yaml` / `env.local.yaml` on change and the updated value takes effect on the next request.

## Platform-injected variables

fn0 injects these automatically in both `forte dev` and deployed apps. You do not declare them in `env.yaml` — they are already present in the runtime environment.

| Variable | Injected when |
|---|---|
| `TURSO_URL` | Always (database endpoint) |
| `TURSO_AUTH_TOKEN` | Always (database auth) |
| `FN0_QUEUE_URL` | Always (queue endpoint; always present even if queue tasks are not used) |
| `FN0_OBJECT_STORAGE_URL` | Always (object storage endpoint) |

If you call `generate_env()` and declare these keys in `env.yaml`, `forte dev` will set them to the injected values automatically. You do not need entries in `env.yaml` for them; refer to the platform clients (`doc_db::turso()`, `object_storage::private::bucket()`, etc.) which read these variables internally.

## Variables you set yourself

| Variable | Required | Purpose |
|---|---|---|
| `COOKIE_SECRET` | Yes, if using `cookie_sign` | HMAC secret for signed cookies |
| `OTEL_SERVICE_NAME` | No | Service name in OpenTelemetry traces; defaults to `"forte-app"` |

Any other variable your application needs (API keys, public config, feature flags) follows the same pattern: `forte env set`, then access via `env_generated::<accessor>()` in Rust.

## Deploy

`forte deploy` bundles `env.yaml` (but not `env.local.yaml`) into `dist/bundle.raw.tar` and uploads it to fn0 Cloud. Secret values are decrypted in-worker at startup by the vault using the project's `__dek`. The bundle is what the worker reads — the fn0 control plane never sees plaintext secrets.
