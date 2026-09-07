const API_BASE = "https://api.cloudflare.com/client/v4";
const BROKER_SCRIPT_NAME = "fn0-broker";
const PROVISIONING_TOKEN_MINUTES = 10;
const CERTIFICATE_VALIDITY_DAYS = 5475;
const CERTIFICATE_REQUEST_TYPE = "origin-ecc";
const RULE_DESCRIPTION = "fn0 frontend assets and public objects";
const REQUEST_WINDOW_MS = 60_000;
const REQUEST_LIMIT = 30;
const REQUEST_MAX_AGE_MS = 5 * 60_000;
const REQUEST_ID_TTL_MS = 10 * 60_000;
const BUCKET_DELETE_ATTEMPTS = 3;
const BUCKET_DELETE_DELAY_MS = 4000;
const requestCounters = new Map();
const requestIds = new Map();

const permissionIds = {
  r2StorageWrite: "bf7481a1826f439697cb59a20b22293e",
  r2BucketItemRead: "6a018a9f2fc74eb6b293b0c548f38b39",
  r2BucketItemWrite: "2efd5506f9c8494dacb1fa10a3e7d5b6",
  zoneRead: "c8fed203ed3043cba015a93ad1616f1f",
  cacheSettingsWrite: "9ff81cbbe65c400b97d92c3c1033cab6",
  zoneSettingsWrite: "3030687196b94b638145a3953da2b699",
  sslAndCertificatesWrite: "c03055bc037c4ea9afb9a9f104b7b721",
  dnsWrite: "4755a26eedb94da69e1066d98aa820be",
  cachePurge: "e17beae8b8cb423a99b1730f21238bed",
  workersScriptsWrite: "e086da7e2179491d91ee5f35b3ca210a",
};

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function errorResponse(message, status = 400) {
  return jsonResponse({ error: message }, status);
}

function cleanupRequestState(now) {
  for (const [key, record] of requestCounters) {
    if (record.expiresAt <= now) {
      requestCounters.delete(key);
    }
  }
  for (const [requestId, expiresAt] of requestIds) {
    if (expiresAt <= now) {
      requestIds.delete(requestId);
    }
  }
  if (requestCounters.size > 4096 || requestIds.size > 4096) {
    requestCounters.clear();
    requestIds.clear();
  }
}

function enforceRequestPolicy(request, operation) {
  const requestId = request.headers.get("x-forte-request-id");
  if (!requestId || !/^[0-9a-f-]{36}$/i.test(requestId)) {
    return errorResponse("missing or invalid request id", 400);
  }
  const requestTimestamp = Number(request.headers.get("x-forte-request-timestamp"));
  if (
    !Number.isSafeInteger(requestTimestamp) ||
    Math.abs(Date.now() - requestTimestamp * 1000) > REQUEST_MAX_AGE_MS
  ) {
    return errorResponse("request expired", 400);
  }
  const now = Date.now();
  cleanupRequestState(now);
  if (requestIds.has(requestId)) {
    return errorResponse("duplicate request", 409);
  }
  requestIds.set(requestId, now + REQUEST_ID_TTL_MS);
  const source = request.headers.get("cf-connecting-ip") ?? "unknown";
  const key = `${source}:${operation}`;
  const current = requestCounters.get(key);
  if (!current || current.expiresAt <= now) {
    requestCounters.set(key, { count: 1, expiresAt: now + REQUEST_WINDOW_MS });
    return null;
  }
  if (current.count >= REQUEST_LIMIT) {
    return errorResponse("rate limit exceeded", 429);
  }
  current.count += 1;
  return null;
}

function ensureString(value, fieldName) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${fieldName} is required`);
  }
  return value;
}

function ensureProjectId(projectId) {
  if (!/^[0-9a-z]{8}$/.test(projectId)) {
    throw new Error("invalid project_id");
  }
  return projectId;
}

function ensureZoneId(zoneId) {
  if (!/^[0-9a-f]{32}$/.test(zoneId)) {
    throw new Error("invalid zone_id");
  }
  return zoneId;
}

function ensureHostname(hostname) {
  if (
    typeof hostname !== "string" ||
    hostname.length === 0 ||
    hostname.length > 253 ||
    hostname.includes("/") ||
    hostname.includes("\\") ||
    hostname.includes("\"") ||
    hostname.trim() !== hostname
  ) {
    throw new Error("invalid hostname");
  }
  return hostname.toLowerCase();
}

function ensureAppOrigin(appOrigin, appHostname) {
  let parsed;
  try {
    parsed = new URL(appOrigin);
  } catch {
    throw new Error("invalid app_origin");
  }
  if (
    parsed.protocol !== "https:" ||
    parsed.hostname !== appHostname ||
    parsed.pathname !== "/" ||
    parsed.search ||
    parsed.hash ||
    parsed.username ||
    parsed.password ||
    parsed.port
  ) {
    throw new Error("invalid app_origin");
  }
  return appOrigin;
}

function accountResource(accountId) {
  return `com.cloudflare.api.account.${accountId}`;
}

function zoneResource(zoneId) {
  return `com.cloudflare.api.account.zone.${zoneId}`;
}

function bucketResource(accountId, bucketName) {
  return `com.cloudflare.edge.r2.bucket.${accountId}_default_${bucketName}`;
}

async function cloudflareRequest(setupToken, method, path, body) {
  const requestOptions = {
    method,
    headers: {
      Authorization: `Bearer ${setupToken}`,
      "content-type": "application/json",
    },
  };
  if (body !== undefined) {
    requestOptions.body = JSON.stringify(body);
  }
  const response = await fetch(`${API_BASE}${path}`, requestOptions);
  const payload = await response.json().catch(() => null);
  if (!response.ok || !payload?.success) {
    const errors = Array.isArray(payload?.errors)
      ? payload.errors
          .map((error) => `${error.message ?? "unknown error"} (${error.code ?? "?"})`)
          .join("; ")
      : "no detail";
    throw new Error(`Cloudflare ${method} ${path} failed (${response.status}): ${errors}`);
  }
  return payload.result;
}

// Cloudflare's token API rejects the milliseconds `Date#toISOString` always
// includes ("expires_on must be a valid date/time in the format
// \"2005-12-30T01:02:03Z\""), so they're stripped here rather than at every
// call site.
function expiresOnMinutesFromNow(minutes) {
  return new Date(Date.now() + minutes * 60_000).toISOString().replace(/\.\d{3}Z$/, "Z");
}

