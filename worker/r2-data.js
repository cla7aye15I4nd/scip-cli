const PACK_PATH = /^\/data\/(generated\/[a-z0-9-]+\/[0-9a-f]{40}\/data\.pack)$/;

function response(status, message, extraHeaders = {}) {
  return new Response(message, {
    status,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      ...extraHeaders
    }
  });
}

export default {
  async fetch(request, env) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return response(405, "Method Not Allowed", { allow: "GET, HEAD" });
    }

    const match = new URL(request.url).pathname.match(PACK_PATH);
    if (!match) return response(404, "Not Found");

    const range = request.headers.get("range");
    if (range && (!/^bytes=\d+-\d+$/.test(range) || range.includes(","))) {
      return response(416, "Range Not Satisfiable", { "accept-ranges": "bytes" });
    }
    if (range) {
      const [start, end] = range.slice(6).split("-").map(Number);
      if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start > end) {
        return response(416, "Range Not Satisfiable", { "accept-ranges": "bytes" });
      }
    }

    const object = request.method === "HEAD"
      ? await env.DATA.head(match[1])
      : await env.DATA.get(match[1], range ? { range: request.headers } : undefined);
    if (!object) return response(404, "Not Found");

    const headers = new Headers();
    object.writeHttpMetadata(headers);
    headers.set("accept-ranges", "bytes");
    headers.set("etag", object.httpEtag);

    let status = 200;
    if (object.range) {
      const { offset, length } = object.range;
      headers.set("content-length", String(length));
      headers.set("content-range", `bytes ${offset}-${offset + length - 1}/${object.size}`);
      status = 206;
    } else {
      headers.set("content-length", String(object.size));
    }

    return new Response(request.method === "HEAD" ? null : object.body, { status, headers });
  }
};
