# Forte CLI Reference

The `forte` CLI is the primary developer tool for creating, running, building, and deploying Forte projects.

## Commands

### `forte init <name>`

Scaffold a new project in a new directory named `<name>`.

Creates:
- `Forte.toml`, `.gitignore`
- `rs/` — Rust backend with an index page handler (`rs/Cargo.toml` is a standalone crate, not a workspace)
- `fe/` — React/TypeScript frontend with Vite
- Runs `npm install` for frontend dependencies

| Flag | Description |
|---|---|
| `--dev` | Use `path = "..."` deps pointing at the local fn0 monorepo instead of crates.io versions. For in-monorepo development only. |

```sh
forte init my-app
cd my-app
forte init --dev my-app   # monorepo development
```

---

### `forte dev [options]`

Start the development server with hot reload.

| Flag | Default | Description |
|---|---|---|
| `-P, --port <port>` | auto (from 3000) | Port to listen on |
| `-p, --project <dir>` | `.` | Project directory |

Behavior:
- Downloads and starts a local sqld (libSQL) server automatically; data persists in `.forte/data/`
- Starts a Vite dev server for the frontend (HMR)
- Rebuilds the Rust backend on `.rs` file changes
- Hot-reloads `env.yaml` / `env.local.yaml` on change — updated variable values take effect on the next request without a Rust rebuild (adding a new variable still requires `cargo build`, since `generate_env()` emits its accessor at build time)
- Handles SSR requests
- Routes queue task messages locally (loopback; no external queue needed)
- Serves object storage from `.forte/data/objects/` (no cloud credentials needed)
- Fires cron jobs from `cron.yaml` at the appropriate minute boundaries

```sh
forte dev
forte dev --port 8080
```

---

### `forte build [options]`

Build the project for production without deploying.

| Flag | Default | Description |
|---|---|---|
| `-p, --project <dir>` | `.` | Project directory |

Build steps:
1. **Codegen** — runs `forte-rs-to-ts` to generate `.props.ts` files, then generates frontend route file (`routes.generated.ts`)
2. **Backend** — `cargo build --release --target wasm32-wasip2` inside `rs/`
3. **Frontend** — `npx vite build --config <config>` (client) and `npx vite build --ssr <entry> --config <config>` (SSR)
4. **Dist** — copies `backend.wasm` and `server.js` to `dist/`

Output files:
- `dist/backend.wasm`
- `dist/server.js`

```sh
forte build
```

---

### `forte deploy [options]`

Build and upload the project to fn0 Cloud.

| Flag | Default | Description |
|---|---|---|
| `-p, --project <dir>` | `.` | Project directory |

**Never prompts.** A project that has not been through `forte cloud init` is
refused rather than set up here, so this behaves the same in CI as it does on
a terminal.

