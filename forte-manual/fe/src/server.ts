import { renderToReadableStream } from "react-dom/server";

export async function handler(req: Request): Promise<Response> {
  const props = await req.json();

  const url = new URL(req.url);
  const pathParts = url.pathname.split("/");

  if (pathParts.length === 2 && pathParts[2] === "") {
    return new Response(
      await renderToReadableStream(
        (await import("./pages/index/page")).default(props)
      )
    );
  }

  if (pathParts.length === 3 && pathParts[2] === "product") {
    return new Response(
      await renderToReadableStream(
        (await import("./pages/product/[id]/page")).default(props)
      )
    );
  }

  return new Response("Not Found", { status: 404 });
}
