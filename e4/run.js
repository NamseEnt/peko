(async () => {
  try {
    const {
      0: url,
      1: method,
      2: headers,
      3: rid,
    } = Deno.core.ops.op_get_request_parts();

    const body = rid !== null ? Deno.core.readableStreamForRid(rid) : null;

    const request = new Request(url, { method, headers, body });

    if (typeof handle !== "function") {
      throw new Error("User code must define a global 'handle' function.");
    }
    const response = await handle(request);

    const responseBody = response.body;
    const responseRid = responseBody
      ? responseBody[Symbol.for("Deno.core.resourceId")]
      : null;

    await Deno.core.opAsync(
      "op_respond",
      response.status,
      Array.from(response.headers.entries()),
      responseRid
    );
  } catch (e) {
    console.error("Error in run.js:", e.message, e.stack);
    await Deno.core.opAsync(
      "op_respond",
      500,
      [["content-type", "text/plain"]],
      null
    );
  }
})();
