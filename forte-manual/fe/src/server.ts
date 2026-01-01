import { renderToReadableStream } from "react-dom/server";

(globalThis as any).handler = async function handler(
  req: Request
): Promise<Response> {
  console.log("Handler called, URL:", req.url);
  console.log("Parsing JSON from request body...");
  const props = await req.json();
  console.log("JSON parsed successfully:", JSON.stringify(props).substring(0, 100));

  const url = new URL(req.url);
  const pathParts = url.pathname.split("/");
  console.log("Path parts:", pathParts);

  if (pathParts.length === 2 && pathParts[1] === "") {
    console.log("Rendering index page");
    const pageModule = await import("./pages/index/page");
    const element = pageModule.default(props);
    const stream = await renderToReadableStream(element);
    return new Response(stream, {
      headers: { "content-type": "text/html; charset=utf-8" }
    });
  }

  if (pathParts.length === 3 && pathParts[1] === "product") {
    console.log("Rendering product page");
    const pageModule = await import("./pages/product/[id]/page");
    const element = pageModule.default(props);
    const stream = await renderToReadableStream(element);
    return new Response(stream, {
      headers: { "content-type": "text/html; charset=utf-8" }
    });
  }

  console.log("No route matched, returning 404");
  return new Response("Not Found", { status: 404 });
};