async function mintToken(setupToken, name, policies, expiresOn) {
  const body = { name, policies };
  if (expiresOn) {
    body.expires_on = expiresOn;
  }
  const result = await cloudflareRequest(setupToken, "POST", "/user/tokens", body);
  if (!result?.id || !result?.value) {
    throw new Error(`Cloudflare did not return the ${name} token`);
  }
  return { id: result.id, value: result.value };
}

async function revokeToken(setupToken, tokenId) {
  await cloudflareRequest(setupToken, "DELETE", `/user/tokens/${encodeURIComponent(tokenId)}`);
}

async function revokeBestEffort(setupToken, tokenId, operation) {
  try {
    await revokeToken(setupToken, tokenId);
  } catch (error) {
    console.error(`${operation} token cleanup failed`, error instanceof Error ? error.message : error);
  }
}

async function withProvisioning(setupToken, accountId, zoneId, purpose, callback) {
  const expiresOn = expiresOnMinutesFromNow(PROVISIONING_TOKEN_MINUTES);
  const provisioning = await mintToken(
    setupToken,
    `fn0 setup (${purpose})`,
    [
      {
        effect: "allow",
        resources: { [accountResource(accountId)]: "*" },
        permission_groups: [{ id: permissionIds.r2StorageWrite }],
      },
      {
        effect: "allow",
        resources: { [zoneResource(zoneId)]: "*" },
        permission_groups: [
          { id: permissionIds.zoneRead },
          { id: permissionIds.cacheSettingsWrite },
          { id: permissionIds.zoneSettingsWrite },
          { id: permissionIds.sslAndCertificatesWrite },
          { id: permissionIds.dnsWrite },
        ],
      },
    ],
    expiresOn,
  );
  try {
    return await callback(provisioning.value);
  } finally {
    await revokeBestEffort(setupToken, provisioning.id, purpose);
  }
}

async function withAuthorizedRequest(request, env, operation, body, callback) {
  const authorization = request.headers.get("authorization");
  if (!authorization || !authorization.startsWith("Bearer ")) {
    return errorResponse("missing authorization", 401);
  }
  const controlUrl = ensureString(env.CONTROL_URL, "CONTROL_URL").replace(/\/$/, "");
  const authorizationRequestBody = JSON.stringify({
    operation,
    account_id: ensureString(env.ACCOUNT_ID, "ACCOUNT_ID"),
    project_id: body.project_id ?? null,
  });
  // This cross-zone hop to control is measurably flakier than a same-account
  // Cloudflare API call — a same-shape request from outside a Worker does
  // not reproduce the failures seen here. The lookup is a read with no side
  // effects, so a few retries are safe rather than surfacing a transient
  // edge failure as "not authorized".
  let authorizationResponse;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (attempt > 0) {
      await new Promise((resolve) => setTimeout(resolve, 200 * attempt));
    }
    try {
      authorizationResponse = await fetch(`${controlUrl}/__forte_action/cloudflare_broker_authorize`, {
        method: "POST",
        headers: {
          Authorization: authorization,
          "content-type": "application/json",
        },
        body: authorizationRequestBody,
      });
    } catch {
      continue;
    }
    if (authorizationResponse.ok) {
      break;
    }
  }
  if (!authorizationResponse?.ok) {
    const detail = authorizationResponse
      ? `${authorizationResponse.status} ${(await authorizationResponse.text().catch(() => "")).slice(0, 200)}`
      : "no response";
    return errorResponse(`request authorization failed: ${detail}`, 403);
  }
  const authorizationResult = await authorizationResponse.json().catch(() => null);
  if (!authorizationResult || authorizationResult.t !== "Authorized") {
    return errorResponse("request not authorized", 403);
  }
  const ownerGithubId = Number(ensureString(env.OWNER_GITHUB_ID, "OWNER_GITHUB_ID"));
  if (!Number.isSafeInteger(ownerGithubId) || authorizationResult.githubId !== ownerGithubId) {
    return errorResponse("request not authorized", 403);
  }
  return callback(authorizationResult.githubId);
}

async function readBody(request) {
  try {
    const body = await request.json();
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      throw new Error("request body must be an object");
    }
    return body;
  } catch (error) {
    throw new Error(`invalid JSON body: ${error instanceof Error ? error.message : error}`);
  }
}

async function setupToken(env) {
  const token = await env.SETUP_TOKEN.get();
  return ensureString(token, "SETUP_TOKEN");
}

