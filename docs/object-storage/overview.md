# object-storage

`object-storage` (`fn0-object-storage`) is a small S3-style object store for
Forte apps. It works in both WASI components (Forte backends) and native Rust
binaries.

Storage is split by access model into two namespaces, which are separate types
rather than two configurations of one:

| | Reachable by | Use for |
|---|---|---|
| `object_storage::private` | the app, or a presigned URL | anything not meant to be world-readable |
| `object_storage::public` | anyone with the URL, served from the CDN | assets embedded in HTML that outlives a signature |

Application code never sees the storage endpoint or credentials: each namespace
reads only its injected placeholder URL, and the worker's object-storage hijack
rewrites, routes, and signs every request out of band — the same hooks-only
model as [doc-db](../doc-db/overview.md).

## Connecting

```rust
use object_storage::private::PrivateBucket;

// Production / `forte dev`: reads the injected FN0_OBJECT_STORAGE_URL.
let bucket: PrivateBucket = object_storage::private::bucket();

// In-memory (tests).
let bucket: PrivateBucket = object_storage::private::memory();
```

`PrivateBucket` is `Clone`. Share it across your handler by cloning.

## Operations

### `put`

```rust
bucket.put("avatars/42.png", Some("image/png"), png_bytes).await?;

// Stored with no type at all:
bucket.put("scratch/blob", None, bytes).await?;
```

Pass a `content_type` for anything a browser will fetch through a presigned URL:
no app is in that path to correct a wrong guess, and R2 stores no type of its
own. Measured against a real bucket, an object uploaded without the header is
served back with **no `Content-Type` at all** — R2 does not substitute
`application/octet-stream`.

