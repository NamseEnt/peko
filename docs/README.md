# fn0 / Forte Documentation

## Getting Started

- [Setup](setup.md) — Prerequisites, installation, environment variables

## Forte Framework

Forte is the full-stack web framework built on fn0.

- [**Quick Reference**](forte/quick-reference.md) — Cheat sheet: all handler patterns, naming rules, common snippets
- [Overview](forte/overview.md) — Architecture and key packages
- [Project Structure](forte/project-structure.md) — Directory layout and conventions
- [Environment Variables](forte/env.md) — env.yaml, env.local.yaml, secrets, and generated Rust accessors
- [CLI Reference](forte/cli.md) — All `forte` commands (including login and cron jobs)
- [Pages](forte/pages.md) — Page handlers, path/search params, redirects, cookies
- [API Endpoints](forte/apis.md) — JSON API handlers under `/api/`, no SSR
- [WebSockets](forte/websockets.md) — Inbound routes, outbound routes, singleton connections, streaming sends, graceful disconnects
- [Actions & Tasks](forte/actions.md) — Server actions, hooks, queue tasks, admin tasks
- [Per-page Head](forte/head.md) — Per-page `<title>` and meta tags via `head` exports
- [Frontend Runtime](forte/frontend.md) — `@forte/react` API, `__FORTE_BASE_URL__`, SSR/hydration lifecycle
- [SDK Reference](forte/sdk.md) — `ForteRequest`, HTTP client, cookies, re-exported crates
- [Serialization](forte/serialization.md) — `forte_json` rules, enum encoding, TypeScript type mapping
- [Code Generation](forte/codegen.md) — How `forte-codegen` and `forte-rs-to-ts` work
- [Testing](forte/testing.md) — `#[forte_sdk::test]`, in-memory DB/storage, testing handlers
- [Troubleshooting](forte/troubleshooting.md) — Common build, dev, and deployment issues

## doc-db

- [Overview](doc-db/overview.md) — Document database API, transactions, `#[forte_doc]` macro

## object-storage

- [Overview](object-storage/overview.md) — Per-project S3-style object store, hooks-only credential model

## fn0 Platform

- [Overview](fn0/overview.md) — FaaS runtime, execution model, limits, cluster architecture, worker environment variables for self-hosting
- [Limits & Quotas](fn0/limits.md) — Per-request runtime limits and fn0 Cloud monthly quotas
- [Bring Your Own Cloudflare](fn0/cloudflare.md) — How fn0 uses your Cloudflare account for storage, CDN, and custom domains
- [Platform Deployment](fn0/deployment.md) — Deploying the fn0 control plane and worker nodes (maintainers only)

## Designs

- [Forte WebSockets](design/forte-websockets.md) — Host-owned connection and direct QUIC routing design
- [Forte Persistent Outbound WebSockets](design/forte-websocket-singleton-connectors.md) — Project-scoped singleton connections kept alive by fn0 without an application invocation
- [HTTP Body Streaming](design/http-body-streaming.md) — 100 MB streaming request and end-to-end response-body contract

## Development

- [Development Workflow](development.md) — Code style, testing, build targets, release process
