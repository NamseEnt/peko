import assert from "node:assert/strict";
import { test } from "node:test";

import worker from "./cloudflare_broker_worker.mjs";

const ACCOUNT_ID = "0123456789abcdef0123456789abcdef";
const OWNER_GITHUB_ID = 42;
const CONTROL_URL = "https://control.example.com";
const BROKER_URL = "https://fn0-broker.example.workers.dev";

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function cf(result) {
  return { success: true, errors: [], result };
}

function cfError(message, code = 1000) {
  return { success: false, errors: [{ code, message }], result: null };
}

// Cloudflare's own base URL is `.../client/v4`; routes below are written
// against the short, logical path the worker's source uses (e.g.
// `/user/tokens/verify`), so that prefix is stripped here rather than
// repeated at every call site.
const CLOUDFLARE_API_PATH_PREFIX = "/client/v4";

// Every outbound call the worker makes goes through `fetch(url, init)` with
// `init.headers.Authorization` set to the bearer credential it authenticated
// with, so routes can tell apart calls made with different Cloudflare
// tokens to the very same path — exactly what the rotate/clear ordering
// tests below need.
function makeFetchStub(routes) {
  const calls = [];
  const fetchStub = async (url, init = {}) => {
    const method = (init.method ?? "GET").toUpperCase();
    const authorization = init.headers?.Authorization ?? null;
    const body = init.body ? JSON.parse(init.body) : undefined;
    const parsedUrl = new URL(url);
    const pathname = parsedUrl.pathname.startsWith(CLOUDFLARE_API_PATH_PREFIX)
      ? parsedUrl.pathname.slice(CLOUDFLARE_API_PATH_PREFIX.length)
      : parsedUrl.pathname;
    const request = { hostname: parsedUrl.hostname, pathname, method, authorization, body };
    calls.push(request);
    for (const route of routes) {
      const response = route(request);
      if (response) return response;
    }
    throw new Error(`unmocked fetch: ${method} ${parsedUrl.hostname}${pathname} (auth=${authorization})`);
  };
  return { fetchStub, calls };
}

function route(method, pathname, authorization, respond) {
  return (request) => {
    if (request.method !== method || request.pathname !== pathname) return null;
    if (authorization !== undefined && request.authorization !== authorization) return null;
    return respond(request);
  };
}

function controlAuthorizes(githubId) {
  return route("POST", "/__forte_action/cloudflare_broker_authorize", undefined, () =>
    jsonResponse({ t: "Authorized", githubId }),
  );
}

// A catch-all for tests that only care about the broker's *policy* layer
// (replay, rate limiting) and are indifferent to whatever the operation goes
// on to do against Cloudflare afterward.
function anyOtherCloudflareCallSucceeds() {
  return (request) =>
    request.hostname === "api.cloudflare.com" ? jsonResponse(cf({})) : null;
}

function makeEnv(overrides = {}) {
  return {
    SETUP_TOKEN: { get: async () => "setup-token" },
    CONTROL_URL,
    ACCOUNT_ID,
    STORE_ID: "store-id",
    OWNER_GITHUB_ID: String(OWNER_GITHUB_ID),
    ...overrides,
  };
}

function defaultHeaders(overrides = {}) {
  return {
    "content-type": "application/json",
    authorization: "Bearer control-token",
    "x-forte-request-id": crypto.randomUUID(),
    "x-forte-request-timestamp": String(Math.floor(Date.now() / 1000)),
    ...overrides,
  };
}