`None` therefore means the object genuinely has no stored type, which is what
lets an [`Object`](#get) that had none round-trip unchanged. `public::put` takes
a required `&str` instead, because a public object is by definition fetched from
the CDN by a browser.

`put` accepts anything that converts into `object_storage::Body` — `Vec<u8>`,
`&[u8]`, `Bytes`, `String`, `&str`. An existing object at the same key is
overwritten.

A body already being streamed also converts, which forwards it without ever
holding the object whole — see the `get` examples below. The length is then
unknown until the body ends, so the upload is sent chunked rather than with a
`Content-Length`.

### `get`

```rust
let object: Option<Object> = bucket.get("avatars/42.png").await?;
```

Returns `None` if the key does not exist. `Object` is the stored type plus the
body, still unread:

```rust
pub struct Object {
    pub content_type: Option<String>,
    pub body: Body,
}
```

`content_type` is `None` only for objects genuinely stored without one — by a
presigned upload, or by a `put` that passed `None`.

Call `bytes()` for the whole object:

```rust
let Some(object) = bucket.get("avatars/42.png").await? else {
    return Ok(not_found());
};
let data: Bytes = object.body.bytes().await?;
```

Or pass the body on without reading it. It is already what `put` accepts, and
inside a WASI component it converts into a `forte_sdk::http::Body` to become the
body of an outgoing request:

```rust
// Copy without holding the object in memory, keeping the stored type as it was.
let object = bucket.get("uploads/clip.mp4").await?.expect("exists");
bucket
    .put("archive/clip.mp4", object.content_type.as_deref(), object.body)
    .await?;

// Or forward it to another service.
let object = bucket.get("uploads/clip.mp4").await?.expect("exists");
let request = http::Request::post("https://transcode.example/jobs")
    .body(forte_sdk::http::Body::from(object.body))?;
let response = forte_sdk::http::Client::new().send(request).await?;
```

### `head`

```rust
let meta: Option<ObjectMetadata> = bucket.head("avatars/42.png").await?;
// ObjectMetadata { size, content_type, etag }
```

Fetches metadata without downloading the body.

### `delete`

```rust
bucket.delete("avatars/42.png").await?;
```

Succeeds whether or not the object existed.

### `list` — scan by key prefix

Lists objects whose key starts with `prefix`, in ascending key order, up to
`limit` entries. Pass `after` to resume after a key.

```rust
let page = bucket.list("avatars/", None, 100).await?;
for entry in &page.entries {
    println!("{} ({} bytes)", entry.key, entry.size);
}

// Next page, if the listing was truncated:
if let Some(cursor) = &page.next_cursor {
    let next = bucket.list("avatars/", Some(cursor), 100).await?;
}
```

`ObjectList { entries: Vec<ListEntry>, next_cursor: Option<String> }`.
`next_cursor` is `Some` only when the result was truncated; pass it as the
next call's `after`.

## Errors

Every operation returns `object_storage::Result<T>`. `object_storage::Error`
is a concrete enum: `Transport`, `UnexpectedStatus { status, message }`,
`Parse`.

## Presigned URLs

`presigned_get_url` / `presigned_put_url` return a time-limited URL that a
browser (or any HTTP client) can use to download from or upload to an object
directly, without routing through the app.

```rust
use std::time::Duration;

let download = bucket
    .presigned_get_url("avatars/42.png", Duration::from_secs(3600))
    .await?;
// hand `download` to a browser <img src> or fetch()

let upload = bucket
    .presigned_put_url("uploads/new.bin", None, Duration::from_secs(900))
    .await?;
// browser: fetch(upload, { method: "PUT", body: file })

// bound to an upload of exactly 1 MiB — anything else is rejected
let bounded = bucket
    .presigned_put_url("uploads/new.bin", Some(1024 * 1024), Duration::from_secs(900))
    .await?;
```

`content_length` binds the URL to an upload of exactly that many bytes; `None`
accepts any size. Use it when handing an upload URL to an untrusted end user —
otherwise a leaked URL can store an object of any size against your project's
storage quota. The bound is exact rather than a maximum because SigV4 cannot
express a size range, so callers that do not know the size up front must read
it first (a browser's `File.size`) and mint the URL per upload.

The URL is signed by the worker's object-storage hijack — application code
never holds credentials. The SigV4 signature, expiry, R2 endpoint, account id
and bucket name appear in the URL; the secret access key never does. On fn0
Cloud, `expires` is capped at 5 minutes (`PRESIGN_MAX_EXPIRES_SECS = 300`);
longer requested durations are clamped, not rejected. Self-hosted deployments
have no cap.

Presigned URL minting counts against per-project quotas (100k/month,
1k/hour on the one-dollar plan). Exceeding the quota blocks minting with
HTTP 429 until the window resets; already-minted URLs stay valid until they
expire. See [limits.md](../fn0/limits.md) for full quota values.

In `forte dev` the URL points at the dev server's local object route
(`/__fn0_object_storage/…`) and does not expire.

## Public objects

`object_storage::public` stores objects at a stable, world-readable URL served
by the CDN, for assets embedded in HTML that outlives a signature.

```rust
let public = object_storage::public::bucket();

let url = public.put("clips/intro.mp4", "video/mp4", bytes).await?;
// https://fn0-<project_id>-public-object-storage.<your-domain>/clips/intro.mp4

public.url("clips/intro.mp4");   // same string, no request
public.delete("clips/intro.mp4").await?;
```

`content_type` is required — a browser fetches these directly, with no app in
the path to correct a wrong guess.

Writing to a key overwrites it and invalidates the edge copy, so the URL can be
persisted and embedded safely. `put` returns once the object is written and the
invalidation is queued, **not** once the edge is consistent; until that drains
the edge may still serve the previous bytes.

There is no `presigned_get_url` here — the object is already public, so signing
access to it means nothing.

### Reading: `get_from_origin` vs `get_from_cdn`

Reading is split by which copy answers, because the two cannot be made to
agree. Both return the same `Option<Object>` as `private::get`.

```rust
// The bucket. Always the bytes this app last wrote.
let object = public.get_from_origin("clips/intro.mp4").await?;

// The edge. Cheaper, and can be a version behind.
let object = public.get_from_cdn("clips/intro.mp4").await?;
```

| | reads | sees its own `put` | costs a bucket read |
|---|---|---|---|
| `get_from_origin` | the bucket | always | yes |
| `get_from_cdn` | the CDN edge | only after the invalidation drains | only on an edge miss |

Use `get_from_origin` whenever the app must see its own write, and
`get_from_cdn` for an object read far more often than it is written — an edge
hit never reaches the bucket, so it costs no R2 class B operation.

`get_from_cdn` can answer with a previous version after a `put`, a `delete`, or
a presigned upload. A presigned upload is the worst case: nothing invalidates
the edge on that path at all, so without an explicit `purge` the stale copy can
survive for up to a year.

`head` and `list` always read the bucket, so they can report an object that
`get_from_cdn` is still serving the previous version of.

The bytes travel compressed when the CDN can compress them — the runtime asks
for `zstd` and decodes the stream before it reaches your app, so what you read
is always the stored bytes. Content types the CDN does not compress (images,
video, anything already compressed) simply travel as they are.

In `forte dev` there is no edge: both calls read the local store, so
`get_from_cdn` never returns a previous version there.

### Presigned uploads

`presigned_put_url` hands out an upload URL so durable file bytes do not pass
through your app. This is the recommended path for large uploads and avoids
using compute for storage transfer, while HTTP handlers can still stream bodies
up to the 100 MB transport limit when application processing is required:

```rust
let url = public
    .presigned_put_url("clips/intro.mp4", "video/mp4", Some(size), Duration::from_secs(300))
    .await?;
```

`Cache-Control` and `Content-Type` are part of the signature, so the upload must
send exactly these two headers or R2 rejects it — a browser-cacheable `max-age`
chosen by the uploader would seed copies that no invalidation could ever reach:

```
Cache-Control: public, max-age=0, s-maxage=31536000
Content-Type: <the content_type you passed>
```

`PublicBucket::UPLOAD_CACHE_CONTROL` is that string, so a handler can hand it to
whatever performs the upload rather than hardcoding it.

**A presigned write does not invalidate the edge copy.** The platform never sees
it. Overwriting a key that is already published needs an explicit purge:

```rust
public.purge("clips/intro.mp4").await?;
```

Skipping it leaves the edge serving the previous bytes for up to a year. Writing
to a key that has never been published needs no purge — the edge does not cache
404s, so there is nothing to invalidate.

`forte purge <key>...` and `fn0 purge <key>...` do the same thing from a
terminal.

### Caching

Objects are stored with a platform-fixed header:

```
Cache-Control: public, max-age=0, s-maxage=31536000
```

The edge holds the object; the browser revalidates on every request. Apps
cannot change this. A cache purge reaches the edge but can never reach a
browser, so any browser-held copy would outlive an overwrite with no way to
correct it.

`s-maxage` is long because purge, not expiry, is what keeps the object correct.

### Everything here is public

The bucket is served by a custom domain, so every object under it is readable
by anyone with the URL. Key naming is a convention, not access control. Use
`object_storage::private` for anything else.

## Local development

`forte dev` serves object storage from the local filesystem under
`.forte/data/objects/`, and public objects under `.forte/data/public/` — no
cloud credentials, no external service. The API is identical to production, so
code paths do not change between `forte dev` and deployed apps.

Public URLs in dev point at the dev server (`/__fn0_public_storage/…`) and carry
no project segment, since one dev server serves one project. Nothing is cached,
so there is no purge step to mirror.
