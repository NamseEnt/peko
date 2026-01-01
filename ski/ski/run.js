import { core } from "ext:core/mod.js";
import { readableStreamForRid, resourceForReadableStream } from "ext:deno_web/06_streams.js";

export async function runHandler() {
  try {
    console.log("[ski/run.js] Getting request parts...");
    const {
      0: url,
      1: method,
      2: headers,
      3: rid,
    } = core.ops.op_get_request_parts();

    const body = rid !== null ? readableStreamForRid(rid) : null;

    const request = new Request(url, { method, headers, body });

    if (typeof handler !== "function") {
      throw new Error("User code must define a global 'handler' function.");
    }
    console.log("[ski/run.js] Calling user handler...");
    const response = await handler(request);
    console.log("[ski/run.js] Handler returned, status:", response.status);

    const responseBody = response.body;
    console.log("[ski/run.js] Response has body:", responseBody !== null);

    let responseRid = null;

    if (responseBody) {
      // Check if it's already a Deno resource-backed stream
      const denoRid = responseBody[Symbol.for("Deno.core.resourceId")];

      if (denoRid !== undefined) {
        console.log("[ski/run.js] Using existing Deno resource RID:", denoRid);
        responseRid = denoRid;
      } else {
        // It's a standard Web ReadableStream - convert to resource
        console.log("[ski/run.js] Standard ReadableStream detected, converting to resource...");
        responseRid = resourceForReadableStream(responseBody);
        console.log("[ski/run.js] Created resource RID:", responseRid);
      }
    }

    console.log("[ski/run.js] Response body RID:", responseRid);
    console.log("[ski/run.js] Calling op_respond...");
    await core.ops.op_respond(
      response.status,
      Array.from(response.headers.entries()),
      responseRid
    );
    console.log("[ski/run.js] op_respond completed");
  } catch (e) {
    console.error("[ski/run.js] Error:", e.message, e.stack);
    await core.ops.op_respond(
      500,
      [["content-type", "text/plain"]],
      null
    );
  }
}