function makeRequest(pathname, body, headers = defaultHeaders()) {
  return new Request(`${BROKER_URL}${pathname}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
}

async function withStubbedFetch(routes, run) {
  const { fetchStub, calls } = makeFetchStub(routes);
  const originalFetch = globalThis.fetch;
  const originalSetTimeout = globalThis.setTimeout;
  globalThis.fetch = fetchStub;
  // The worker's backoff sleeps (auth retry, bucket-delete retry) are real
  // wall-clock waits; collapse them so tests exercising a retry path do not
  // actually sit for tens of seconds.
  globalThis.setTimeout = (callback) => originalSetTimeout(callback, 0);
  try {
    return { calls, result: await run() };
  } finally {
    globalThis.fetch = originalFetch;
    globalThis.setTimeout = originalSetTimeout;
  }
}

// Replay: a missing or malformed request id is refused before anything else runs.
test("refuses a request with no request id", async () => {
  const headers = defaultHeaders();
  delete headers["x-forte-request-id"];
  const request = makeRequest("/v1/resolve-zone", { zone_name: "example.com" }, headers);

  const response = await worker.fetch(request, makeEnv());

  assert.equal(response.status, 400);
});

// Replay: a timestamp older than the broker's 5 minute window is refused.
test("refuses a request with a stale timestamp", async () => {
  const headers = defaultHeaders({
    "x-forte-request-timestamp": String(Math.floor(Date.now() / 1000) - 10 * 60),
  });
  const request = makeRequest("/v1/resolve-zone", { zone_name: "example.com" }, headers);

  const response = await worker.fetch(request, makeEnv());

  assert.equal(response.status, 400);
  assert.match(await response.text(), /expired/);
});

// Replay: reusing the same request id a second time is refused, even for an
// otherwise well-formed request.
test("refuses a request id it has already seen", async () => {
  const headers = defaultHeaders();
  const first = makeRequest("/v1/resolve-zone", { zone_name: "example.com" }, headers);
  const second = makeRequest("/v1/resolve-zone", { zone_name: "example.com" }, headers);

  const { calls, result: firstResponse } = await withStubbedFetch(
    [controlAuthorizes(OWNER_GITHUB_ID), anyOtherCloudflareCallSucceeds()],
    () => worker.fetch(first, makeEnv()),
  );
  const controlCalls = calls.filter(
    (call) => call.pathname === "/__forte_action/cloudflare_broker_authorize",
  );
  assert.equal(controlCalls.length, 1, "the first request should have reached control");

  const secondResponse = await worker.fetch(second, makeEnv());

  assert.notEqual(firstResponse.status, 409);
  assert.equal(secondResponse.status, 409);
});

// Rate limit: the same source and operation may not exceed the broker's
// per-minute ceiling, even across distinct, otherwise-valid request ids.
test("rate limits repeated calls to the same operation from the same source", async () => {
  const env = makeEnv();
  const source = { headers: { "cf-connecting-ip": "203.0.113.9" } };
  let lastStatus;
  const routes = [controlAuthorizes(OWNER_GITHUB_ID), anyOtherCloudflareCallSucceeds()];
  await withStubbedFetch(routes, async () => {
    for (let attempt = 0; attempt < 31; attempt += 1) {
      const headers = defaultHeaders({ "cf-connecting-ip": source.headers["cf-connecting-ip"] });
      const request = makeRequest("/v1/resolve-zone", { zone_name: "example.com" }, headers);
      const response = await worker.fetch(request, env);
      lastStatus = response.status;
    }
  });

  assert.equal(lastStatus, 429);
});

// Auth failure: no bearer credential at all is refused before control is asked.
test("refuses a request with no authorization header", async () => {
  const headers = defaultHeaders();
  delete headers.authorization;
  const request = makeRequest("/v1/resolve-zone", { zone_name: "example.com" }, headers);

  const { calls, result: response } = await withStubbedFetch([], () =>
    worker.fetch(request, makeEnv()),
  );

  assert.equal(response.status, 401);
  assert.equal(calls.length, 0, "control must not be asked without a credential");
});

// Auth failure: control refusing the caller (not logged in, wrong project
// owner, ...) refuses the broker request too.
test("refuses a request control does not authorize", async () => {
  const request = makeRequest("/v1/resolve-zone", { zone_name: "example.com" });
  const routes = [
    route("POST", "/__forte_action/cloudflare_broker_authorize", undefined, () =>
      jsonResponse({ t: "NotLoggedIn" }),
    ),
  ];

  const { result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(request, makeEnv()),
  );

  assert.equal(response.status, 403);
});

// Auth failure: control authorizing a *different* account owner than the one
// this broker was bootstrapped for is refused, not silently accepted.
test("refuses a caller who is authorized but is not this broker's owner", async () => {
  const request = makeRequest("/v1/resolve-zone", { zone_name: "example.com" });

  const { result: response } = await withStubbedFetch([controlAuthorizes(OWNER_GITHUB_ID + 1)], () =>
    worker.fetch(request, makeEnv()),
  );

  assert.equal(response.status, 403);
});

function rotateTokenRoutes({ patchSecretResponds, revokeOldTokenResponds }) {
  return [
    controlAuthorizes(OWNER_GITHUB_ID),
    route("GET", "/user/tokens/verify", "Bearer replacement-token", () =>
      jsonResponse(cf({ status: "active", id: "replacement-token-id" })),
    ),
    route("GET", "/user/tokens/permission_groups", "Bearer setup-token", () =>
      jsonResponse(cf([{ id: "secret-store-group", name: "Secrets Store Edit" }])),
    ),
    route("POST", "/user/tokens", "Bearer setup-token", () =>
      jsonResponse(cf({ id: "temporary-store-token-id", value: "temporary-store-token" })),
    ),
    route("DELETE", "/user/tokens/temporary-store-token-id", "Bearer setup-token", () =>
      jsonResponse(cf({})),
    ),
    route(
      "GET",
      `/accounts/${ACCOUNT_ID}/secrets_store/stores/store-id/secrets`,
      "Bearer temporary-store-token",
      () => jsonResponse(cf([{ id: "secret-id", name: "FN0_SETUP_TOKEN" }])),
    ),
    route(
      "PATCH",
      `/accounts/${ACCOUNT_ID}/secrets_store/stores/store-id/secrets/secret-id`,
      "Bearer temporary-store-token",
      () => patchSecretResponds(),
    ),
    route("GET", "/user/tokens/verify", "Bearer setup-token", () =>
      jsonResponse(cf({ status: "active", id: "a".repeat(32) })),
    ),
    route("DELETE", `/user/tokens/${"a".repeat(32)}`, "Bearer setup-token", () =>
      revokeOldTokenResponds(),
    ),
  ];
}

// Partial failure: if writing the new token into the Secrets Store fails,
// the broker must not go on to revoke the still-working old setup token —
// that would brick the broker with no valid token anywhere.
test("does not revoke the old setup token when the new one failed to save", async () => {
  const request = makeRequest("/v1/rotate-token", { new_setup_token: "replacement-token" });
  const routes = rotateTokenRoutes({
    patchSecretResponds: () => jsonResponse(cfError("internal error writing secret"), 500),
    revokeOldTokenResponds: () => {
      throw new Error("must not be called");
    },
  });

  const { calls, result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(request, makeEnv()),
  );

  assert.equal(response.status, 500);
  const oldTokenTouched = calls.some(
    (call) => call.pathname === "/user/tokens/verify" && call.authorization === "Bearer setup-token",
  );
  assert.equal(oldTokenTouched, false, "the old setup token must be left alone");
});

// External token revocation: the new token is already live in the Secrets
// Store by the time the broker tries to clean up the old one, so a failure
// revoking that now-redundant old token (e.g. someone had already revoked it
// by hand) must not turn an otherwise-successful rotation into a reported
// failure.
test("still reports success if the now-redundant old token cannot be revoked", async () => {
  const request = makeRequest("/v1/rotate-token", { new_setup_token: "replacement-token" });
  const routes = rotateTokenRoutes({
    patchSecretResponds: () => jsonResponse(cf({})),
    revokeOldTokenResponds: () => jsonResponse(cfError("invalid or expired token"), 401),
  });

  const { result: response } = await withStubbedFetch(routes, () => worker.fetch(request, makeEnv()));

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true });
});

// The broker's own error responses must say which Cloudflare (or control)
// call actually failed, not a one-size-fits-all message that leaves the CLI
// unable to tell the user anything useful.
test("surfaces the underlying Cloudflare error instead of a generic message", async () => {
  const request = makeRequest("/v1/rotate-token", { new_setup_token: "replacement-token" });
  const routes = rotateTokenRoutes({
    patchSecretResponds: () => jsonResponse(cfError("internal error writing secret"), 500),
    revokeOldTokenResponds: () => {
      throw new Error("must not be called");
    },
  });

  const { result: response } = await withStubbedFetch(routes, () => worker.fetch(request, makeEnv()));

  assert.equal(response.status, 500);
  const { error } = await response.json();
  assert.match(error, /internal error writing secret/);
  assert.doesNotMatch(error, /^broker request failed$/);
});

// Defense in depth: refuses to revoke a project credential whose stored name
// does not match the id the caller claims it is, instead of trusting the id.
test("refuses to revoke a project credential whose name does not match", async () => {
  const request = makeRequest("/v1/revoke-project-credentials", {
    project_id: "abcd1234",
    worker: "11111111111111111111111111111111".slice(0, 32),
    frontend_asset: "22222222222222222222222222222222".slice(0, 32),
    purge: "33333333333333333333333333333333".slice(0, 32),
  });
  const routes = [
    controlAuthorizes(OWNER_GITHUB_ID),
    route(
      "GET",
      `/user/tokens/${"11111111111111111111111111111111".slice(0, 32)}`,
      "Bearer setup-token",
      () => jsonResponse(cf({ name: "fn0 worker (someone-elses-project)" })),
    ),
  ];

  const { calls, result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(request, makeEnv()),
  );

  assert.equal(response.status, 500);
  assert.equal(
    calls.some((call) => call.method === "DELETE"),
    false,
    "a name mismatch must stop every revoke, not just the mismatched one",
  );
});

// Cloudflare's token API rejects the milliseconds `Date#toISOString` always
// produces, so every minted token's `expires_on` must go out without them —
// confirmed against a real account, where the millisecond form fails with
// "expires_on must be a valid date/time in the format ...".
test("mints tokens with an expires_on that has no milliseconds", async () => {
  const request = makeRequest("/v1/rotate-token", { new_setup_token: "replacement-token" });
  const routes = rotateTokenRoutes({
    patchSecretResponds: () => jsonResponse(cf({})),
    revokeOldTokenResponds: () => jsonResponse(cf({})),
  });

  const { calls } = await withStubbedFetch(routes, () => worker.fetch(request, makeEnv()));

  const mintCall = calls.find((call) => call.method === "POST" && call.pathname === "/user/tokens");
  assert.ok(mintCall, "expected a token to be minted");
  assert.match(mintCall.body.expires_on, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
});

function destroyBrokerRoutes({
  secretsListResponds,
  deleteSecretResponds,
  deleteStoreResponds,
  deleteScriptResponds,
  revokeOldTokenResponds,
}) {
  return [
    controlAuthorizes(OWNER_GITHUB_ID),
    route("GET", "/user/tokens/permission_groups", "Bearer setup-token", () =>
      jsonResponse(cf([{ id: "secret-store-group", name: "Secrets Store Write" }])),
    ),
    route("POST", "/user/tokens", "Bearer setup-token", () =>
      jsonResponse(cf({ id: "temp-token-id", value: "temp-token-value" })),
    ),
    route("DELETE", "/user/tokens/temp-token-id", "Bearer setup-token", () => jsonResponse(cf({}))),
    route(
      "GET",
      `/accounts/${ACCOUNT_ID}/secrets_store/stores/store-id/secrets`,
      "Bearer temp-token-value",
      () => secretsListResponds(),
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/secrets_store/stores/store-id/secrets/existing-secret-id`,
      "Bearer temp-token-value",
      () => deleteSecretResponds(),
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/secrets_store/stores/store-id`,
      "Bearer temp-token-value",
      () => deleteStoreResponds(),
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/workers/scripts/fn0-broker`,
      "Bearer temp-token-value",
      () => deleteScriptResponds(),
    ),
    route("GET", "/user/tokens/verify", "Bearer setup-token", () =>
      jsonResponse(cf({ status: "active", id: "a".repeat(32) })),
    ),
    route("DELETE", `/user/tokens/${"a".repeat(32)}`, "Bearer setup-token", () =>
      revokeOldTokenResponds(),
    ),
  ];
}

// The happy path: the setup secret, the Secrets Store that held it, the
// Worker script itself, and finally the setup token, all removed.
test("destroy-broker removes the secret, the store, the script, and the setup token", async () => {
  const request = makeRequest("/v1/destroy-broker", {});
  const routes = destroyBrokerRoutes({
    secretsListResponds: () =>
      jsonResponse(cf([{ id: "existing-secret-id", name: "FN0_SETUP_TOKEN" }])),
    deleteSecretResponds: () => jsonResponse(cf({})),
    deleteStoreResponds: () => jsonResponse(cf({})),
    deleteScriptResponds: () => jsonResponse(cf({})),
    revokeOldTokenResponds: () => jsonResponse(cf({})),
  });

  const { calls, result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(request, makeEnv()),
  );

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true });
  assert.ok(
    calls.some((call) => call.method === "DELETE" && call.pathname.endsWith("/existing-secret-id")),
  );
  assert.ok(calls.some((call) => call.method === "DELETE" && call.pathname.endsWith("/store-id")));
  assert.ok(
    calls.some((call) => call.method === "DELETE" && call.pathname.endsWith("/workers/scripts/fn0-broker")),
  );
});

// Every target of destroy-broker may already be gone — a retry after a
// partial failure, or a second run against an already-destroyed broker —
// and that must not be reported as a failure.
test("destroy-broker is a no-op, not a failure, when everything is already gone", async () => {
  const request = makeRequest("/v1/destroy-broker", {});
  const routes = destroyBrokerRoutes({
    secretsListResponds: () =>
      jsonResponse(cfError("Secrets Store not found", 1000), 404),
    deleteSecretResponds: () => {
      throw new Error("must not be called: the secret list already 404'd");
    },
    deleteStoreResponds: () => jsonResponse(cfError("Secrets Store not found", 1000), 404),
    deleteScriptResponds: () => jsonResponse(cfError("script not found", 1000), 404),
    revokeOldTokenResponds: () => jsonResponse(cf({})),
  });

  const { result: response } = await withStubbedFetch(routes, () => worker.fetch(request, makeEnv()));

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true });
});

const TEARDOWN_ZONE_ID = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4";
const TEARDOWN_PROJECT_ID = "abcd1234";
const TEARDOWN_APP_HOSTNAME = "my-app.example.com";
const PUBLIC_BUCKET_HOSTNAME = `fn0-${TEARDOWN_PROJECT_ID}-public-object-storage.example.com`;
const ASSET_BUCKET_HOSTNAME = `fn0-${TEARDOWN_PROJECT_ID}-frontend-asset.example.com`;

function teardownRequest(overrides = {}) {
  return makeRequest("/v1/teardown-project", {
    project_id: TEARDOWN_PROJECT_ID,
    zone_id: TEARDOWN_ZONE_ID,
    zone_name: "example.com",
    app_hostname: TEARDOWN_APP_HOSTNAME,
    origin_hostname: "oci-ap-osaka-1-nlb.fn0.dev",
    delete_buckets: false,
    ...overrides,
  });
}

function teardownProjectRoutes({
  dnsRecordsResponds = () =>
    jsonResponse(cf([{ id: "cname-id", type: "CNAME", content: "oci-ap-osaka-1-nlb.fn0.dev", proxied: true }])),
  certificatesResponds = () =>
    jsonResponse(cf([{ id: "cert-id", hostnames: [TEARDOWN_APP_HOSTNAME] }])),
  userTokensResponds = () =>
    jsonResponse(
      cf([
        { id: "w".repeat(32), name: `fn0 worker (${TEARDOWN_PROJECT_ID})` },
        { id: "f".repeat(32), name: `fn0 frontend assets (${TEARDOWN_PROJECT_ID})` },
        { id: "p".repeat(32), name: `fn0 cache purge (${TEARDOWN_PROJECT_ID})` },
        { id: "o".repeat(32), name: "fn0 cache purge (some-other-project)" },
      ]),
    ),
  deleteBucketResponds = () => jsonResponse(cf({})),
} = {}) {
  return [
    controlAuthorizes(OWNER_GITHUB_ID),
    route("POST", "/user/tokens", "Bearer setup-token", () =>
      jsonResponse(cf({ id: "prov-id", value: "prov-token" })),
    ),
    route("DELETE", "/user/tokens/prov-id", "Bearer setup-token", () => jsonResponse(cf({}))),
    route("GET", `/zones/${TEARDOWN_ZONE_ID}/dns_records`, "Bearer prov-token", dnsRecordsResponds),
    route("DELETE", `/zones/${TEARDOWN_ZONE_ID}/dns_records/cname-id`, "Bearer prov-token", () =>
      jsonResponse(cf({})),
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/r2/buckets/fn0-${TEARDOWN_PROJECT_ID}-public-object-storage/domains/custom/${PUBLIC_BUCKET_HOSTNAME}`,
      "Bearer prov-token",
      () => jsonResponse(cf({})),
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/r2/buckets/fn0-${TEARDOWN_PROJECT_ID}-frontend-asset/domains/custom/${ASSET_BUCKET_HOSTNAME}`,
      "Bearer prov-token",
      () => jsonResponse(cf({})),
    ),
    route("GET", "/certificates", "Bearer prov-token", certificatesResponds),
    route("DELETE", "/certificates/cert-id", "Bearer prov-token", () => jsonResponse(cf({}))),
    route("GET", "/user/tokens", "Bearer setup-token", userTokensResponds),
    route("DELETE", `/user/tokens/${"w".repeat(32)}`, "Bearer setup-token", () => jsonResponse(cf({}))),
    route("DELETE", `/user/tokens/${"f".repeat(32)}`, "Bearer setup-token", () => jsonResponse(cf({}))),
    route("DELETE", `/user/tokens/${"p".repeat(32)}`, "Bearer setup-token", () => jsonResponse(cf({}))),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/r2/buckets/fn0-${TEARDOWN_PROJECT_ID}-private-object-storage`,
      "Bearer prov-token",
      deleteBucketResponds,
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/r2/buckets/fn0-${TEARDOWN_PROJECT_ID}-public-object-storage`,
      "Bearer prov-token",
      deleteBucketResponds,
    ),
    route(
      "DELETE",
      `/accounts/${ACCOUNT_ID}/r2/buckets/fn0-${TEARDOWN_PROJECT_ID}-frontend-asset`,
      "Bearer prov-token",
      deleteBucketResponds,
    ),
  ];
}

// Happy path: the app DNS record, both bucket custom domains, the origin
// certificate, and exactly the three project tokens (never another project's)
// are removed; buckets are left standing.
test("teardown-project removes the DNS record, custom domains, certificate, and the three project tokens", async () => {
  const { calls, result: response } = await withStubbedFetch(teardownProjectRoutes(), () =>
    worker.fetch(teardownRequest(), makeEnv()),
  );

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true, notes: [] });
  const deletes = calls.filter((call) => call.method === "DELETE").map((call) => call.pathname);
  assert.ok(deletes.includes(`/zones/${TEARDOWN_ZONE_ID}/dns_records/cname-id`));
  assert.ok(deletes.includes("/certificates/cert-id"));
  assert.ok(deletes.includes(`/user/tokens/${"w".repeat(32)}`));
  assert.ok(deletes.includes(`/user/tokens/${"p".repeat(32)}`));
  assert.equal(
    deletes.includes(`/user/tokens/${"o".repeat(32)}`),
    false,
    "another project's token must not be touched",
  );
  assert.equal(
    deletes.some((path) => path === `/accounts/${ACCOUNT_ID}/r2/buckets/fn0-${TEARDOWN_PROJECT_ID}-private-object-storage`),
    false,
    "buckets must be left standing without --delete-buckets",
  );
});

// The owner edited the app record (added an A record); it is left in place and
// reported rather than deleted.
test("teardown-project leaves an app DNS record the owner has edited and reports it", async () => {
  const routes = teardownProjectRoutes({
    dnsRecordsResponds: () =>
      jsonResponse(cf([{ id: "a-id", type: "A", proxied: true }, { id: "cname-id", type: "CNAME", proxied: true }])),
  });

  const { calls, result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(teardownRequest(), makeEnv()),
  );

  assert.equal(response.status, 200);
  const { notes } = await response.json();
  assert.equal(notes.length, 1);
  assert.match(notes[0], /my-app\.example\.com/);
  assert.equal(
    calls.some((call) => call.method === "DELETE" && call.pathname.startsWith(`/zones/${TEARDOWN_ZONE_ID}/dns_records`)),
    false,
  );
});

test("teardown-project preserves a proxied CNAME with a different origin", async () => {
  const routes = teardownProjectRoutes({
    dnsRecordsResponds: () =>
      jsonResponse(cf([{ id: "cname-id", type: "CNAME", content: "user-origin.example.net", proxied: true }])),
  });

  const { calls, result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(teardownRequest(), makeEnv()),
  );

  assert.equal(response.status, 200);
  const { notes } = await response.json();
  assert.equal(notes.length, 1);
  assert.match(notes[0], /my-app\.example\.com/);
  assert.equal(
    calls.some(
      (call) => call.method === "DELETE" && call.pathname === `/zones/${TEARDOWN_ZONE_ID}/dns_records/cname-id`,
    ),
    false,
  );
});

test("teardown-project fails when a project token cannot be revoked", async () => {
  const routes = teardownProjectRoutes({
    userTokensResponds: () =>
      jsonResponse(
        cf([{ id: "w".repeat(32), name: `fn0 worker (${TEARDOWN_PROJECT_ID})` }]),
      ),
  });
  routes.unshift(
    route("DELETE", `/user/tokens/${"w".repeat(32)}`, "Bearer setup-token", () =>
      jsonResponse(cfError("permission denied", 9100), 403),
    ),
  );

  const { result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(teardownRequest(), makeEnv()),
  );

  assert.equal(response.status, 500);
  assert.match(await response.text(), /token cleanup failed/);
});

// A missing certificate list (Origin CA never issued one, or already revoked)
// is tolerated, not fatal.
test("teardown-project tolerates a certificate lookup that 404s", async () => {
  const routes = teardownProjectRoutes({
    certificatesResponds: () => jsonResponse(cfError("not found", 1000), 404),
  });

  const { result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(teardownRequest(), makeEnv()),
  );

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true, notes: [] });
});

// --delete-buckets: the three buckets are deleted once empty.
test("teardown-project deletes the buckets with delete_buckets", async () => {
  const { calls, result: response } = await withStubbedFetch(teardownProjectRoutes(), () =>
    worker.fetch(teardownRequest({ delete_buckets: true }), makeEnv()),
  );

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true, notes: [] });
  const bucketDeletes = calls.filter(
    (call) => call.method === "DELETE" && /\/r2\/buckets\/fn0-abcd1234-[a-z-]+$/.test(call.pathname),
  );
  assert.equal(bucketDeletes.length, 3);
});

// --delete-buckets: a bucket teardown has not finished emptying is reported,
// and the rest of the teardown still completes.
test("teardown-project reports a bucket that is still not empty", async () => {
  const routes = teardownProjectRoutes({
    deleteBucketResponds: () => jsonResponse(cfError("The bucket you tried to delete is not empty", 10000), 409),
  });

  const { calls, result: response } = await withStubbedFetch(routes, () =>
    worker.fetch(teardownRequest({ delete_buckets: true }), makeEnv()),
  );

  assert.equal(response.status, 200);
  const { ok, notes } = await response.json();
  assert.equal(ok, true);
  assert.equal(notes.length, 3);
  assert.ok(notes.every((note) => /not empty/.test(note)));
  assert.ok(calls.some((call) => call.method === "DELETE" && call.pathname === "/certificates/cert-id"));
});