async function resolveZone(setupToken, accountId, body) {
  const zoneName = ensureHostname(ensureString(body.zone_name, "zone_name"));
  const expiresOn = expiresOnMinutesFromNow(PROVISIONING_TOKEN_MINUTES);
  const reader = await mintToken(
    setupToken,
    "fn0 setup (zone discovery)",
    [
      {
        effect: "allow",
        resources: { "com.cloudflare.api.account.*": "*" },
        permission_groups: [{ id: permissionIds.zoneRead }],
      },
    ],
    expiresOn,
  );
  try {
    const zones = await cloudflareRequest(reader.value, "GET", "/zones?per_page=200");
    const matches = (zones ?? []).filter(
      (zone) => zone.name === zoneName && zone.account?.id === accountId,
    );
    if (matches.length !== 1) {
      throw new Error(
        matches.length === 0
          ? `zone ${zoneName} was not found in this Cloudflare account`
          : `zone ${zoneName} exists more than once in this Cloudflare account`,
      );
    }
    return {
      zone_id: matches[0].id,
      zone_name: matches[0].name,
      account_id: matches[0].account.id,
      account_name: matches[0].account.name ?? "",
    };
  } finally {
    await revokeBestEffort(setupToken, reader.id, "zone discovery");
  }
}

function bucketNames(projectId) {
  return {
    privateObjectStorageBucket: `fn0-${projectId}-private-object-storage`,
    publicObjectStorageBucket: `fn0-${projectId}-public-object-storage`,
    frontendAssetBucket: `fn0-${projectId}-frontend-asset`,
  };
}

async function createBucket(token, accountId, bucketName) {
  try {
    await cloudflareRequest(token, "POST", `/accounts/${accountId}/r2/buckets`, {
      name: bucketName,
      locationHint: "apac",
    });
  } catch (error) {
    if (!String(error).toLowerCase().includes("already exists")) {
      throw error;
    }
  }
}

async function putCors(token, accountId, bucketName, appOrigin) {
  await cloudflareRequest(token, "PUT", `/accounts/${accountId}/r2/buckets/${bucketName}/cors`, {
    rules: [
      {
        allowed: { methods: ["GET", "PUT", "HEAD"], origins: [appOrigin], headers: ["*"] },
        exposeHeaders: ["ETag"],
        maxAgeSeconds: 86400,
      },
    ],
  });
}

async function customDomainPresent(token, accountId, bucketName, hostname) {
  try {
    const result = await cloudflareRequest(
      token,
      "GET",
      `/accounts/${accountId}/r2/buckets/${bucketName}/domains/custom`,
    );
    return (result?.domains ?? []).some((domain) => domain.domain === hostname);
  } catch {
    return false;
  }
}

async function attachCustomDomain(token, accountId, bucketName, hostname, zoneId) {
  if (await customDomainPresent(token, accountId, bucketName, hostname)) {
    await cloudflareRequest(
      token,
      "PUT",
      `/accounts/${accountId}/r2/buckets/${bucketName}/domains/custom/${encodeURIComponent(hostname)}`,
      { enabled: true },
    );
    return;
  }
  try {
    await cloudflareRequest(
      token,
      "POST",
      `/accounts/${accountId}/r2/buckets/${bucketName}/domains/custom`,
      { domain: hostname, zoneId, enabled: true, minTLS: "1.2" },
    );
  } catch (error) {
    if (!String(error).toLowerCase().includes("already")) {
      throw error;
    }
  }
}

function extractCacheHosts(rule) {
  const expression = typeof rule?.expression === "string" ? rule.expression : "";
  const exactHosts = expression.match(/http\.host eq "([^"]+)"/);
  if (exactHosts) {
    return new Set([exactHosts[1]]);
  }
  const listedHosts = expression.match(/http\.host in \{([^}]+)\}/);
  if (!listedHosts) {
    return new Set();
  }
  return new Set(
    [...listedHosts[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]).filter(Boolean),
  );
}

function cacheHostExpression(zoneName, appHostnames) {
  const expressions = [
    `http.host wildcard "fn0-*-frontend-asset.${zoneName}"`,
    `http.host wildcard "fn0-*-public-object-storage.${zoneName}"`,
  ];
  if (appHostnames.size > 0) {
    expressions.push(`http.host in {${[...appHostnames].map((host) => `"${host}"`).join(" ")}}`);
  }
  return expressions.join(" or ");
}

async function ensureCacheRule(token, zoneId, zoneName, appHostname, replacedHostname) {
  const path = `/zones/${zoneId}/rulesets/phases/http_request_cache_settings/entrypoint`;
  let rules = [];
  try {
    const result = await cloudflareRequest(token, "GET", path);
    rules = result?.rules ?? [];
  } catch (error) {
    if (!String(error).includes("(404)")) {
      throw error;
    }
  }
  const managedRule = rules.find((rule) => rule.description === RULE_DESCRIPTION);
  const appHostnames = extractCacheHosts(managedRule);
  if (replacedHostname) {
    appHostnames.delete(replacedHostname);
  }
  appHostnames.add(appHostname);
  rules = rules.filter((rule) => rule.description !== RULE_DESCRIPTION);
  rules.unshift({
    action: "set_cache_settings",
    expression: `((${cacheHostExpression(zoneName, appHostnames)}) and http.request.method in {"GET" "HEAD" "PURGE"})`,
    description: RULE_DESCRIPTION,
    action_parameters: { cache: true, browser_ttl: { mode: "respect_origin" } },
  });
  await cloudflareRequest(token, "PUT", path, { rules });
}