If a `cron.yaml` file exists in the project root, its scheduled jobs are registered during deploy. See [Cron Jobs](#cron-jobs) below.

Deploy steps (in addition to `forte build`):
1. **Checks the project is set up** — `Forte.toml` must name a `project_id` with a Cloudflare account behind it, and if it declares a `domain`, that must be the domain the project actually answers on. Otherwise the deploy stops and points at `forte cloud init`.
2. **Uploads static assets** — `fe/dist/` (client JS/CSS/assets) is uploaded to the project's own frontend-asset bucket, served from `https://fn0-<project_id>-frontend-asset.<zone>/<code_version>/`. The `VITE_PUBLIC_URL` env var is set to this URL during the Vite client build so asset references resolve correctly.
3. **Uploads backend bundle** — packages `dist/backend.wasm`, `dist/server.js`, and `env.yaml` into `dist/bundle.raw.tar` and uploads it to the fn0 Cloud control plane.
4. **Compiles to native** — the control plane invokes the `fn0-cwasm-compiler` Lambda to ahead-of-time compile `backend.wasm` to a Wasmtime-native `.cwasm` bundle. The CLI polls `deploy_status` until compilation finishes (one compilation per active Wasmtime version). Workers load the pre-compiled bundle on the next request, so there is no JIT cost at runtime.
5. **Registers cron jobs** — if `cron.yaml` exists, schedules are synced.

```sh
forte deploy
```

The app URL is printed when the deploy finishes.

---

### `forte destroy`

Delete the deployed project and all of its resources: routing, custom domain, deployed bundles, static assets, object storage, and its database.

| Flag | Default | Description |
|---|---|---|
| `--yes` | off | Skip the interactive confirmation |
| `--delete-buckets` | off | Also delete the project's now-empty R2 buckets, not just their contents |

Runs against the project in the current directory only — the `project_id` is read from `Forte.toml`, and there is no flag to target another directory or another project id. Without `--yes`, you must type the project id to confirm.

fn0's control plane removes the routing, bundles, assets, and database and empties the three R2 buckets. The command then goes through the setup broker to remove the project's Cloudflare footprint control cannot reach: the app hostname's DNS record, the two public buckets' custom domains, the origin certificate, and the `fn0 worker/frontend assets/cache purge (<project_id>)` tokens. The app DNS record is removed only when it is still the single proxied CNAME pointing at fn0's origin; a record you have edited is left in place and reported.

The three buckets are left standing (empty) unless `--delete-buckets` is passed — deleting a bucket needs a token minted from the setup token, so only the broker can do it, and only once control has emptied it. A bucket that is not empty yet is reported; re-running is safe.

On success the `project_id`, `project_name`, `zone`, and `domain` keys are removed from `Forte.toml` (all other keys and formatting are preserved), so the next `forte cloud init` registers a new project. The fn0-side teardown is enqueued on the control plane and runs asynchronously. If broker cleanup fails, the command returns the error and leaves `Forte.toml` unchanged so the cleanup can be retried.

```sh
forte destroy
forte destroy --yes
forte destroy --yes --delete-buckets
```

---

### `forte open [options]`

Print the deployed app's URL and open it in the default browser.

| Flag | Default | Description |
|---|---|---|
| `-p, --project <dir>` | `.` | Project directory |
| `--print` | off | Print the URL only, do not open a browser |

There is no default `fn0.dev` subdomain. A project answers only on the custom domain its owner attaches, so `forte open` prints that URL once the domain is attached and its origin certificate is held. Until then, it prints the reason instead.

```sh
forte open
forte open --print
```

Requires `project_id` in `Forte.toml`, which `forte cloud init` writes.

---

### `forte purge <key>... [options]`

Invalidate the CDN edge copy of one or more public objects. Use this after an out-of-band write (e.g. a presigned PUT upload) that bypasses your app's `public::put` call — `public::put` triggers its own invalidation automatically, so an explicit purge is only needed for presigned uploads.

| Flag | Default | Description |
|---|---|---|
| `-p, --project <dir>` | `.` | Project directory |

`<key>...` is one or more keys inside the project's public object namespace (the same strings you pass to `object_storage::public::put`), e.g. `clips/intro.mp4`.

```sh
forte purge clips/intro.mp4
forte purge avatars/1.png avatars/2.png
```

Prints each queued CDN invalidation URL and a count of invalidations submitted. Purges are subject to the project's hourly quota (1 000 per hour on the one-dollar plan); see [limits.md](../fn0/limits.md) for details.

See [object-storage/overview.md](../object-storage/overview.md#presigned-uploads) for the full presigned-upload pattern where this is needed.

---

### `forte purge-page <path>... [options]`

Invalidate the CDN edge copy of one or more `#[cache_static]` pages without deploying. Use this when page content changes because data was written at runtime (e.g. publishing a record) rather than because code changed.

| Flag | Default | Description |
|---|---|---|
| `-p, --project <dir>` | `.` | Project directory |

`<path>...` is one or more route paths as a visitor requests them (e.g. `/episode/1`). Leading `/` is required; no query string, fragment, backslash, or dot segment.

```sh
forte purge-page /episode/1
forte purge-page /episode/1 /episodes
```

Prints each queued invalidation path and a count of invalidations submitted. Subject to the same hourly purge quota as `forte purge`.

See [forte/pages.md](pages.md#invalidating-a-page-without-deploying) for when to use this and for the programmatic API (`static_page_cache::purge`).

---

### `forte add page <path>`

Add a new page (Rust handler + React component).

The `path` argument supports dynamic segments using `[param]` syntax:

```sh
forte add page about
forte add page product/[id]
forte add page blog/[year]/[slug]
```

Creates:
- `rs/src/pages/<path>/mod.rs` — Rust handler
- `fe/src/pages/<path>/page.tsx` — React component

---

### `forte add action <path>`

Add a new server action (Rust handler only).

```sh
forte add action user_login
forte add action products_list
```

Creates:
- `rs/src/actions/<path>.rs` — Rust action handler with correct `Input` / `Output` / `handler` names

The TypeScript client is generated automatically by `forte-rs-to-ts` on the next `forte build` or `forte dev`. Import from `fe/src/actions/.generated/<name>.ts`.

> **Important:** Use underscores, not slashes, in action paths. `forte add action user/login` creates `rs/src/actions/user/login.rs`, but codegen only discovers handlers at `actions/<name>.rs` (flat file) or `actions/<name>/mod.rs` (directory module) — nested files like `actions/user/login.rs` are never discovered. Use `forte add action user_login` instead (or create `actions/user_login/mod.rs` manually if you need to split the file across multiple modules).

Dashes in the path are automatically converted to underscores: `forte add action user-login` creates `rs/src/actions/user_login.rs`.

---

### Adding hooks, queue tasks, and admin tasks

There are no `forte add hook`, `forte add queue-task`, or `forte add admin` commands. Create these files manually:

- Hooks: `rs/src/hooks/<name>.rs` — follow the `Input` / `Output` / `pub async fn handler` pattern (see [actions.md](actions.md#hooks))
- Queue tasks: `rs/src/queue_task/<name>.rs` — follow the `Input` / `pub async fn handle` pattern
- Admin tasks: `rs/src/admin/<name>.rs` — follow the `Input` / `pub async fn handle` pattern

Codegen picks them up automatically on the next build; no `mod` declaration needed in `lib.rs`.

---

### `forte login`

Authenticate with fn0 Cloud using a PKCE OAuth flow and saves credentials locally (shared with the `fn0` CLI).

| Flag | Default | Description |
|---|---|---|
| `--token <token>` | — | Provide token directly (skips interactive flow) |

Default interactive flow:
1. A loopback TCP listener is started on a random port.
2. A PKCE authorization URL is printed and the browser is opened automatically (falls back to manual URL if auto-open fails).
3. After you approve in the browser, the callback redirects to `http://127.0.0.1:<port>/callback`.
4. The CLI exchanges the authorization code for a token and saves it locally.

With `--token`, the interactive flow is skipped — the token is validated (must start with `fn0_`) and saved directly. Credentials are saved to a local file (path printed on success).

```sh
forte login
forte login --token fn0_xxxxx
```

---

### `forte cloud init`

Give a project an identity and a Cloudflare zone. The public hostname is
derived from the project name and zone, and this setup must complete before the
project can be deployed. See [Bring Your Own Cloudflare](../fn0/cloudflare.md)
for what it creates and which token permissions it needs. The normative command
contract is [the cloud init specification](../../forte/cli/CLOUD_INIT_SPEC.md).

**Prerequisites:** `forte login` must be run before `forte cloud init` — the command loads fn0 credentials and fails with "not signed in" if they are missing.

```sh
forte cloud init \
  --project . \
  --project-name my-app \
  --zone example.com
```

On the first run for a Cloudflare account, the command asks for a setup token
through a masked prompt — never a command-line argument, an environment
variable, or something it prints or saves anywhere in the clear. That first
run uses the token to install a small broker Worker in your own Cloudflare
account and stores the token only inside that Worker's Secrets Store. Every
later run, for this project or any other project on the same account, reuses
the broker and does not ask for the token again.

`--setup-token-from-clipboard` replaces that masked prompt with a clipboard
hand-off: the command waits while you (or an AI agent driving your browser)
create the token in the Cloudflare dashboard and click **Copy**, then reads it
from the clipboard, verifies it, and wipes the clipboard. During bootstrap
`forte` rolls the token's secret in place, so the string that was copied stops
working while the setup token itself keeps its name and permission. Needs a
desktop session (it cannot reach a clipboard over SSH or in CI). See
[AI-assisted setup](../fn0/cloudflare.md#ai-assisted-setup).

`--zone` is a Cloudflare zone name such as `example.com`, not the internal
hexadecimal zone ID. The CLI resolves the exact zone and must not choose one
implicitly when several zones are accessible.

`--project-name` must be a single DNS hostname label: lowercase ASCII letters,
digits, and hyphens only; 1–63 characters; and no leading or trailing hyphen.
The CLI rejects invalid values without normalizing them.

The app hostname is derived automatically:

```text
<project-name>.<zone>
```

For example, `my-app` in `example.com` becomes `my-app.example.com`. There is
no separate `--domain` argument in the default contract.

What it does, in order:

1. Validates all arguments; installs the broker Worker first if this
   Cloudflare account does not have one yet
2. Resolves the requested zone and derives the app hostname
3. Registers the project and writes `project_id`, `project_name`, `zone`,
   `domain`, `cloudflare_account_id`, and `cloudflare_broker_url` to
   `Forte.toml`
4. Creates or reuses the buckets, CDN hostnames, and cache rule on your
   account, through the broker
5. Hands fn0 the narrow project credentials the broker minted
6. Has the broker sign an origin certificate through your Origin CA — fn0
   holds no token that can sign one — and registers the derived domain
7. Writes the proxied `CNAME` for the derived domain into your zone

The setup token is never sent to fn0. After the first run it is not sent
anywhere at all — the broker Worker in your own account is the only thing
that ever reads it again, to mint the short-lived, narrowly-scoped
credentials each of these steps actually needs.

Re-running setup for an existing project checks the stored project name,
zone, and derived domain; it refuses mismatches rather than entering a
prompt-driven reconfiguration flow. Moving a project to a different
Cloudflare account is not supported.

#### `forte cloud rotate`

```sh
forte cloud rotate --project .
```

Replaces the setup token stored in the broker's Secrets Store. Asks for the
new token through the same masked prompt as `init` (or reads it from the
clipboard with `--setup-token-from-clipboard`), then has the broker save it
and revoke the old one. Every project already connected through this broker
keeps working — this only changes which token the broker itself holds.

#### `forte cloud clear`

```sh
forte cloud clear --project . --yes
```

Deletes the setup token secret from the broker's Secrets Store and revokes
it. The broker Worker and its Secrets Store are left in place, empty — this
is for retiring a compromised or unwanted token, not for undoing `cloud
init`. Without `--yes` this asks for confirmation first.

#### `forte cloud destroy`

```sh
forte cloud destroy --project . --yes
```

Deletes the broker Worker, its Secrets Store, and the setup token stored in
it — the full undo of a `cloud init` bootstrap for this Cloudflare account.
This is account-scoped: a broker is meant to be shared by every project on
the account that has run `cloud init`, and this command does not check
whether another project still depends on it before tearing it down. Without
`--yes` it asks you to type the Cloudflare account ID to confirm.

This does not touch a project's own resources (buckets, DNS record, minted
credentials) — that's `forte destroy`, run per project.

---

### `forte env <subcommand>`

Manage `env.yaml` entries for the project. Entries are bundled into the deploy and exposed as environment variables at runtime. Secret entries are encrypted via fn0 Cloud's vault and decrypted in-worker.

| Subcommand | Description |
|---|---|
| `set <key> <value> [--secret]` | Set an env entry. Plain by default; `--secret` encrypts the value via the control vault (requires `forte login`). Silently overwrites an existing key. |
| `migrate` | Convert a legacy `.env` file into `env.local.yaml`. Refuses to run if `env.local.yaml` already exists, and leaves `.env` on disk for you to delete. |

| Flag | Default | Description |
|---|---|---|
| `--secret` | `false` | Encrypt the value as a secret entry |
| `-p, --project <dir>` | `.` | Project directory |

```sh
forte env set PUBLIC_API_URL https://api.example.com
forte env set DATABASE_PASSWORD hunter2 --secret
forte env migrate    # convert legacy .env to env.local.yaml
```

---

### `forte admin run <task> [options]`

Run an admin task against the deployed app.

| Flag | Default | Description |
|---|---|---|
| `task` | — | Task name (matches `rs/src/admin/<name>.rs`) |
| `-p, --project <dir>` | `.` | Project directory |
| `--input-file <file>` | — | Read input JSON from file |
| `--input <json>` | — | Input JSON as string |
| `--timeout-seconds <n>` | 300 | Timeout |

```sh
forte admin run seed-database --input '{"count": 100}'
```

### `forte admin run-local <task> [options]`

Same as `run` but targets a locally-running `forte dev` server.

| Flag | Default | Description |
|---|---|---|
| `task` | — | Task name (matches `rs/src/admin/<name>.rs`) |
| `-P, --port <port>` | 3000 | Local dev server port |
| `--input-file <file>` | — | Read input JSON from file |
| `--input <json>` | — | Input JSON as string |
| `--timeout-seconds <n>` | 300 | Timeout |

### `forte db query <sql> [options]`

Run one SQL statement against the deployed project's document database, for
one-off inspection or fixes. The query runs through the fn0 control plane —
you never hold database credentials — and the output ends with the engine's
row read/write counts for the statement.

| Flag | Default | Description |
|---|---|---|
| `sql` | — | The SQL statement; use `?` placeholders for arguments |
| `--arg <value>` | — | Bind one `?` placeholder (repeatable). Parsed as JSON (`42`, `1.5`, `null`, `"text"`); anything that is not valid JSON is bound as a plain string |
| `-p, --project <dir>` | `.` | Project directory |
| `--json` | off | Print results as JSON instead of a table |
| `--timeout-seconds <n>` | 300 | Timeout |

```sh
forte db query 'SELECT pk, sk FROM docs LIMIT 10'
forte db query 'SELECT data FROM docs WHERE pk = ?' --arg 'UserDoc/github_id=09223372036858010238'
```

### `forte db exec <file> [options]`

Run every statement of a SQL file — a migration — as **one transaction**:
either every statement commits or none does. A failing statement rolls the
whole file back and reports which statement failed.

| Flag | Default | Description |
|---|---|---|
| `file` | — | SQL file; statements separated by `;` |
| `-p, --project <dir>` | `.` | Project directory |
| `--json` | off | Print results as JSON instead of a table |
| `--timeout-seconds <n>` | 300 | Timeout |

```sh
forte db exec migrations/2026-08-18-backfill.sql
```

Rules for writing directly to `docs`:

- **Bump `version` on every `UPDATE` of `data`** — `SET data = ..., version =
  version + 1`. The transaction layer uses `version` for conflict detection;
  an update that leaves it unchanged can be silently overwritten by a
  concurrently running app transaction.
- A `SELECT` without `LIMIT` can read every row of a partition; the row read
  count you see in the output is what a quota would charge for.

---

## Secrets

Use `forte env set <key> <value> --secret` to encrypt a value and write it to `env.yaml`. To list or remove entries, use the `fn0` CLI from the project directory:

```sh
fn0 env list
fn0 env unset COOKIE_SECRET
```

Secrets are encrypted via fn0 Cloud's vault and decrypted in-worker at runtime. Decryption needs the vault, so `forte dev` cannot open them; give any secret you need locally a plaintext value in `env.local.yaml`.


### env.yaml format

`forte env set` manages `env.yaml` for you. The file is a YAML mapping of key → value, where plain values are strings and secret values are a nested mapping with a `secret` key:

```yaml
# env.yaml — managed by `forte env set`
PUBLIC_API_URL: https://api.example.com
STRIPE_SECRET_KEY:
  secret: <ciphertext written by forte env set --secret>
```

Do not edit the `__dek` key (auto-managed data-encryption-key entry). Plain values are available as environment variables in the deployed app; secret values are decrypted at runtime by the worker.


See [forte env](#forte-env-subcommand) above and [fn0 CLI Reference](../fn0/overview.md#fn0-cli-commands) for full `fn0 env` documentation.

---

## Cron Jobs

Place a `cron.yaml` file in the project root to schedule queue tasks. The file is read during `forte deploy` and the jobs are registered with fn0 Cloud.

```yaml
# cron.yaml
- function: send_digest_email
  every_minutes: 60
- function: cleanup_old_sessions
  every_minutes: 1440
```

Each entry:
- `function` — must match a file in `rs/src/queue_task/<name>.rs`, and that task's `Input` must be empty — either a unit struct (`pub struct Input;`, `pub struct Input {}`, or `pub struct Input()`) or the type alias `pub type Input = ();`.
- `every_minutes` — run interval (must be ≥ 1).

Cron jobs run locally during `forte dev`: the CLI ticks at each minute boundary, reads `cron.yaml`, and enqueues matching tasks through the loopback queue.

---

## Local Tool Cache

`forte dev` and `forte build` download external tools on first use and cache them in `~/.forte/bin/`. You do not need to manage these manually; downloads happen automatically.

| Tool | Cache path | Source |
|---|---|---|
| `sqld` (libSQL server) | `~/.forte/bin/sqld-<version>` | tursodatabase/libsql GitHub Releases |
| `forte-rs-to-ts` | `~/.forte/bin/forte-rs-to-ts-<version>/forte-rs-to-ts` | NamseEnt/fn0 GitHub Releases |

To force a fresh download (e.g., after a corrupted download), delete the relevant entry:

```sh
rm -rf ~/.forte/bin/sqld-*
rm -rf ~/.forte/bin/forte-rs-to-ts-*
```

The next `forte dev` or `forte build` will re-download the tools.