async function ensureTieredCache(token, zoneId) {
  await cloudflareRequest(
    token,
    "PATCH",
    `/zones/${zoneId}/cache/tiered_cache_smart_topology_enable`,
    { value: "on" },
  );
}

async function ensureWebsockets(token, zoneId) {
  await cloudflareRequest(token, "PATCH", `/zones/${zoneId}/settings/websockets`, { value: "on" });
}

async function ensureWebsocketsForProject(setupToken, accountId, body) {
  const projectId = ensureProjectId(ensureString(body.project_id, "project_id"));
  const zoneId = ensureZoneId(ensureString(body.zone_id, "zone_id"));
  return withProvisioning(
    setupToken,
    accountId,
    zoneId,
    `WebSockets ${projectId}`,
    async (provisioningToken) => {
      await ensureWebsockets(provisioningToken, zoneId);
      return { ok: true };
    },
  );
}

function bucketScope(accountId, bucketList) {
  const resources = Object.fromEntries(
    bucketList.map((bucketName) => [bucketResource(accountId, bucketName), "*"]),
  );
  return {
    effect: "allow",
    resources,
    permission_groups: [
      { id: permissionIds.r2BucketItemRead },
      { id: permissionIds.r2BucketItemWrite },
    ],
  };
}

async function sha256Hex(value) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function mintProjectCredentials(setupToken, accountId, zoneId, projectId, buckets) {
  const mintedTokenIds = [];
  try {
    const workerToken = await mintToken(
      setupToken,
      `fn0 worker (${projectId})`,
      [
        bucketScope(accountId, [
          buckets.privateObjectStorageBucket,
          buckets.publicObjectStorageBucket,
        ]),
      ],
    );
    mintedTokenIds.push(workerToken.id);
    const assetToken = await mintToken(
      setupToken,
      `fn0 frontend assets (${projectId})`,
      [bucketScope(accountId, [buckets.frontendAssetBucket])],
    );
    mintedTokenIds.push(assetToken.id);
    const purgeToken = await mintToken(
      setupToken,
      `fn0 cache purge (${projectId})`,
      [
        {
          effect: "allow",
          resources: { [zoneResource(zoneId)]: "*" },
          permission_groups: [{ id: permissionIds.cachePurge }],
        },
      ],
    );
    mintedTokenIds.push(purgeToken.id);
    return {
      worker_access_key_id: workerToken.id,
      worker_secret: await sha256Hex(workerToken.value),
      frontend_asset_access_key_id: assetToken.id,
      frontend_asset_secret: await sha256Hex(assetToken.value),
      purge_token: purgeToken.value,
      minted_token_ids: mintedTokenIds,
    };
  } catch (error) {
    for (const tokenId of mintedTokenIds) {
      await revokeBestEffort(setupToken, tokenId, "project credential");
    }
    throw error;
  }
}

async function provisionProject(setupToken, accountId, body) {
  const projectId = ensureProjectId(ensureString(body.project_id, "project_id"));
  const zoneId = ensureZoneId(ensureString(body.zone_id, "zone_id"));
  const appHostname = ensureHostname(ensureString(body.app_hostname, "app_hostname"));
  const appOrigin = ensureAppOrigin(ensureString(body.app_origin, "app_origin"), appHostname);
  const buckets = bucketNames(projectId);
  const result = await withProvisioning(
    setupToken,
    accountId,
    zoneId,
    projectId,
    async (provisioningToken) => {
      const zone = await cloudflareRequest(provisioningToken, "GET", `/zones/${zoneId}`);
      const zoneName = ensureHostname(zone.name);
      if (!appHostname.endsWith(`.${zoneName}`)) {
        throw new Error("app hostname does not belong to the selected zone");
      }
      for (const bucketName of Object.values(buckets)) {
        await createBucket(provisioningToken, accountId, bucketName);
        await putCors(provisioningToken, accountId, bucketName, appOrigin);
      }
      await attachCustomDomain(
        provisioningToken,
        accountId,
        buckets.frontendAssetBucket,
        `${buckets.frontendAssetBucket}.${zoneName}`,
        zoneId,
      );
      await attachCustomDomain(
        provisioningToken,
        accountId,
        buckets.publicObjectStorageBucket,
        `${buckets.publicObjectStorageBucket}.${zoneName}`,
        zoneId,
      );
      await ensureCacheRule(provisioningToken, zoneId, zoneName, appHostname, null);
      await ensureTieredCache(provisioningToken, zoneId);
      await ensureWebsockets(provisioningToken, zoneId);
      const credentials = await mintProjectCredentials(
        setupToken,
        accountId,
        zoneId,
        projectId,
        buckets,
      );
      return {
        zone_name: zoneName,
        frontend_asset_hostname: `${buckets.frontendAssetBucket}.${zoneName}`,
        public_object_storage_hostname: `${buckets.publicObjectStorageBucket}.${zoneName}`,
        private_object_storage_bucket: buckets.privateObjectStorageBucket,
        public_object_storage_bucket: buckets.publicObjectStorageBucket,
        frontend_asset_bucket: buckets.frontendAssetBucket,
        worker_access_key_id: credentials.worker_access_key_id,
        worker_secret: credentials.worker_secret,
        frontend_asset_access_key_id: credentials.frontend_asset_access_key_id,
        frontend_asset_secret: credentials.frontend_asset_secret,
        purge_token: credentials.purge_token,
        minted_token_ids: credentials.minted_token_ids,
      };
    },
  );
  return result;
}

function ensureTokenId(tokenId, fieldName) {
  if (typeof tokenId !== "string" || !/^[0-9a-f]{32}$/.test(tokenId)) {
    throw new Error(`invalid ${fieldName}`);
  }
  return tokenId;
}

async function revokeProjectCredentials(setupToken, body) {
  const projectId = ensureProjectId(ensureString(body.project_id, "project_id"));
  const credentialNames = [
    ["worker", ensureTokenId(body.worker, "worker")],
    ["frontend assets", ensureTokenId(body.frontend_asset, "frontend_asset")],
    ["cache purge", ensureTokenId(body.purge, "purge")],
  ];
  const expectedNames = new Set(credentialNames.map(([purpose]) => `fn0 ${purpose} (${projectId})`));
  for (const [purpose, tokenId] of credentialNames) {
    const token = await cloudflareRequest(
      setupToken,
      "GET",
      `/user/tokens/${encodeURIComponent(tokenId)}`,
    );
    if (!expectedNames.has(token?.name) || token.name !== `fn0 ${purpose} (${projectId})`) {
      throw new Error(`refusing to revoke an unexpected ${purpose} token`);
    }
  }
  for (const [, tokenId] of credentialNames) {
    await revokeToken(setupToken, tokenId);
  }
  return { ok: true };
}

async function secretStorePermissionId(setupToken) {
  const groups = await cloudflareRequest(setupToken, "GET", "/user/tokens/permission_groups");
  const group = (groups ?? []).find((candidate) =>
    ["Secrets Store Edit", "Account Secrets Store Edit", "Secrets Store Write"].includes(
      candidate.name,
    ),
  );
  if (!group?.id) {
    throw new Error("Cloudflare did not expose the Secrets Store edit permission");
  }
  return group.id;
}

async function withSecretStoreToken(setupToken, accountId, purpose, callback) {
  const permissionId = await secretStorePermissionId(setupToken);
  const temporary = await mintToken(
    setupToken,
    `fn0 broker (${purpose})`,
    [
      {
        effect: "allow",
        resources: { [accountResource(accountId)]: "*" },
        permission_groups: [{ id: permissionId }],
      },
    ],
    expiresOnMinutesFromNow(PROVISIONING_TOKEN_MINUTES),
  );
  try {
    return await callback(temporary.value);
  } finally {
    await revokeBestEffort(setupToken, temporary.id, purpose);
  }
}

async function withWorkersScriptsToken(setupToken, accountId, purpose, callback) {
  const temporary = await mintToken(
    setupToken,
    `fn0 broker (${purpose})`,
    [
      {
        effect: "allow",
        resources: { [accountResource(accountId)]: "*" },
        permission_groups: [{ id: permissionIds.workersScriptsWrite }],
      },
    ],
    expiresOnMinutesFromNow(PROVISIONING_TOKEN_MINUTES),
  );
  try {
    return await callback(temporary.value);
  } finally {
    await revokeBestEffort(setupToken, temporary.id, purpose);
  }
}

async function setupSecretId(storeToken, env) {
  const storeId = ensureString(env.STORE_ID, "STORE_ID");
  const secrets = await cloudflareRequest(
    storeToken,
    "GET",
    `/accounts/${ensureString(env.ACCOUNT_ID, "ACCOUNT_ID")}/secrets_store/stores/${storeId}/secrets`,
  );
  const secret = (secrets ?? []).find((candidate) => candidate.name === "FN0_SETUP_TOKEN");
  if (!secret?.id) {
    throw new Error("FN0_SETUP_TOKEN secret was not found");
  }
  return { storeId, secretId: secret.id };
}

// By the time this runs, the state change that actually matters — a new
// token saved, or the old secret deleted — has already succeeded. Revoking
// the now-redundant old Cloudflare token is just tidying up: it may already
// be gone (that's often *why* the caller is rotating or clearing it), so a
// failure here must not turn an otherwise-successful rotate/clear into a
// reported failure.
async function revokeOldSetupTokenBestEffort(setupToken, purpose) {
  try {
    const current = await cloudflareRequest(setupToken, "GET", "/user/tokens/verify");
    await revokeToken(setupToken, ensureTokenId(current.id, "current setup token"));
  } catch (error) {
    console.error(
      `${purpose}: could not revoke the old setup token`,
      error instanceof Error ? error.message : error,
    );
  }
}

async function rotateSetupToken(setupToken, accountId, env, body) {
  const replacement = ensureString(body.new_setup_token, "new_setup_token");
  if (replacement === setupToken) {
    throw new Error("new setup token must be different");
  }
  const verified = await cloudflareRequest(replacement, "GET", "/user/tokens/verify");
  if (verified?.status !== "active") {
    throw new Error("new setup token is not active");
  }
  await withSecretStoreToken(setupToken, accountId, "token rotation", async (storeToken) => {
    const secret = await setupSecretId(storeToken, env);
    await cloudflareRequest(
      storeToken,
      "PATCH",
      `/accounts/${accountId}/secrets_store/stores/${secret.storeId}/secrets/${secret.secretId}`,
      {
        value: replacement,
        scopes: ["workers"],
        comment: "Forte Cloudflare broker setup token",
      },
    );
  });
  await revokeOldSetupTokenBestEffort(setupToken, "token rotation");
  return { ok: true };
}

async function clearSetupToken(setupToken, accountId, env) {
  await withSecretStoreToken(setupToken, accountId, "token removal", async (storeToken) => {
    const secret = await setupSecretId(storeToken, env);
    await cloudflareRequest(
      storeToken,
      "DELETE",
      `/accounts/${accountId}/secrets_store/stores/${secret.storeId}/secrets/${secret.secretId}`,
    );
  });
  await revokeOldSetupTokenBestEffort(setupToken, "token removal");
  return { ok: true };
}

function isNotFound(error) {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("(404)") || message.includes("was not found");
}

// Tears the broker down entirely: the setup secret, the Secrets Store that
// held it, the Worker script itself, and finally the setup token. Every step
// tolerates its target already being gone, so a retry after a partial
// failure (or a second run against an already-destroyed broker) is a no-op
// rather than an error.
async function destroyBroker(setupToken, accountId, env) {
  const storeId = ensureString(env.STORE_ID, "STORE_ID");
  await withSecretStoreToken(setupToken, accountId, "broker teardown", async (storeToken) => {
    try {
      const secret = await setupSecretId(storeToken, env);
      await cloudflareRequest(
        storeToken,
        "DELETE",
        `/accounts/${accountId}/secrets_store/stores/${secret.storeId}/secrets/${secret.secretId}`,
      );
    } catch (error) {
      if (!isNotFound(error)) {
        throw error;
      }
    }
    try {
      await cloudflareRequest(
        storeToken,
        "DELETE",
        `/accounts/${accountId}/secrets_store/stores/${storeId}`,
      );
    } catch (error) {
      if (!isNotFound(error)) {
        throw error;
      }
    }
  });
  await withWorkersScriptsToken(setupToken, accountId, "broker teardown", async (scriptsToken) => {
    try {
      await cloudflareRequest(
        scriptsToken,
        "DELETE",
        `/accounts/${accountId}/workers/scripts/${BROKER_SCRIPT_NAME}`,
      );
    } catch (error) {
      if (!isNotFound(error)) {
        throw error;
      }
    }
  });
  await revokeOldSetupTokenBestEffort(setupToken, "broker teardown");
  return { ok: true };
}

async function issueOriginCertificate(setupToken, accountId, body) {
  const zoneId = ensureZoneId(ensureString(body.zone_id, "zone_id"));
  const hostname = ensureHostname(ensureString(body.hostname, "hostname"));
  const csr = ensureString(body.csr, "csr");
  return withProvisioning(
    setupToken,
    accountId,
    zoneId,
    "origin certificate",
    async (provisioningToken) => {
      const zone = await cloudflareRequest(provisioningToken, "GET", `/zones/${zoneId}`);
      const zoneName = ensureHostname(zone.name);
      if (!hostname.endsWith(`.${zoneName}`)) {
        throw new Error("certificate hostname does not belong to the selected zone");
      }
      const certificate = await cloudflareRequest(provisioningToken, "POST", "/certificates", {
        csr,
        hostnames: [hostname],
        request_type: CERTIFICATE_REQUEST_TYPE,
        requested_validity: CERTIFICATE_VALIDITY_DAYS,
      });
      return {
        certificate_pem: certificate.certificate,
        not_after_epoch_seconds: Math.floor(Date.now() / 1000) + CERTIFICATE_VALIDITY_DAYS * 86400,
      };
    },
  );
}

async function dnsRecords(token, zoneId, hostname) {
  return (
    (await cloudflareRequest(
      token,
      "GET",
      `/zones/${zoneId}/dns_records?name=${encodeURIComponent(hostname)}`,
    )) ?? []
  );
}

async function ensureAppDnsRecord(token, zoneId, appHostname, originHostname, replacedHostname) {
  const records = await dnsRecords(token, zoneId, appHostname);
  const resolvingRecords = records.filter((record) => ["A", "AAAA", "CNAME"].includes(record.type));
  if (resolvingRecords.length === 0) {
    await cloudflareRequest(token, "POST", `/zones/${zoneId}/dns_records`, {
      type: "CNAME",
      name: appHostname,
      content: originHostname,
      proxied: true,
    });
  } else if (
    resolvingRecords.length === 1 &&
    resolvingRecords[0].type === "CNAME"
  ) {
    if (resolvingRecords[0].content !== originHostname || !resolvingRecords[0].proxied) {
      await cloudflareRequest(
        token,
        "PATCH",
        `/zones/${zoneId}/dns_records/${resolvingRecords[0].id}`,
        { content: originHostname, proxied: true },
      );
    }
  } else {
    throw new Error(`${appHostname} already has incompatible DNS records`);
  }

  if (!replacedHostname || replacedHostname === appHostname) {
    return;
  }
  const previousRecords = await dnsRecords(token, zoneId, replacedHostname);
  const previousRecord = previousRecords.find(
    (record) => record.type === "CNAME" && record.proxied && record.content === originHostname,
  );
  if (previousRecord) {
    await cloudflareRequest(token, "DELETE", `/zones/${zoneId}/dns_records/${previousRecord.id}`);
  }
}

async function finalizeDomain(setupToken, accountId, body) {
  const projectId = ensureProjectId(ensureString(body.project_id, "project_id"));
  const zoneId = ensureZoneId(ensureString(body.zone_id, "zone_id"));
  const zoneName = ensureHostname(ensureString(body.zone_name, "zone_name"));
  const appHostname = ensureHostname(ensureString(body.app_hostname, "app_hostname"));
  const originHostname = ensureHostname(ensureString(body.origin_hostname, "origin_hostname"));
  const replacedHostname = body.replaced_app_hostname
    ? ensureHostname(body.replaced_app_hostname)
    : null;
  const appOrigin = `https://${appHostname}`;
  const buckets = bucketNames(projectId);
  return withProvisioning(
    setupToken,
    accountId,
    zoneId,
    `domain ${appHostname}`,
    async (provisioningToken) => {
      const liveZone = await cloudflareRequest(provisioningToken, "GET", `/zones/${zoneId}`);
      if (ensureHostname(liveZone.name) !== zoneName) {
        throw new Error("zone_name does not match the selected zone");
      }
      for (const bucketName of Object.values(buckets)) {
        await putCors(provisioningToken, accountId, bucketName, appOrigin);
      }
      await ensureCacheRule(provisioningToken, zoneId, zoneName, appHostname, replacedHostname);
      await ensureTieredCache(provisioningToken, zoneId);
      await ensureAppDnsRecord(
        provisioningToken,
        zoneId,
        appHostname,
        originHostname,
        replacedHostname,
      );
      return { ok: true };
    },
  );
}

async function deleteAppDnsRecord(token, zoneId, appHostname, originHostname) {
  const records = await dnsRecords(token, zoneId, appHostname);
  const resolving = records.filter((record) => ["A", "AAAA", "CNAME"].includes(record.type));
  if (
    resolving.length === 1 &&
    resolving[0].type === "CNAME" &&
    resolving[0].proxied &&
    originHostname &&
    resolving[0].content === originHostname
  ) {
    await cloudflareRequest(token, "DELETE", `/zones/${zoneId}/dns_records/${resolving[0].id}`);
    return null;
  }
  if (resolving.length === 0) {
    return null;
  }
  return `left ${appHostname}: its DNS records are not the single proxied CNAME fn0 wrote`;
}

async function detachBucketCustomDomain(token, accountId, bucketName, hostname) {
  try {
    await cloudflareRequest(
      token,
      "DELETE",
      `/accounts/${accountId}/r2/buckets/${bucketName}/domains/custom/${encodeURIComponent(hostname)}`,
    );
  } catch (error) {
    if (!isNotFound(error)) {
      throw error;
    }
  }
}

async function deleteOriginCertificates(token, zoneId, appHostname) {
  let certificates;
  try {
    certificates = await cloudflareRequest(token, "GET", `/certificates?zone_id=${zoneId}`);
  } catch (error) {
    if (isNotFound(error)) {
      return;
    }
    throw error;
  }
  for (const certificate of certificates ?? []) {
    if (!Array.isArray(certificate.hostnames) || !certificate.hostnames.includes(appHostname)) {
      continue;
    }
    try {
      await cloudflareRequest(token, "DELETE", `/certificates/${certificate.id}`);
    } catch (error) {
      if (!isNotFound(error)) {
        throw error;
      }
    }
  }
}

async function revokeProjectCredentialsByName(setupToken, projectId) {
  const wanted = new Map(
    ["worker", "frontend assets", "cache purge"].map((purpose) => [
      `fn0 ${purpose} (${projectId})`,
      purpose,
    ]),
  );
  const seen = [];
  for (let page = 1; page <= 20; page += 1) {
    const batch =
      (await cloudflareRequest(setupToken, "GET", `/user/tokens?per_page=50&page=${page}`)) ?? [];
    seen.push(...batch);
    if (batch.length < 50) {
      break;
    }
  }
  for (const token of seen) {
    if (wanted.has(token.name)) {
      try {
        await revokeToken(setupToken, token.id);
      } catch (error) {
        if (!isNotFound(error)) {
          throw new Error(
            `${wanted.get(token.name)} token cleanup failed: ${
              error instanceof Error ? error.message : error
            }`,
          );
        }
      }
    }
  }
}

// The bucket can only be deleted once it is empty, and control's teardown
// empties it asynchronously. A couple of short retries (BUCKET_DELETE_*) cover
// the case where that has just started; if it is still not empty, the caller
// reports it and the owner re-runs (idempotent) or deletes the empty bucket
// from the dashboard later.
async function deleteBucketShell(token, accountId, bucketName) {
  for (let attempt = 0; attempt < BUCKET_DELETE_ATTEMPTS; attempt += 1) {
    try {
      await cloudflareRequest(
        token,
        "DELETE",
        `/accounts/${accountId}/r2/buckets/${bucketName}`,
      );
      return null;
    } catch (error) {
      if (isNotFound(error)) {
        return null;
      }
      const message = error instanceof Error ? error.message : String(error);
      if (!message.includes("not empty") && !message.includes("(409)")) {
        throw error;
      }
      if (attempt < BUCKET_DELETE_ATTEMPTS - 1) {
        await new Promise((resolve) => setTimeout(resolve, BUCKET_DELETE_DELAY_MS));
      }
    }
  }
  return `left bucket ${bucketName}: still not empty (teardown is still clearing it) — re-run once it finishes`;
}

async function teardownProject(setupToken, accountId, body) {
  const projectId = ensureProjectId(ensureString(body.project_id, "project_id"));
  const zoneId = ensureZoneId(ensureString(body.zone_id, "zone_id"));
  const zoneName = ensureHostname(ensureString(body.zone_name, "zone_name"));
  const appHostname = ensureHostname(ensureString(body.app_hostname, "app_hostname"));
  const originHostname =
    body.origin_hostname == null
      ? null
      : ensureHostname(ensureString(body.origin_hostname, "origin_hostname"));
  const deleteBuckets = body.delete_buckets === true;
  if (!appHostname.endsWith(`.${zoneName}`)) {
    throw new Error("app hostname does not belong to the selected zone");
  }
  const buckets = bucketNames(projectId);
  const notes = [];

  await withProvisioning(setupToken, accountId, zoneId, `teardown ${projectId}`, async (token) => {
    const dnsNote = await deleteAppDnsRecord(token, zoneId, appHostname, originHostname);
    if (dnsNote) {
      notes.push(dnsNote);
    }
    await detachBucketCustomDomain(
      token,
      accountId,
      buckets.publicObjectStorageBucket,
      `${buckets.publicObjectStorageBucket}.${zoneName}`,
    );
    await detachBucketCustomDomain(
      token,
      accountId,
      buckets.frontendAssetBucket,
      `${buckets.frontendAssetBucket}.${zoneName}`,
    );
    await deleteOriginCertificates(token, zoneId, appHostname);
    if (deleteBuckets) {
      for (const bucketName of Object.values(buckets)) {
        const bucketNote = await deleteBucketShell(token, accountId, bucketName);
        if (bucketNote) {
          notes.push(bucketNote);
        }
      }
    }
  });

  await revokeProjectCredentialsByName(setupToken, projectId);

  return { ok: true, notes };
}

async function handleRequest(request, env) {
  if (request.method !== "POST") {
    return errorResponse("method not allowed", 405);
  }
  const body = await readBody(request);
  const route = new URL(request.url).pathname;
  const operationByRoute = {
    "/v1/resolve-zone": "resolve_zone",
    "/v1/provision-project": "provision_project",
    "/v1/ensure-websockets": "ensure_websockets",
    "/v1/issue-origin-certificate": "issue_origin_certificate",
    "/v1/finalize-domain": "finalize_domain",
    "/v1/revoke-project-credentials": "revoke_project_credentials",
    "/v1/rotate-token": "rotate_token",
    "/v1/clear-token": "clear_token",
    "/v1/destroy-broker": "destroy_broker",
    "/v1/teardown-project": "teardown_project",
  };
  const operation = operationByRoute[route];
  if (!operation) {
    return errorResponse("not found", 404);
  }
  const policyResponse = enforceRequestPolicy(request, operation);
  if (policyResponse) {
    return policyResponse;
  }
  const accountId = ensureString(env.ACCOUNT_ID, "ACCOUNT_ID");
  const setupTokenValue = await setupToken(env);

  if (route === "/v1/resolve-zone") {
    return withAuthorizedRequest(request, env, "resolve_zone", body, async () =>
      jsonResponse(await resolveZone(setupTokenValue, accountId, body)),
    );
  }
  if (route === "/v1/provision-project") {
    return withAuthorizedRequest(request, env, "provision_project", body, async () =>
      jsonResponse(await provisionProject(setupTokenValue, accountId, body)),
    );
  }
  if (route === "/v1/ensure-websockets") {
    return withAuthorizedRequest(request, env, "ensure_websockets", body, async () =>
      jsonResponse(await ensureWebsocketsForProject(setupTokenValue, accountId, body)),
    );
  }
  if (route === "/v1/issue-origin-certificate") {
    return withAuthorizedRequest(request, env, "issue_origin_certificate", body, async () =>
      jsonResponse(await issueOriginCertificate(setupTokenValue, accountId, body)),
    );
  }
  if (route === "/v1/finalize-domain") {
    return withAuthorizedRequest(request, env, "finalize_domain", body, async () =>
      jsonResponse(await finalizeDomain(setupTokenValue, accountId, body)),
    );
  }
  if (route === "/v1/revoke-project-credentials") {
    return withAuthorizedRequest(request, env, "revoke_project_credentials", body, async () =>
      jsonResponse(await revokeProjectCredentials(setupTokenValue, body)),
    );
  }
  if (route === "/v1/rotate-token") {
    return withAuthorizedRequest(request, env, "rotate_token", body, async () =>
      jsonResponse(await rotateSetupToken(setupTokenValue, accountId, env, body)),
    );
  }
  if (route === "/v1/clear-token") {
    return withAuthorizedRequest(request, env, "clear_token", body, async () =>
      jsonResponse(await clearSetupToken(setupTokenValue, accountId, env)),
    );
  }
  if (route === "/v1/destroy-broker") {
    return withAuthorizedRequest(request, env, "destroy_broker", body, async () =>
      jsonResponse(await destroyBroker(setupTokenValue, accountId, env)),
    );
  }
  if (route === "/v1/teardown-project") {
    return withAuthorizedRequest(request, env, "teardown_project", body, async () =>
      jsonResponse(await teardownProject(setupTokenValue, accountId, body)),
    );
  }
}

export default {
  async fetch(request, env) {
    try {
      return await handleRequest(request, env);
    } catch (error) {
      // Every throw site in this module builds its own message from fixed
      // strings, request-shape checks, or Cloudflare's own error envelope —
      // never from the setup token itself — so it is safe to hand straight
      // back to the caller instead of collapsing into an unhelpful
      // "broker request failed" that hides which of those cases occurred.
      const message = error instanceof Error ? error.message : String(error);
      console.error("broker request failed", message);
      return errorResponse(message, 500);
    }
  },
};
